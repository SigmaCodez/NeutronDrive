//! What `Event`'s listen and notify paths cost: taking a listener, alone and dropped at once,
//! sending a notification to nobody, waking one listener and a hundred taken beforehand, and
//! `lock::Mutex` and `lock::Semaphore`, built on an event, taken by one thread alone. `Event` needs
//! no runtime, so no id here goes through `zruntime::block_on`: they poll listeners by hand or take
//! the mutex with `try_lock` and a permit with `try_acquire`, all on one thread.
//! `mutex/4-threads`, the mutex taken by four contending threads, is in `contention.rs`, as only
//! the clock can measure it.
//!
//! Each id but `wake-all/100` repeats what it times within one timed call, [`OPS`] times or, for
//! `listen` and `wake-one`, [`LISTENERS_PER_BATCH`] times, and its throughput is in those
//! repetitions. CodSpeed's CPU simulation times a single call of each id, which starts with nothing
//! in the simulated cache. A call of a few dozen instructions, such as one notification sent to
//! nobody, then costs mostly the cache lines it touches: one line more or less, as where the stack
//! starts moves with the size of the environment, moves its time by more than the 10% that CodSpeed
//! reports as a change. A call that repeats it pays for those lines once, and costs what its
//! instructions do.
//!
//! `listen` times taking a listener, alone, on an event whose shared state is already allocated:
//! what every wait starts with. The listeners a call takes are dropped once it has been timed, so
//! the event's queue holds a batch of them at most.
//!
//! `listen-drop` times taking a listener and dropping it at once, on an event whose shared state
//! is already allocated: the common case of a listener whose wait never has to happen because the
//! condition it was taken for already held.
//!
//! `notify-none` times `notify(1)` on an initialised event with nobody listening: the path a
//! lock's release takes when nobody is waiting for it, and so what `notify` costs even where
//! there is nothing to wake.
//!
//! `mutex/uncontended` times taking and releasing zruntime's own `lock::Mutex` on one thread, with
//! nobody else after it: the commonest use of a lock, whose release notifies the mutex's event with
//! nobody listening.
//!
//! `semaphore/uncontended` times taking a permit of zruntime's own `lock::Semaphore` and giving it
//! back on one thread, with nobody else after one: what a semaphore that caps work which seldom
//! reaches the cap costs, whose release notifies its event with nobody listening.
//!
//! `wake-one` times notifying one listener and polling it once to `Ready`: the end of a wait,
//! from being told it is over to finding so, without its start, which `listen` times.
//!
//! `wake-all/100` times waking a hundred listeners with one `notify(usize::MAX)` and polling each
//! to `Ready`: a broadcast to every listener an event has, such as a connection announcing that it
//! has closed.
//!
//! Every event that `wake-one` and `wake-all/100` notify is one of their own, made and listened to
//! before the timing starts: a call of `wake-one` notifies [`LISTENERS_PER_BATCH`] events with a
//! listener each, and one of `wake-all/100` an event with a hundred. The listeners a call polls are
//! then the only ones its notifications can reach, however many calls a harness sets up before it
//! runs them: CodSpeed's CPU simulation sets up two for the one it measures.

use std::{
    future::Future,
    hint::black_box,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use zruntime::{
    Event, EventListener,
    lock::{Mutex, Semaphore},
};

/// Times `listen`, `notify` and the poll that resolves a listener, all on a single thread.
fn event_benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("event");

    let listen = Event::new();
    // Allocates the event's shared state up front, so the timed batches below never pay for that.
    listen.notify(0);
    group.throughput(Throughput::Elements(LISTENERS_PER_BATCH as u64));
    group.bench_function("listen", |b| {
        // The listeners each call takes are handed back, to be dropped once it has been timed.
        b.iter_batched(
            || Vec::with_capacity(LISTENERS_PER_BATCH),
            |mut listeners| {
                for _ in 0..LISTENERS_PER_BATCH {
                    listeners.push(listen.listen());
                }

                listeners
            },
            BatchSize::PerIteration,
        );
    });

    let listen_drop = Event::new();
    // Allocates the event's shared state up front, so the timed loop below never pays for that.
    listen_drop.notify(0);
    // For `listen-drop` and the three ids after it, which repeat what they time as many times.
    group.throughput(Throughput::Elements(OPS));
    group.bench_function("listen-drop", |b| {
        b.iter(|| {
            for _ in 0..OPS {
                drop(black_box(listen_drop.listen()));
            }
        });
    });

    let notify_none = Event::new();
    notify_none.notify(0);
    group.bench_function("notify-none", |b| {
        b.iter(|| {
            for _ in 0..OPS {
                black_box(notify_none.notify(1));
            }
        });
    });

    let uncontended = Mutex::new(());
    group.bench_function("mutex/uncontended", |b| {
        // The guard is dropped at once: the mutex is taken and released.
        b.iter(|| {
            for _ in 0..OPS {
                drop(black_box(uncontended.try_lock()));
            }
        });
    });

    let semaphore = Semaphore::new(1);
    group.bench_function("semaphore/uncontended", |b| {
        // The guard is dropped at once: the permit is taken and given back.
        b.iter(|| {
            for _ in 0..OPS {
                drop(black_box(semaphore.try_acquire()));
            }
        });
    });

    group.throughput(Throughput::Elements(LISTENERS_PER_BATCH as u64));
    group.bench_function("wake-one", |b| {
        // The events and their listeners are handed back, to be dropped once the call has been
        // timed.
        b.iter_batched(
            || {
                (0..LISTENERS_PER_BATCH)
                    .map(|_| listened_to(1))
                    .collect::<Vec<_>>()
            },
            |mut events| {
                for (event, listeners) in &mut events {
                    assert_eq!(event.notify(1), 1);
                    assert!(poll_once(&mut listeners[0]).is_ready());
                }

                events
            },
            BatchSize::PerIteration,
        );
    });

    group.throughput(Throughput::Elements(100));
    group.bench_function("wake-all/100", |b| {
        b.iter_batched(
            || listened_to(100),
            |(event, mut listeners)| {
                assert_eq!(event.notify(usize::MAX), 100);
                for listener in &mut listeners {
                    assert!(poll_once(listener).is_ready());
                }

                (event, listeners)
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

/// A new event, and `count` listeners taken on it, the oldest first.
fn listened_to(count: usize) -> (Event, Vec<EventListener>) {
    let event = Event::new();
    let listeners = (0..count).map(|_| event.listen()).collect();

    (event, listeners)
}

/// Polls `listener` once, with a waker that goes nowhere, and hands back what the poll found.
fn poll_once(listener: &mut EventListener) -> Poll<()> {
    Pin::new(listener).poll(&mut Context::from_waker(Waker::noop()))
}

/// How many times a timed call of `listen-drop`, `notify-none`, `mutex/uncontended` and
/// `semaphore/uncontended` repeats what it times: enough that the call costs what its instructions
/// do, rather than the cache lines it touches (see the module documentation).
const OPS: u64 = 1000;

/// How many listeners a timed call of `listen` takes, and how many events one of `wake-one` wakes:
/// few enough to keep the queue of the event `listen` takes its listeners on short, and enough that
/// a call costs what its instructions do, rather than the cache lines it touches, and that reading
/// the clock once a call costs little next to them.
const LISTENERS_PER_BATCH: usize = 64;

criterion_group!(benches, event_benches);
criterion_main!(benches);
