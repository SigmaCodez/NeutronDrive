//! Async signal handling: a stream of the signals of the OS that a program listens for, such as
//! Ctrl-C, `SIGTERM` and `SIGHUP`, like `tokio::signal` and the async-signal crate.
//!
//! [`Signals`] listens for the signals of a set, and hands out each one as it arrives, through
//! [`Signals::recv`] or as a [`Stream`]. A program that shuts down gracefully on Ctrl-C or
//! `SIGTERM`, or reloads its configuration on `SIGHUP`, waits for them alongside its work.
//!
//! # Which signals
//!
//! On unix, a [`Signal`] is one of the signals that programs commonly handle: `SIGINT`, `SIGTERM`,
//! `SIGHUP`, `SIGQUIT`, `SIGUSR1`, `SIGUSR2`, `SIGCHLD` and `SIGWINCH`. On Windows, it is one of
//! the two events that a console sends to the processes attached to it: Ctrl-C, which is
//! [`Signal::Interrupt`] there as `SIGINT` is on unix, and Ctrl-Break. WASI has no signals, and
//! this module builds nothing there.
//!
//! # Taking a signal over
//!
//! A signal goes to the process as a whole, and what is done with it is set for the process as a
//! whole, not for a thread, a runtime or a library. So the first `Signals` that listens for a
//! signal takes it over from whatever handled it before. By default that is the OS, which ends the
//! process on Ctrl-C, `SIGTERM` or `SIGHUP`, for example. From then on, the signal reaches every
//! `Signals` of the process that listens for it, whatever runtime each is on, and does nothing
//! else. In particular, a handler that other code had set for the signal does not run.
//!
//! Once the last `Signals` that listens for a signal is dropped, the signal is handled as it was
//! before. So a program that listens for Ctrl-C only while it runs a job can be ended by Ctrl-C
//! again after the job. On unix, the handler from before is put back, unless other code has set
//! one of its own in the meantime, which is left in place. On Windows, the handler of the console's
//! events that this module adds stays, but only takes an event while a `Signals` listens for it.
//! Otherwise, it passes the event on to the handlers added before it, the last of which is the
//! default one, which ends the process.
//!
//! # Receiving signals
//!
//! A signal that arrives again before a `Signals` has handed it out is handed out once: as the OS
//! keeps one signal of each kind pending, a `Signals` keeps one of each kind waiting. A signal that
//! arrived before a `Signals` was created does not reach it.
//!
//! On unix, the handler that this module sets writes to a pipe, which the runtime of each
//! `Signals` watches. So a `Signals` receives signals as an [`AsyncIo`] makes progress: while a
//! thread is running [`Runtime::block_on`] on its runtime, or, on a runtime from
//! `SharedRuntime::current`, while the helper thread runs it. A signal that arrives meanwhile is
//! not lost: it waits in the pipe until then.
//!
//! On Windows, the console calls the handler on a thread of its own, which hands the event to each
//! `Signals` that listens for it there and then.
//!
//! # Example
//!
//! A program that runs until it is asked to stop, by Ctrl-C at the terminal or by a `SIGTERM`
//! from a service manager. Here it asks itself:
//!
//! ```
//! # #[cfg(unix)]
//! # fn main() -> std::io::Result<()> {
//! use std::{future, pin::pin};
//!
//! use futures::future::{Either, select};
//! use rustix::process::{self, getpid, kill_process};
//! use zruntime::{
//!     LocalRuntime,
//!     signal::{Signal, Signals},
//! };
//!
//! let runtime = LocalRuntime::new()?;
//!
//! let stopped_by = runtime.block_on(async {
//!     let mut signals = Signals::new(&runtime, [Signal::Interrupt, Signal::Terminate])?;
//!     // As a service manager stopping the program would.
//!     kill_process(getpid(), process::Signal::TERM)?;
//!
//!     // The program's work, which would serve requests until it is stopped.
//!     let work = pin!(future::pending::<()>());
//!     match select(pin!(signals.recv()), work).await {
//!         Either::Left((signal, _)) => signal,
//!         Either::Right(((), _)) => unreachable!("the work only ends when it is stopped"),
//!     }
//! })?;
//!
//! // Time to shut down gracefully.
//! assert_eq!(stopped_by, Signal::Terminate);
//! # Ok(())
//! # }
//! # #[cfg(not(unix))]
//! # fn main() {}
//! ```
//!
//! [`Stream`]: futures_core::Stream
//! [`AsyncIo`]: crate::AsyncIo
//! [`Runtime::block_on`]: crate::Runtime::block_on

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use std::{
    any::Any,
    fmt,
    future::poll_fn,
    io,
    panic::{self, AssertUnwindSafe},
    pin::Pin,
    sync::{Mutex, MutexGuard, PoisonError},
    task::{Context, Poll, Waker},
};

use futures_core::Stream;

#[cfg(unix)]
use self::unix::{Handlers, Watch, arrived};
#[cfg(all(test, unix))]
pub(crate) use self::unix::{fill_pipe, handle};
#[cfg(all(test, windows))]
pub(crate) use self::windows::handle;
#[cfg(windows)]
use self::windows::{Handlers, Watch, arrived};
use crate::{Local, Mode, Runtime, scheduler::dispose};

/// A stream of the signals of a set, which it hands out as they arrive.
///
/// [`Signals::new`] takes the set, and the runtime that watches for its signals. From then on,
/// the stream hands out each signal of the set that arrives, through [`recv`](Signals::recv) or as
/// a [`Stream`], which never ends. The [module documentation](self) describes how a signal is
/// taken over from the OS, and given back once the last `Signals` that listens for it is dropped.
///
/// Any number of them can listen at once, for the same signals or for others, on one runtime or on
/// several. Each gets every signal of its own set.
///
/// The type carries the flavour of its runtime, as [`AsyncIo`](crate::AsyncIo) does:
///
/// * On a [`LocalRuntime`](crate::LocalRuntime), it is a `Signals<Local>` ([`Local`] is the
///   default), and stays on the thread that created it.
/// * On a [`SharedRuntime`](crate::SharedRuntime), it is a `Signals<Shared>`, which can be sent to
///   and used from any thread.
///
/// # Example
///
/// Reloads the configuration on each `SIGHUP`, and stops on `SIGTERM`:
///
/// ```
/// # #[cfg(unix)]
/// # fn main() -> std::io::Result<()> {
/// use rustix::process::{self, getpid, kill_process};
/// use zruntime::{
///     LocalRuntime,
///     signal::{Signal, Signals},
/// };
///
/// let runtime = LocalRuntime::new()?;
///
/// let reloads = runtime.block_on(async {
///     let mut signals = Signals::new(&runtime, [Signal::Hangup, Signal::Terminate])?;
///     // As an administrator would, to have the configuration reloaded.
///     kill_process(getpid(), process::Signal::HUP)?;
///
///     let mut reloads = 0;
///     loop {
///         match signals.recv().await? {
///             Signal::Hangup => {
///                 reloads += 1;
///                 // And then to have the program stopped.
///                 kill_process(getpid(), process::Signal::TERM)?;
///             }
///             Signal::Terminate => break,
///             _ => unreachable!("only the signals of the set are handed out"),
///         }
///     }
///
///     Ok::<_, std::io::Error>(reloads)
/// })?;
///
/// assert_eq!(reloads, 1);
/// # Ok(())
/// # }
/// # #[cfg(not(unix))]
/// # fn main() {}
/// ```
///
/// [`Stream`]: futures_core::Stream
pub struct Signals<M = Local>
where
    M: Mode,
{
    /// The signals it listens for.
    signals: Set,
    /// Where its listener is in [`REGISTRY`].
    key: usize,
    /// What it waits on for its signals to arrive.
    watch: Watch<M>,
}

impl<M> Signals<M>
where
    M: Mode,
{
    /// Starts listening for `signals`, which `runtime` watches for.
    ///
    /// Each signal that no other `Signals` of the process listens for yet is taken over here, as
    /// the [module documentation](self) describes. A signal named more than once is listened for
    /// once. An empty set is no error: the stream then never hands anything out.
    ///
    /// # Errors
    ///
    /// Fails if the OS refuses to let a signal be handled, or cannot give what the runtime needs to
    /// watch for the signals: on unix, the pipe that they come through, made the first time, and a
    /// descriptor of it for each `Signals`, which the runtime then watches. Nothing is taken over
    /// where it fails.
    pub fn new<I>(runtime: &Runtime<M>, signals: I) -> io::Result<Self>
    where
        I: IntoIterator<Item = Signal>,
    {
        let signals = signals.into_iter().collect();
        // Before the signals are taken over, so that a failure here leaves nothing to give back.
        let watch = Watch::new(runtime)?;
        let (added, woken) = listen(signals);
        let created = added.map(|key| Self {
            signals,
            key,
            watch,
        });
        // Woken only once the listener is in its `Signals`: a wake is somebody else's code, which
        // may panic, and the `Signals` then takes the listener out of the registry again as the
        // panic unwinds, and gives back what it took over.
        if let Some(panic) = wake(woken.into_iter().flatten()) {
            panic::resume_unwind(panic);
        }

        created
    }

    /// Waits for the next signal, and returns it.
    ///
    /// If several signals of the set are waiting, each call returns one of them.
    ///
    /// # Errors
    ///
    /// Fails if the runtime cannot watch for the signals: on unix, if the OS refuses to watch the
    /// pipe that they come through, as a wait of [`AsyncIo::readable`](crate::AsyncIo::readable)
    /// fails. The signals are not lost: the next call can wait for them again.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no signal: the next call returns it.
    pub async fn recv(&mut self) -> io::Result<Signal> {
        poll_fn(|cx| self.poll_recv(cx)).await
    }

    /// Polls for the next signal.
    ///
    /// If none is waiting, the waker of `cx` is woken once one arrives. Only the waker of the last
    /// poll is woken, so only one task at a time can wait on a `Signals`.
    pub fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<Signal>> {
        // What has arrived is handed out to every listener first, and only then does this one take
        // what it has waiting: a signal that arrived again after the first of its kind was handed
        // to it so joins that one, rather than come after it.
        let delivered = loop {
            match self.watch.poll_deliver(cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(e)) => break Err(e),
                Poll::Pending => break Ok(()),
            }
        };
        if let Some(signal) = take(self.key, cx.waker()) {
            return Poll::Ready(Ok(signal));
        }

        match delivered {
            Ok(()) => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl<M> fmt::Debug for Signals<M>
where
    M: Mode,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signals")
            .field("signals", &self.signals)
            .finish_non_exhaustive()
    }
}

impl<M> Stream for Signals<M>
where
    M: Mode,
{
    type Item = io::Result<Signal>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().poll_recv(cx).map(Some)
    }
}

impl<M> Drop for Signals<M>
where
    M: Mode,
{
    fn drop(&mut self) {
        unlisten(self.key);
    }
}

/// A signal that a [`Signals`] can listen for.
///
/// The variants are the platform's: on unix, the signals that programs commonly handle, and on
/// Windows, the events that a console sends. `Interrupt` is on both, so that Ctrl-C is one variant
/// wherever the program runs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Signal {
    /// Ctrl-C: `SIGINT` on unix, which a terminal sends to the processes in its foreground on
    /// Ctrl-C, and the console's Ctrl-C event on Windows.
    Interrupt,
    /// `SIGTERM`: a request to end the process, which `kill` sends unless told otherwise, and
    /// service managers such as systemd send to stop a service. On unix only.
    #[cfg(unix)]
    Terminate,
    /// `SIGHUP`: the terminal of the process has gone. A daemon, which has none, commonly takes it
    /// as a request to reload its configuration. On unix only.
    #[cfg(unix)]
    Hangup,
    /// `SIGQUIT`, which a terminal sends to the processes in its foreground on Ctrl-\\. On unix
    /// only.
    #[cfg(unix)]
    Quit,
    /// `SIGUSR1`, which means whatever the program makes it mean. On unix only.
    #[cfg(unix)]
    User1,
    /// `SIGUSR2`, which means whatever the program makes it mean. On unix only.
    #[cfg(unix)]
    User2,
    /// `SIGCHLD`: a child process has exited, or has been stopped or continued. On unix only.
    ///
    /// The `process` module of this crate does not need it: it waits for its children in other
    /// ways, which a `Signals` that listens for this leaves as they are.
    #[cfg(unix)]
    Child,
    /// `SIGWINCH`: the window of the terminal has changed size. On unix only.
    #[cfg(unix)]
    WindowChange,
    /// The console's Ctrl-Break event. On Windows only.
    #[cfg(windows)]
    Break,
}

impl Signal {
    /// Every variant, each once, in the order they are declared in.
    const ALL: &[Signal] = &[
        Signal::Interrupt,
        #[cfg(unix)]
        Signal::Terminate,
        #[cfg(unix)]
        Signal::Hangup,
        #[cfg(unix)]
        Signal::Quit,
        #[cfg(unix)]
        Signal::User1,
        #[cfg(unix)]
        Signal::User2,
        #[cfg(unix)]
        Signal::Child,
        #[cfg(unix)]
        Signal::WindowChange,
        #[cfg(windows)]
        Signal::Break,
    ];
}

// Each variant is listed at its discriminant, and has a bit in a set: a set keeps a signal's bit at
// its discriminant, and unix keeps the handler from before a signal was taken over there too.
const _: () = {
    assert!(Signal::ALL.len() <= u32::BITS as usize);
    let mut index = 0;
    while index < Signal::ALL.len() {
        assert!(Signal::ALL[index] as usize == index);
        index += 1;
    }
};

/// Adds a listener of `signals` to the registry, and returns where it is, with the wakers of the
/// listeners that it handed signals to, which it does not wake: [`Signals::new`] does.
///
/// What has arrived and no listener has been handed yet is handed out first, to the listeners
/// there are, as a signal that arrived before a listener was created is not for it.
fn listen(signals: Set) -> (io::Result<usize>, Option<Vec<Waker>>) {
    let mut registry = lock();
    let woken = registry.deliver(arrived());

    (registry.add(signals), woken)
}

/// Wakes each of `wakers`, every one of them even where one before it panics, and hands back the
/// panic of the first that panicked, if one did, for the caller to raise again or dispose of.
///
/// Each was taken out of a listener that has been handed a signal, so one left unwoken would leave
/// its task waiting for a signal that has come already. The payload of any panic after the first is
/// disposed of: dropped with a panic of its own destructor caught, as that would otherwise escape
/// the loop, past the wakers still to wake.
fn wake<I>(wakers: I) -> Option<Box<dyn Any + Send>>
where
    I: IntoIterator<Item = Waker>,
{
    let mut first_panic = None;
    for waker in wakers {
        if let Err(panic) = panic::catch_unwind(AssertUnwindSafe(|| waker.wake())) {
            match first_panic {
                None => first_panic = Some(panic),
                Some(_) => dispose(panic),
            }
        }
    }

    first_panic
}

/// Takes the listener at `key` out of the registry, and gives back each signal that it listened
/// for and no other listener does.
fn unlisten(key: usize) {
    let removed = {
        let mut registry = lock();
        let removed = registry.listeners.get_mut(key).and_then(Option::take);
        if let Some(listener) = &removed {
            for signal in listener.signals.difference(registry.listened()).iter() {
                registry.handlers.give_back(signal);
            }
        }

        removed
    };
    // Clear of the lock: dropping a waker can drop a task, whose future may hold a `Signals` that
    // takes the lock as it goes.
    drop(removed);
}

/// Takes the first signal that has arrived for the listener at `key` out of what it has waiting,
/// or, where nothing is waiting, stores `waker` to be woken once something arrives.
fn take(key: usize, waker: &Waker) -> Option<Signal> {
    // Cloned before the lock is taken: a waker's clone is somebody else's code, which is not to run
    // with the lock held. Where a signal is waiting, the clone is dropped after the lock goes.
    let waker = waker.clone();
    let replaced = {
        let mut registry = lock();
        let Some(Some(listener)) = registry.listeners.get_mut(key) else {
            unreachable!("a listener stays in the registry for as long as its `Signals` lives");
        };
        if let Some(signal) = listener.arrived.first() {
            listener.arrived = listener.arrived.difference(signal.into());

            return Some(signal);
        }

        listener.waker.replace(waker)
    };
    // Clear of the lock, as `unlisten` drops the waker it takes out.
    drop(replaced);

    None
}

/// The registry behind its lock, taken whether or not a panic poisoned it.
fn lock() -> MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Every listener of the process, whatever its runtime, and what handles the signals they listen
/// for.
///
/// Signals and their handlers are the process's, so there is one registry, which every runtime
/// and every listener shares. The lock is only ever taken outside a signal handler: on unix, the
/// handler wakes the listeners through a pipe instead, and the signals are handed out as a
/// listener is polled, and on Windows, the console calls the handler on a thread of its own.
static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    listeners: Vec::new(),
    handlers: Handlers::new(),
});

/// What [`REGISTRY`] holds.
struct Registry {
    /// Each listener under its key, which is where it is: a place is `None` once its `Signals` is
    /// dropped, until another listener takes it.
    listeners: Vec<Option<Listener>>,
    /// What handles the signals of the platform, set up while a listener listens for them.
    handlers: Handlers,
}

impl Registry {
    /// Adds a listener of `signals`, takes over each of them that no other listener listens for,
    /// and returns where the listener is.
    ///
    /// Where a signal cannot be taken over, those taken over before it are given back, and nothing
    /// is added.
    fn add(&mut self, signals: Set) -> io::Result<usize> {
        let mut taken = Set::EMPTY;
        for signal in signals.difference(self.listened()).iter() {
            if let Err(e) = self.handlers.take(signal) {
                for signal in taken.iter() {
                    self.handlers.give_back(signal);
                }

                return Err(e);
            }
            taken = taken.union(signal.into());
        }

        let listener = Some(Listener {
            signals,
            arrived: Set::EMPTY,
            waker: None,
        });
        // The first place a listener has left, so that the registry grows no larger than the most
        // listeners there have been at once.
        let key = match self.listeners.iter().position(Option::is_none) {
            Some(key) => {
                self.listeners[key] = listener;

                key
            }
            None => {
                self.listeners.push(listener);

                self.listeners.len() - 1
            }
        };

        Ok(key)
    }

    /// Hands each of `arrived` to every listener of it, and returns the wakers of the listeners
    /// that got any, or `None` where there were none.
    ///
    /// The wakers are to be woken once the lock is let go of, through [`wake`]: a wake is somebody
    /// else's code, which may poll a `Signals` right there, and so take the lock.
    fn deliver(&mut self, arrived: Set) -> Option<Vec<Waker>> {
        let mut woken = None;
        for listener in self.listeners.iter_mut().flatten() {
            let sent = arrived.intersection(listener.signals);
            if sent.is_empty() {
                continue;
            }
            listener.arrived = listener.arrived.union(sent);
            woken
                .get_or_insert_with(Vec::new)
                .extend(listener.waker.take());
        }

        woken
    }

    /// Every signal that some listener listens for.
    fn listened(&self) -> Set {
        self.listeners
            .iter()
            .flatten()
            .fold(Set::EMPTY, |listened, listener| {
                listened.union(listener.signals)
            })
    }
}

/// What the registry keeps of a `Signals`.
struct Listener {
    /// The signals it listens for.
    signals: Set,
    /// The signals that have arrived for it, and that it has not handed out yet.
    arrived: Set,
    /// The waker of the last poll that found nothing waiting, until a signal arrives to wake it.
    waker: Option<Waker>,
}

/// A set of signals, as a bit for each, at its variant's discriminant.
#[derive(Clone, Copy, Eq, PartialEq)]
struct Set(u32);

impl Set {
    const EMPTY: Self = Self(0);

    fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn contains(self, signal: Signal) -> bool {
        !self.intersection(signal.into()).is_empty()
    }

    fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// The first of the set, in the order of [`Signal::ALL`].
    fn first(self) -> Option<Signal> {
        self.iter().next()
    }

    /// The signals of the set, in the order of [`Signal::ALL`].
    fn iter(self) -> impl Iterator<Item = Signal> {
        Signal::ALL
            .iter()
            .copied()
            .filter(move |signal| self.contains(*signal))
    }
}

impl fmt::Debug for Set {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl From<Signal> for Set {
    fn from(signal: Signal) -> Self {
        Self(1 << signal as u32)
    }
}

impl FromIterator<Signal> for Set {
    fn from_iter<I>(signals: I) -> Self
    where
        I: IntoIterator<Item = Signal>,
    {
        signals
            .into_iter()
            .fold(Self::EMPTY, |set, signal| set.union(signal.into()))
    }
}
