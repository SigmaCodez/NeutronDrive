//! Signals on Windows: the console's Ctrl-C and Ctrl-Break events, through a handler routine that
//! the console calls on a thread of its own, which hands each event to the listeners there and
//! then.

use std::{
    io,
    marker::PhantomData,
    task::{Context, Poll},
};

use windows_sys::{
    Win32::{
        Foundation::{FALSE, TRUE},
        System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler},
    },
    core::BOOL,
};

use super::{Set, Signal, lock, wake};
use crate::{Mode, Runtime, scheduler::dispose};

/// What a `Signals` waits on for signals to arrive: nothing, as the handler routine wakes the
/// listeners itself.
///
/// The pointer that a runtime of the flavour shares its state by is what the type is tied to its
/// flavour through, so that a `Signals` built on a local runtime stays on its thread on every
/// platform, as the `AsyncIo` of unix keeps it there.
pub(super) struct Watch<M>(PhantomData<M::Ptr<()>>)
where
    M: Mode;

impl<M> Watch<M>
where
    M: Mode,
{
    /// A watch for a `Signals` on `runtime`, which needs nothing of the runtime.
    pub(super) fn new(_runtime: &Runtime<M>) -> io::Result<Self> {
        Ok(Self(PhantomData))
    }

    /// Never resolves: the handler routine hands each event to its listeners as it comes, and
    /// wakes them.
    pub(super) fn poll_deliver(&self, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

/// Nothing, as nothing waits to be handed out outside the registry: the handler routine hands
/// each event to the listeners as it comes, with the registry locked.
pub(super) fn arrived() -> Set {
    Set::EMPTY
}

/// Whether the handler routine has been added to the console's.
///
/// It is added by the first listener, and stays: it passes on every event that nothing listens
/// for, which is what removing it would come to, and adding it again later would put it ahead of
/// the routines added in between, as the console calls the last one added first.
pub(super) struct Handlers {
    added: bool,
}

impl Handlers {
    pub(super) const fn new() -> Self {
        Self { added: false }
    }

    /// Adds the handler routine, unless it is added already.
    pub(super) fn take(&mut self, _signal: Signal) -> io::Result<()> {
        if self.added {
            return Ok(());
        }
        // SAFETY: `handle` is a handler routine, as the console calls one, for the life of the
        // process: it is never removed.
        if unsafe { SetConsoleCtrlHandler(Some(handle), TRUE) } == FALSE {
            return Err(io::Error::last_os_error());
        }
        self.added = true;

        Ok(())
    }

    /// Leaves the handler routine in place, which passes on the events of `_signal` from now on.
    pub(super) fn give_back(&mut self, _signal: Signal) {}
}

/// The handler routine: hands the event to its listeners, and takes it if there are any.
///
/// Where there are none, it passes the event on to the routines added before it, the last of which
/// is the default one, which ends the process. So do the other events, of the console closing, the
/// user logging off and the system shutting down, which are no signals here.
///
/// A waker that panics as the event is handed out keeps no other listener from being woken, and
/// its panic goes no further than here: no code of the program's could catch it on the console's
/// thread, and one that unwound out of the routine would end the process.
pub(crate) extern "system" fn handle(event: u32) -> BOOL {
    let signal = match event {
        CTRL_C_EVENT => Signal::Interrupt,
        CTRL_BREAK_EVENT => Signal::Break,
        _ => return FALSE,
    };

    let delivered = lock().deliver(signal.into());
    let Some(woken) = delivered else {
        return FALSE;
    };
    // Clear of the lock, which went with the statement above. The panic hook has reported a panic
    // of a waker already.
    if let Some(panic) = wake(woken) {
        dispose(panic);
    }

    TRUE
}
