# Pools and compute threads

Run CPU-heavy work on a fixed set of threads while a separate coordinator thread keeps
handling async operations.

## Create and configure a pool {#create}

`Pool::new(&owner)` returns a lazy `PoolInit`. Before awaiting it, you can set `max_threads`
for the number of compute threads, `max_async_jobs` for how many async operations may be
active at once, and `queue_capacity` for how many operations may wait. Awaiting it finishes
once the coordinator and every compute thread are ready. You can cancel initialization with
`cancel_on` or a cancellation handle.

A pool consists of one coordinator thread, which runs async code, plus the compute threads,
which run synchronous CPU work. By default it uses up to four compute threads, limited by
the browser’s hardware concurrency hint, and allows four active coordinator operations with
64 more waiting. After initialization, `pool.threads()` reports how many compute threads
were selected. Zero is not a valid thread or async limit. `queue_capacity(0)` is valid and
means work is rejected whenever no execution slot is free.

- [Prepare a threaded build](/docs/workers/deployment#threads)
- [Check browser capabilities](/docs/workers/deployment#capabilities)
- [PoolInit signatures](/docs/workers/api#pool-init)
- [Pool methods](/docs/workers/api#pool)

## Choose where each operation runs {#placement}

Call `.on(&pool)` on a generated task job, a stream, or a `spawn` before polling it to run
it in that pool. Annotations can make this mandatory: `#[task(pool)]` and `#[worker(pool)]`
fail with `PoolRequired` when no pool has been selected. Fusor never falls back to running
such work on the UI thread.

Where code runs inside the pool depends on the kind of operation. Synchronous tasks run on
compute threads. Async tasks, stream producers, service constructors, and service methods
run on the coordinator. Several async operations can be in flight at once while they wait on
I/O, but their synchronous stretches still share that one coordinator thread.
`max_async_jobs` limits how many coordinator operations are active at a time, and each
service still handles its own calls one at a time.

## Move CPU work to a compute thread {#compute}

Async code runs on the coordinator, so a long CPU loop there would delay everything else the
coordinator is doing. `TaskContext::compute` hands such work to the pool’s compute threads.
It takes an owned synchronous closure, `FnOnce(ComputeContext<P>) -> R + Send + 'static`
with `R: Send + 'static`, and awaiting it returns `Result<R, WorkerError>`. The
`ComputeContext` passed to the closure is thread-safe and provides cancellation checks,
progress reporting, and shared data.

When the closure itself returns a `Result`, awaiting `compute` produces two layers. The
outer one reports scheduling and runtime failures, and the inner one is the closure’s own
error, so you write `.await??` to unwrap both. A closure can instead return
`TaskResult<T, E>` when it needs to report an application error.

Waiting `compute` calls have their own allowance, which uses the same number as
`queue_capacity` but is counted separately from the pool’s outer operation queue. If that
allowance overflows, the call fails with `QueueFull { capacity }`.

```rust source=tutorial/lessons/workers/pools.rs title=Rust · pools.rs
```

## What a pool parallelizes {#parallelism}

A pool lets independent operations overlap. It doesn’t split up a single computation.
Independent synchronous jobs run at the same time only when they are polled concurrently:
creating several jobs and then awaiting each in turn submits them one after another. A
single sequential loop occupies one compute thread, however many the pool has.

To parallelize an algorithm itself, use Rayon inside pool CPU work, and add Rayon as a
direct dependency of your application because Fusor doesn’t re-export its internal copy.
Ordinary browser requests usually need async I/O rather than more CPU threads. Two costs are
worth separating. Starting worker threads is paid when a runtime is created, and existing
runtimes are reused by later calls. Encoding and decoding messages is paid on every call.
Measure whether the work per call is large enough to outweigh the per-call cost.

## Queues, limits, and pool lifetime {#budget}

Operations waiting for a slot sit in a bounded admission queue, and an operation that
overflows it fails with `QueueFull { capacity }`. `ctx.compute` has a separate waiting
allowance that uses the same `queue_capacity` number, and it overflows with the same error.
Cancelling a caller doesn’t free a running slot. The slot is released only when the remote
computation actually stops.

Each pool has its own Wasm memory and its own limits. Keep the `Pool` value for as long as
you use it: dropping it, disposing its owner, or calling `terminate()` stops its runtime. A
graceful `close().await` lets admitted work finish before the pool’s shared handles become
invalid.

- [Close and failure behavior](/docs/workers/lifecycle#close)
- [Reuse allocations within one pool](/docs/workers/shared)
