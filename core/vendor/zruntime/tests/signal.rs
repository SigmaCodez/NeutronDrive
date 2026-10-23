//! Tests of `zruntime::signal` on unix, which send the process real signals, and so are a binary of
//! their own.
//!
//! What handles a signal is set for the process as a whole, so a test that listens for a signal, or
//! sends one, acts on every thread of the process. The unit tests of the crate are one binary,
//! whose tests run side by side, and some of them spawn children, whose exits send `SIGCHLD`. A
//! file under `tests/` is a binary of its own, which Cargo runs alone. Its tests still run side by
//! side, so each test sends signals that no other test listens for or sends. It sends them only
//! while a `Signals` of its own listens for them: a signal that arrived while none did would get
//! its default handling, which for most signals is to end the process. The test of putting back
//! the handler from before uses `SIGWINCH`, whose default is to ignore it.
//!
//! The signals are raised on the thread that sends them, which runs the handler before `raise`
//! returns: one sent to the process as a whole would go to whichever thread the OS picks, at a
//! moment of its own choosing, and a test could not tell when it had arrived.
//!
//! The first tests are of a signal reaching its listener, on a runtime of each flavour, and of
//! every listener of it getting it, on runtimes of their own. Then come how signals that arrive
//! together are handed out, that a dropped wait loses none, and that a task on the helper thread
//! gets them. Last come the tests of putting back the handlers from before.

#![cfg(all(feature = "signal", unix))]

use std::{io, mem::MaybeUninit, ptr, thread, time::Duration};
#[cfg(feature = "helper")]
use std::{process::Command, sync::mpsc};

use futures::{StreamExt, future::poll_immediate};
use libc::c_int;
use ntest::timeout;
use zruntime::{
    Local, LocalRuntime, Mode, Runtime, Shared, SharedRuntime,
    signal::{Signal, Signals},
};

/// A signal reaches its listener on a local runtime, and again each time it is sent.
#[test]
#[timeout(15000)]
fn a_signal_reaches_its_listener_on_a_local_runtime() {
    reaches_its_listener::<Local>(Signal::User1, libc::SIGUSR1);
}

/// A signal reaches its listener on a shared runtime, and again each time it is sent.
#[test]
#[timeout(15000)]
fn a_signal_reaches_its_listener_on_a_shared_runtime() {
    reaches_its_listener::<Shared>(Signal::User2, libc::SIGUSR2);
}

/// The body of the two tests above, on a runtime of the flavour `M`: `signal`, numbered `raw`,
/// is handed out through `recv` and through the `Stream`, which goes on after each.
fn reaches_its_listener<M>(signal: Signal, raw: c_int)
where
    M: Mode,
{
    let runtime = Runtime::<M>::new().unwrap();
    let mut signals = Signals::new(&runtime, [signal]).unwrap();
    assert_eq!(
        format!("{signals:?}"),
        format!("Signals {{ signals: {{{signal:?}}}, .. }}")
    );

    raise(raw);
    assert_eq!(runtime.block_on(signals.recv()).unwrap(), signal);
    raise(raw);
    let next = runtime.block_on(signals.next());
    assert_eq!(next.expect("the stream never ends").unwrap(), signal);
    // Each was handed out once.
    assert!(nothing_waits(&runtime, &mut signals));
}

/// Every listener of a signal gets it, on whatever runtime it is, on whichever thread drives that:
/// one on a local runtime of the test's thread, and one on a shared runtime of a thread of its own.
/// A listener created after the signal arrived does not, though neither of the others had been
/// handed it yet: what arrived before a listener was created is not for it.
///
/// The creation of the third hands the signal to the first two, which were there as it arrived.
#[test]
#[timeout(15000)]
fn every_listener_of_a_signal_gets_it_and_no_later_one() {
    let local = LocalRuntime::new().unwrap();
    let mut here = Signals::new(&local, [Signal::Hangup]).unwrap();
    let shared = SharedRuntime::new().unwrap();
    let mut there = Signals::new(&shared, [Signal::Hangup]).unwrap();

    raise(libc::SIGHUP);
    let mut later = Signals::new(&local, [Signal::Hangup]).unwrap();
    let elsewhere = thread::spawn(move || shared.block_on(there.recv()).unwrap());
    assert_eq!(local.block_on(here.recv()).unwrap(), Signal::Hangup);
    assert_eq!(elsewhere.join().unwrap(), Signal::Hangup);
    assert!(nothing_waits(&local, &mut later));
}

/// Signals that arrive before a listener is polled are handed out one per call, each once,
/// however many times it arrived, and none of them reaches a listener that does not listen for it.
#[test]
#[timeout(15000)]
fn signals_that_arrive_together_are_handed_out_once_each() {
    let runtime = LocalRuntime::new().unwrap();
    let mut both = Signals::new(&runtime, [Signal::Interrupt, Signal::Terminate]).unwrap();
    let mut one = Signals::new(&runtime, [Signal::Terminate]).unwrap();

    raise(libc::SIGTERM);
    raise(libc::SIGINT);
    raise(libc::SIGTERM);
    // In no order that the API promises.
    let handed_out = [
        runtime.block_on(both.recv()).unwrap(),
        runtime.block_on(both.recv()).unwrap(),
    ];
    assert!(handed_out.contains(&Signal::Interrupt));
    assert!(handed_out.contains(&Signal::Terminate));
    assert!(nothing_waits(&runtime, &mut both));

    assert_eq!(runtime.block_on(one.recv()).unwrap(), Signal::Terminate);
    assert!(nothing_waits(&runtime, &mut one));
}

/// A wait that is dropped before its signal arrives loses nothing: the next one gets the signal.
#[test]
#[timeout(15000)]
fn a_dropped_wait_loses_no_signal() {
    let runtime = LocalRuntime::new().unwrap();
    let mut signals = Signals::new(&runtime, [Signal::Quit]).unwrap();

    // Polled once, which finds nothing, and dropped.
    assert!(runtime.block_on(poll_immediate(signals.recv())).is_none());
    raise(libc::SIGQUIT);
    assert_eq!(runtime.block_on(signals.recv()).unwrap(), Signal::Quit);
}

/// A task on the helper thread of a runtime from `SharedRuntime::current` gets the signal, with no
/// thread running `block_on`: the helper watches the pipe that the signals come through, as it
/// watches any other source of the runtime.
///
/// The signal is `SIGCHLD`, which a child that exits sends.
#[cfg(feature = "helper")]
#[test]
#[timeout(15000)]
fn a_task_on_the_helper_gets_its_signal() {
    let runtime = SharedRuntime::current().unwrap();
    let mut signals = Signals::new(&runtime, [Signal::Child]).unwrap();
    let (report, reported) = mpsc::channel();
    runtime
        .spawn("a listener", async move {
            report.send(signals.recv().await.unwrap()).unwrap();
        })
        .detach();

    let status = Command::new("true").status().unwrap();
    assert!(status.success());
    assert_eq!(reported.recv().unwrap(), Signal::Child);
}

/// The handler from before a signal was taken over is put back once the last of its listeners is
/// dropped, and not before, whatever it was: the default, or a choice to ignore the signal. A
/// handler that other code has set in the meantime is left in place.
#[test]
#[timeout(15000)]
fn the_handler_from_before_is_put_back_once_nothing_listens() {
    let runtime = LocalRuntime::new().unwrap();
    // As the OS starts a process, unless its parent ignored the signal.
    set_handler(libc::SIGWINCH, libc::SIG_DFL);

    let first = Signals::new(&runtime, [Signal::WindowChange]).unwrap();
    let taken_over = handler(libc::SIGWINCH);
    assert_ne!(taken_over, libc::SIG_DFL);
    assert_ne!(taken_over, libc::SIG_IGN);
    let mut second = Signals::new(&runtime, [Signal::WindowChange]).unwrap();
    drop(first);
    // The second listens still.
    assert_eq!(handler(libc::SIGWINCH), taken_over);
    raise(libc::SIGWINCH);
    assert_eq!(
        runtime.block_on(second.recv()).unwrap(),
        Signal::WindowChange
    );
    drop(second);
    assert_eq!(handler(libc::SIGWINCH), libc::SIG_DFL);

    // A signal that the process ignored is ignored again.
    set_handler(libc::SIGWINCH, libc::SIG_IGN);
    let listening = Signals::new(&runtime, [Signal::WindowChange]).unwrap();
    assert_eq!(handler(libc::SIGWINCH), taken_over);
    drop(listening);
    assert_eq!(handler(libc::SIGWINCH), libc::SIG_IGN);

    // Other code set its own while a listener listened, which stays once the listener goes.
    set_handler(libc::SIGWINCH, libc::SIG_DFL);
    let listening = Signals::new(&runtime, [Signal::WindowChange]).unwrap();
    set_handler(libc::SIGWINCH, libc::SIG_IGN);
    drop(listening);
    assert_eq!(handler(libc::SIGWINCH), libc::SIG_IGN);

    set_handler(libc::SIGWINCH, libc::SIG_DFL);
}

/// Sends `raw` to the calling thread, which has run the handler by the time this returns, as POSIX
/// says of `raise`.
fn raise(raw: c_int) {
    // SAFETY: `raise` only sends a signal, which every test sends while a handler is set for it.
    let raised = unsafe { libc::raise(raw) };
    assert_eq!(raised, 0, "{}", io::Error::last_os_error());
}

/// Whether nothing is waiting for `signals`: a wait for the next one has not ended by the time a
/// short while has passed.
fn nothing_waits<M>(runtime: &Runtime<M>, signals: &mut Signals<M>) -> bool
where
    M: Mode,
{
    let waited = runtime.timeout(Duration::from_millis(50), signals.recv());

    runtime.block_on(waited).is_err()
}

/// The handler set for the signal numbered `raw`: `SIG_DFL`, `SIG_IGN` or a function.
fn handler(raw: c_int) -> libc::sighandler_t {
    let mut action = MaybeUninit::<libc::sigaction>::uninit();
    // SAFETY: with no action to set, the call only writes the one that is set, which `action` has
    // room for.
    let read = unsafe { libc::sigaction(raw, ptr::null(), action.as_mut_ptr()) };
    assert_eq!(read, 0, "{}", io::Error::last_os_error());

    // SAFETY: the call succeeded, and so wrote the action.
    unsafe { action.assume_init() }.sa_sigaction
}

/// Sets `handler` for the signal numbered `raw`, as other code than zruntime would: `SIG_DFL` or
/// `SIG_IGN`, either of which needs no function to be safe to run in a signal handler.
fn set_handler(raw: c_int, handler: libc::sighandler_t) {
    // SAFETY: `signal` sets the handler it is given, which is either of the two that run no code.
    let previous = unsafe { libc::signal(raw, handler) };
    assert_ne!(previous, libc::SIG_ERR, "{}", io::Error::last_os_error());
}
