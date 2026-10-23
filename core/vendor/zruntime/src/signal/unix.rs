//! Signals on unix: a handler of the process's own, set with `sigaction`, that marks each signal it
//! is called for as arrived and wakes the listeners through a pipe, whose read end their runtimes
//! watch.
//!
//! A handler interrupts whatever the thread it runs on was doing, which may hold a lock or be in
//! the middle of an allocation, so it does only what is safe at any point of any code: it sets the
//! signal's bit in [`ARRIVED`], an atomic word, and writes a byte to the pipe, which `write(2)` is
//! safe to do, and puts `errno` back as it found it. Everything else is done as a task polls a
//! [`Signals`](super::Signals), and as one is created: the poll that finds the pipe readable, or
//! the creation, empties it, takes every bit out of `ARRIVED` and hands the signals to their
//! listeners.

use std::{
    io,
    mem::{self, MaybeUninit},
    os::fd::OwnedFd,
    panic, ptr,
    sync::{
        OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    task::{Context, Poll, ready},
};

use libc::c_int;
use rustix::io::{read, write};

use super::{Set, Signal, lock, wake};
use crate::{AsyncIo, Mode, Runtime, log::error, poll::pipe::pipe};

/// What a `Signals` waits on for signals to arrive: a descriptor of its own of the pipe's read end,
/// which its runtime watches.
///
/// A runtime watches each descriptor through one registration at a time, so each listener has a
/// descriptor of its own: a duplicate of the read end that [`PIPE`] keeps. A byte in the pipe turns
/// every duplicate readable, and the first listener that is polled then empties the pipe for all
/// of them.
pub(super) struct Watch<M>
where
    M: Mode,
{
    pipe: AsyncIo<OwnedFd, M>,
}

impl<M> Watch<M>
where
    M: Mode,
{
    /// A watch of the pipe on `runtime`; the first one makes the pipe.
    pub(super) fn new(runtime: &Runtime<M>) -> io::Result<Self> {
        // A duplicate shares the open file description, and with it the non-blocking mode that the
        // pipe was made with. `try_clone` makes it close-on-exec, as the pipe's own ends are, so
        // that no child process inherits it.
        let read = Pipe::get()?.read.try_clone()?;

        Ok(Self {
            pipe: AsyncIo::from_nonblocking(runtime, read)?,
        })
    }

    /// Hands out the signals that have arrived, once the pipe says that some have.
    ///
    /// Resolves once it has handed them to their listeners, whichever those are, and is pending
    /// otherwise, with the waker of `cx` to be woken as the pipe turns readable.
    pub(super) fn poll_deliver(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let delivered = ready!(self.pipe.poll_read_with(cx, |pipe| {
            // Locked from before the pipe is emptied until the signals are handed out: a listener
            // that looked at what it has waiting in between would see none of them, and could
            // hand out a signal that one of them was to join, before it arrived to come after it.
            let mut registry = lock();
            empty(pipe)?;

            // Taken once the pipe is empty, as `ARRIVED` describes.
            Ok(registry.deliver(Set(ARRIVED.swap(0, Ordering::SeqCst))))
        }))?;
        // Clear of the lock, which went with the operation.
        if let Some(panic) = wake(delivered.into_iter().flatten()) {
            panic::resume_unwind(panic);
        }

        Poll::Ready(Ok(()))
    }
}

/// Takes the signals that have arrived and that no listener has been handed yet, for the registry,
/// which the caller holds locked, to hand out.
///
/// What arrived before a listener is created goes to the listeners there are then, and not to it:
/// this is called before it is added.
pub(super) fn arrived() -> Set {
    // No handler is set before the pipe is made, so nothing can have arrived before it.
    let Some(pipe) = PIPE.get() else {
        return Set::EMPTY;
    };
    // Emptied first, as `ARRIVED` describes. A pipe that is empty already turns the read away,
    // which is no failure here.
    let _ = empty(&pipe.read);

    Set(ARRIVED.swap(0, Ordering::SeqCst))
}

/// Fills the pipe, as a test of a handler whose write fails needs it to be: nothing else can, as
/// [`ARRIVED`] says.
#[cfg(test)]
pub(crate) fn fill_pipe() -> io::Result<()> {
    let pipe = Pipe::get()?;
    // In whole pages first, and then byte by byte, as a write of up to a page goes in whole or
    // not at all.
    let page = [0; 4096];
    while write(&pipe.write, &page).is_ok() {}
    while write(&pipe.write, &page[..1]).is_ok() {}

    Ok(())
}

/// The actions that zruntime's handler took the place of, to be put back once nothing listens for
/// their signals: one for each signal, at its variant's discriminant, while zruntime's handler is
/// set for it.
pub(super) struct Handlers {
    previous: [Option<libc::sigaction>; Signal::ALL.len()],
}

impl Handlers {
    pub(super) const fn new() -> Self {
        Self {
            previous: [const { None }; Signal::ALL.len()],
        }
    }

    /// Sets zruntime's handler for `signal`, and keeps the action it takes the place of.
    pub(super) fn take(&mut self, signal: Signal) -> io::Result<()> {
        // SAFETY: all zeroes are a valid `sigaction`: its fields are integers, a set of signals
        // and, on some platforms, an optional function pointer, for which they are `None`.
        let mut action: libc::sigaction = unsafe { mem::zeroed() };
        action.sa_sigaction = handler();
        // A call that the signal interrupts on another thread is restarted, where the OS can
        // restart it, rather than failed with `EINTR`.
        action.sa_flags = libc::SA_RESTART;
        // SAFETY: `sigemptyset` writes the set it is given, which is a field of `action`.
        unsafe { libc::sigemptyset(&mut action.sa_mask) };
        let mut previous = MaybeUninit::uninit();
        // SAFETY: `handle` is safe to run in a signal handler, as its documentation says. `action`
        // is a valid action, and `previous` has room for the one that the call writes there.
        if unsafe { libc::sigaction(raw(signal), &action, previous.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call succeeded, and so wrote the action it replaced.
        self.previous[signal as usize] = Some(unsafe { previous.assume_init() });

        Ok(())
    }

    /// Puts back the action that zruntime's handler of `signal` took the place of, unless other
    /// code has set one since, which stays.
    pub(super) fn give_back(&mut self, signal: Signal) {
        let Some(previous) = self.previous[signal as usize].take() else {
            return;
        };
        if let Err(e) = restore(raw(signal), &previous) {
            error!("Failed to put back the handler of {:?}: {}", signal, e);
        }
    }
}

/// Sets `previous` as the action of the signal numbered `raw`, if zruntime's handler is still the
/// one set for it.
///
/// Code that set an action of its own after zruntime's set it may hand the signal on to zruntime's,
/// as the previous action, and so rely on it: it is left in place.
fn restore(raw: c_int, previous: &libc::sigaction) -> io::Result<()> {
    let mut current = MaybeUninit::<libc::sigaction>::uninit();
    // SAFETY: with no action to set, the call only writes the one that is set, which `current` has
    // room for.
    if unsafe { libc::sigaction(raw, ptr::null(), current.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call succeeded, and so wrote the action.
    let current = unsafe { current.assume_init() };
    if current.sa_sigaction != handler() {
        return Ok(());
    }
    // SAFETY: `previous` is the action that the OS gave back as zruntime's took its place, which
    // was set for the signal then, and so is one to set for it again.
    if unsafe { libc::sigaction(raw, previous, ptr::null_mut()) } != 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

/// zruntime's handler, as an action holds it.
fn handler() -> libc::sighandler_t {
    handle as extern "C" fn(c_int) as libc::sighandler_t
}

/// zruntime's handler of every signal it listens for: marks the signal arrived, and wakes the
/// listeners unless a signal before it has already.
///
/// It runs wherever the thread it interrupts was, which may hold any lock or be in the middle of
/// any function, so it only does what is safe there: atomic operations, a read of [`PIPE`], which
/// `OnceLock::get` makes without a lock and which is set before any handler is, and a `write(2)`,
/// which POSIX counts among the functions that are safe to call in a signal handler. It allocates
/// nothing, takes no lock and does not panic.
///
/// Its write fails only where the pipe is full, which [`ARRIVED`] keeps it from being, and loses
/// nothing where it does: the bytes in the pipe wake a listener as its own would. It leaves `errno`
/// as it found it all the same, as the code it interrupts may have had a call fail, and be about to
/// read why, and a write that goes through the C library, as it does on most platforms, sets
/// `errno` where it fails.
pub(crate) extern "C" fn handle(raw: c_int) {
    let Some(signal) = from_raw(raw) else {
        return;
    };
    // A byte for the signals already marked is in the pipe, or on its way.
    if ARRIVED.fetch_or(Set::from(signal).0, Ordering::SeqCst) != 0 {
        return;
    }
    let Some(pipe) = PIPE.get() else {
        return;
    };
    let errno = errno::errno();
    let _ = write(&pipe.write, &[0]);
    errno::set_errno(errno);
}

/// Reads the pipe until it is empty, or fails with `WouldBlock` where it was empty already.
fn empty(pipe: &OwnedFd) -> io::Result<()> {
    let mut bytes = [0; 64];
    let mut filled = read(pipe, &mut bytes)?;
    // A read that filled the buffer may have left more behind it; one that did not, or that the
    // pipe turned away as empty, has emptied it.
    while filled == bytes.len() {
        match read(pipe, &mut bytes) {
            Ok(more) => filled = more,
            Err(_) => break,
        }
    }

    Ok(())
}

/// The number of `signal` on this platform.
fn raw(signal: Signal) -> c_int {
    match signal {
        Signal::Interrupt => libc::SIGINT,
        Signal::Terminate => libc::SIGTERM,
        Signal::Hangup => libc::SIGHUP,
        Signal::Quit => libc::SIGQUIT,
        Signal::User1 => libc::SIGUSR1,
        Signal::User2 => libc::SIGUSR2,
        Signal::Child => libc::SIGCHLD,
        Signal::WindowChange => libc::SIGWINCH,
    }
}

/// The signal numbered `raw` on this platform, where it is one that a `Signals` can listen for.
fn from_raw(raw: c_int) -> Option<Signal> {
    Signal::ALL
        .iter()
        .copied()
        .find(|signal| self::raw(*signal) == raw)
}

/// The signals that have arrived and that no listener has been handed yet, as the bits of a
/// [`Set`].
///
/// The handler sets the bit of its signal, and writes a byte to the pipe where it found no bit set.
/// The bits are only ever taken all at once, and only after the pipe is emptied: by the poll of a
/// listener that finds the pipe readable, and as a listener is created. So a signal whose bit is
/// set after the bits are taken finds none set, and writes a byte of its own, and one whose bit is
/// set before is among those taken.
///
/// A byte is written only where no bit was set, and every bit is taken only once the bytes before
/// it are read, so the pipe holds no more than a few: one for each handler that found no bit set
/// and has not had its byte read yet, which only handlers running at once on several threads can
/// be. Should the pipe fill all the same, the handler's write fails, and loses nothing, as
/// [`handle`] says.
static ARRIVED: AtomicU32 = AtomicU32::new(0);

/// The pipe through which the handler wakes the listeners, made by the first [`Watch`].
///
/// It is never closed: a handler may run on any thread at any moment, and write to it.
static PIPE: OnceLock<Pipe> = OnceLock::new();

/// What [`PIPE`] holds: the end the handler writes to, and the one each [`Watch`] watches a
/// duplicate of.
struct Pipe {
    read: OwnedFd,
    write: OwnedFd,
}

impl Pipe {
    /// The pipe, made here if it does not exist yet.
    fn get() -> io::Result<&'static Self> {
        if let Some(pipe) = PIPE.get() {
            return Ok(pipe);
        }
        let (read, write) = pipe()?;

        // Another thread may have made one in the meantime: the first one set stays, and the other
        // is closed here, before any handler could write to it.
        Ok(PIPE.get_or_init(|| Self { read, write }))
    }
}
