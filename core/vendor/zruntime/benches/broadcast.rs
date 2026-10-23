//! What a `broadcast` channel costs per message: one `Sender::broadcast` followed by a
//! `Receiver::recv` on each of 1, 2, 4 and 8 receivers, driven on a single thread with
//! `futures::executor::block_on`. The channel has a capacity of one, so every message is
//! received by all receivers before the next one is sent. `broadcast` needs no runtime.
//!
//! A timed call sends [`MESSAGES`] messages, each in a `block_on` of its own, and the throughput is
//! in messages. CodSpeed's CPU simulation times a single call of each id, which starts with nothing
//! in the simulated cache. A call of one message, a few hundred instructions, would cost mostly the
//! cache lines it touches, and move with where they lie, as where the stack starts moves with the
//! size of the environment. A call of many pays for those lines once, and costs what its
//! instructions do.

use std::num::NonZeroUsize;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use futures::executor::block_on;
use zruntime::broadcast::channel;

/// Times sending one message and receiving it on every one of a growing set of receivers.
fn broadcast_and_recv(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadcast");

    let (s, r1) = channel(NonZeroUsize::MIN);
    let mut receivers = vec![r1];
    let mut n = 0u64;
    group.throughput(Throughput::Elements(MESSAGES));

    for count in [1, 2, 4, 8] {
        while receivers.len() < count {
            let r = receivers[0].new_receiver();
            receivers.push(r);
        }
        group.bench_function(format!("1-to-{count}"), |b| {
            b.iter(|| {
                for _ in 0..MESSAGES {
                    block_on(async {
                        s.broadcast(n).await.unwrap();
                        for r in receivers.iter_mut() {
                            assert_eq!(r.recv().await.unwrap(), n);
                        }
                        n += 1;
                    })
                }
            })
        });
    }

    group.finish();
}

/// How many messages a timed call sends: enough that the call costs what its instructions do,
/// rather than the cache lines it touches (see the module documentation).
const MESSAGES: u64 = 100;

criterion_group!(benches, broadcast_and_recv);
criterion_main!(benches);
