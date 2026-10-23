//! Tests of `zruntime::signal` that send no signal: they call the handler themselves, as the OS
//! would.
//!
//! What handles a signal is set for the process as a whole, so a real signal would reach every
//! test of this binary, whose tests run side by side, and a real Ctrl-C or Ctrl-Break of the
//! console would reach every process attached to it, cargo and the test harness among them. The
//! tests that send unix's real signals are a binary of their own, `tests/signal.rs`.
//!
//! On unix, the tests are of the handler's `errno`, which only a write that fails can change, and
//! of a waker that panics as a signal is handed out. On Windows, they are of the routine that the
//! console calls on a thread of its own, which hands each event to the listeners itself, and of a
//! waker that panics there. They share the listeners of the process and what hands signals to them,
//! unix's pipe among it, which no other test of this binary reaches, and take turns with them
//! ([`take_turn`]): the test of `errno` needs the pipe to stay full until the handler has run, and
//! each of the others needs its signals to reach its own listeners alone.

#[cfg(unix)]
use std::{
    io,
    mem::MaybeUninit,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
};
#[cfg(windows)]
use std::{panic, thread};
use std::{
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

#[cfg(unix)]
use errno::{Errno, errno, set_errno};
#[cfg(unix)]
use libc::c_int;
use ntest::timeout;
#[cfg(windows)]
use windows_sys::{
    Win32::{
        Foundation::{FALSE, TRUE},
        System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT},
    },
    core::BOOL,
};

#[cfg(unix)]
use crate::signal::fill_pipe;
use crate::{
    LocalRuntime,
    signal::{Signal, Signals, handle},
};

/// The handler leaves `errno` as the code it interrupted had it, even where its write to the pipe
/// fails, as it does once the pipe is full. A write that goes through the C library, as it does on
/// most platforms, sets `errno` where it fails, and the code may have been about to read it.
#[cfg(unix)]
#[test]
#[timeout(15000)]
fn the_handler_leaves_errno_as_it_found_it() {
    let _turn = take_turn();
    let runtime = LocalRuntime::new().unwrap();
    // Makes the pipe, and takes no signal over: the test calls the handler itself.
    let made = Signals::new(&runtime, []).unwrap();
    fill_pipe().unwrap();

    set_errno(Errno(libc::E2BIG));
    handle(libc::SIGUSR1);
    assert_eq!(errno(), Errno(libc::E2BIG));

    // Creating a listener empties the pipe again, and hands what arrived to no listener, as none
    // listens for it.
    drop(made);
    drop(Signals::new(&runtime, []).unwrap());
}

/// A listener whose creation panics is dropped as the panic unwinds, and gives back what it took
/// over. The panic is that of a waker that the creation wakes: of a listener that it hands a
/// signal to, which arrived before it.
#[cfg(unix)]
#[test]
#[timeout(15000)]
fn a_listener_whose_creation_panics_gives_back_what_it_took_over() {
    let _turn = take_turn();
    let runtime = LocalRuntime::new().unwrap();
    let before = handler(libc::SIGWINCH);
    let mut first = Signals::new(&runtime, [Signal::User2]).unwrap();
    let panics = Waker::from(Arc::new(PanicOnWake));
    let polled = first.poll_recv(&mut Context::from_waker(&panics));
    assert!(polled.is_pending());
    handle(libc::SIGUSR2);

    let created = catch_unwind(AssertUnwindSafe(|| {
        Signals::new(&runtime, [Signal::WindowChange])
    }));
    assert!(created.is_err());
    assert_eq!(handler(libc::SIGWINCH), before);
}

/// A waker that panics as a signal is handed out keeps no other listener's task from being woken,
/// and its panic then reaches the poll that handed the signal out. Every listener still hands the
/// signal out.
#[cfg(unix)]
#[test]
#[timeout(15000)]
fn a_waker_that_panics_keeps_no_other_listener_from_being_woken() {
    let _turn = take_turn();
    let runtime = LocalRuntime::new().unwrap();
    // The listeners are handed signals in the order of their places in the registry, and each
    // takes the first free one: the first is handed the signal before the second.
    let mut first = Signals::new(&runtime, [Signal::User2]).unwrap();
    let mut second = Signals::new(&runtime, [Signal::User2]).unwrap();
    let mut third = Signals::new(&runtime, []).unwrap();
    let panics = Waker::from(Arc::new(PanicOnWake));
    let polled = first.poll_recv(&mut Context::from_waker(&panics));
    assert!(polled.is_pending());
    let (flag, waker) = Flag::new();
    let polled = second.poll_recv(&mut Context::from_waker(&waker));
    assert!(polled.is_pending());
    handle(libc::SIGUSR2);

    // The poll of the third finds the pipe readable, and hands the signal to the other two.
    let mut cx = Context::from_waker(Waker::noop());
    let panic = catch_unwind(AssertUnwindSafe(|| third.poll_recv(&mut cx)))
        .expect_err("the waker's panic reaches the poll");
    assert_eq!(panic.downcast_ref::<&str>(), Some(&"a waker that panics"));
    assert!(flag.0.load(Ordering::SeqCst));
    for listener in [&mut first, &mut second] {
        let polled = listener.poll_recv(&mut cx).map(Result::unwrap);
        assert_eq!(polled, Poll::Ready(Signal::User2));
    }
}

/// The routine takes the events that something listens for, and hands each to its listener. It
/// passes on the rest, to the routines added before it, the last of which ends the process: the
/// events that nothing listens for, and those that are no signals here, such as the console
/// closing. Once nothing listens, it passes on every event.
#[cfg(windows)]
#[test]
#[timeout(15000)]
fn the_console_routine_takes_only_the_events_listened_for() {
    let _turn = take_turn();
    let runtime = LocalRuntime::new().unwrap();
    let mut signals = Signals::new(&runtime, [Signal::Break]).unwrap();

    assert_eq!(console(CTRL_BREAK_EVENT), TRUE);
    assert_eq!(runtime.block_on(signals.recv()).unwrap(), Signal::Break);
    assert_eq!(console(CTRL_C_EVENT), FALSE);
    assert_eq!(console(CTRL_CLOSE_EVENT), FALSE);

    drop(signals);
    assert_eq!(console(CTRL_BREAK_EVENT), FALSE);
}

/// A waker that panics as the routine hands an event out keeps no other listener's task from being
/// woken, and its panic goes no further than the routine, which returns as it would have: one that
/// unwound out of it would end the process, as no code of the program's could catch it on the
/// console's thread. Nor does a panic of its payload, as it is dropped there. Every listener still
/// hands the event out.
#[cfg(windows)]
#[test]
#[timeout(15000)]
fn a_waker_that_panics_leaves_the_console_routine_to_return() {
    let _turn = take_turn();
    let runtime = LocalRuntime::new().unwrap();
    // The listeners are handed signals in the order of their places in the registry, and each
    // takes the first free one: the first is handed the event before the second.
    let mut first = Signals::new(&runtime, [Signal::Break]).unwrap();
    let mut second = Signals::new(&runtime, [Signal::Break]).unwrap();
    let panics = Waker::from(Arc::new(PanicWithPayloadOnWake));
    let polled = first.poll_recv(&mut Context::from_waker(&panics));
    assert!(polled.is_pending());
    let (flag, waker) = Flag::new();
    let polled = second.poll_recv(&mut Context::from_waker(&waker));
    assert!(polled.is_pending());

    assert_eq!(console(CTRL_BREAK_EVENT), TRUE);
    assert!(flag.0.load(Ordering::SeqCst));
    let mut cx = Context::from_waker(Waker::noop());
    for listener in [&mut first, &mut second] {
        let polled = listener.poll_recv(&mut cx).map(Result::unwrap);
        assert_eq!(polled, Poll::Ready(Signal::Break));
    }
}

/// The turn of a test with the listeners of the process and what hands signals to them, which it
/// holds until the guard is dropped, whether or not a test that failed with it poisoned it.
fn take_turn() -> MutexGuard<'static, ()> {
    static TURN: Mutex<()> = Mutex::new(());

    TURN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The handler set for the signal numbered `raw`: `SIG_DFL`, `SIG_IGN` or a function.
#[cfg(unix)]
fn handler(raw: c_int) -> libc::sighandler_t {
    let mut action = MaybeUninit::<libc::sigaction>::uninit();
    // SAFETY: with no action to set, the call only writes the one that is set, which `action` has
    // room for.
    let read = unsafe { libc::sigaction(raw, ptr::null(), action.as_mut_ptr()) };
    assert_eq!(read, 0, "{}", io::Error::last_os_error());

    // SAFETY: the call succeeded, and so wrote the action.
    unsafe { action.assume_init() }.sa_sigaction
}

/// Calls the routine with `event`, as the console does, on a thread of its own.
#[cfg(windows)]
fn console(event: u32) -> BOOL {
    thread::spawn(move || handle(event)).join().unwrap()
}

/// A waker whose `wake` panics, for a wake-up that goes wrong.
#[cfg(unix)]
struct PanicOnWake;

#[cfg(unix)]
impl Wake for PanicOnWake {
    fn wake(self: Arc<Self>) {
        panic!("a waker that panics");
    }
}

/// A waker that panics on every wake with a payload that panics as it is dropped.
#[cfg(windows)]
struct PanicWithPayloadOnWake;

#[cfg(windows)]
impl Wake for PanicWithPayloadOnWake {
    fn wake(self: Arc<Self>) {
        panic::panic_any(PayloadPanickingOnDrop);
    }
}

/// A panic payload that panics as it is dropped.
#[cfg(windows)]
struct PayloadPanickingOnDrop;

#[cfg(windows)]
impl Drop for PayloadPanickingOnDrop {
    fn drop(&mut self) {
        panic!("a panic payload that panics as it is dropped");
    }
}

/// A waker that sets a flag, to see whether a task has been woken.
struct Flag(AtomicBool);

impl Flag {
    /// A flag and the waker that sets it.
    fn new() -> (Arc<Self>, Waker) {
        let flag = Arc::new(Self(AtomicBool::new(false)));
        let waker = Waker::from(flag.clone());

        (flag, waker)
    }
}

impl Wake for Flag {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}
