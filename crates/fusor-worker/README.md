# fusor-worker

Background tasks, persistent worker services, bounded streams and shared-memory
pools for fusor. The [guide](https://fusor.build/docs/workers) describes the public
API; [examples/workers](../../examples/workers) is a normal application using it.

```rust
use fusor_worker::TaskResult;

#[fusor_worker::task]
pub fn total(values: Vec<u64>) -> TaskResult<u64> {
    Ok(values.into_iter().sum())
}
// In an async UI operation: total::run(&owner, values).await
```

## Compilation and runtime boundaries

The attribute retains the authored item and generates a typed proxy, dispatcher,
and `inventory` registration. Identity includes package name/version and Rust
module path. A versioned wasm-bindgen custom section exposes compiled annotation
metadata without initializing the application. An imported capability marker
retained by pool construction/selection also selects threaded packaging. The CLI
never scans Rust source or asks a proc macro to write files. Worker assets live
inside the application’s immutable generation.

Automatic packaging requires an `<App>` entry. The compiler's internal
`FUSOR_WORKER_BUILD` mode excludes its generated startup roots. The same
application, dependency graph and feature flags compile in
separate target directories; dead-code elimination removes unrelated UI imports.
The bootstrap validates protocol version and registration uniqueness. Workers
cannot execute DOM-dependent code called by their own entry points.

Ordinary builds use stable Rust. Threaded builds pin `nightly-2025-11-15`,
Rayon 1.11.0 and wasm-bindgen-rayon 1.3.0, rebuilding std with atomics, bulk memory,
shared/imported memory and TLS exports. The CLI replaces the adapter's worker host
using its pinned public PoolBuilder ABI. Every physical compute worker belongs
to the UI runtime, so a blocked/crashed coordinator cannot prevent termination.
No unsafe code is introduced in fusor. The existing reviewed dependency adapter
owns the Wasm/Rayon thread initialization boundary.

Each pool has an independent shared Wasm memory and Rayon executor. The coordinator
admits async work and serial service calls, while synchronous tasks run on compute
threads. Public admission and `TaskContext::compute` have separate bounded queues.
Caller cancellation never releases a running slot prematurely. Pool lifetime is
owned by `Pool` and its fusor owner; clients cannot keep it logically alive.

## Wire ownership

Protocol version 1 carries JSON values or explicit shared leases. Ordinary values
are encoded with a bounded writer (16 MiB); `Shared<T>` has no Serde implementation.
A shared token names a pool, generation and allocation, never a Rust pointer.
Its type name is diagnostic only: the UI and worker compilers can spell the same
type differently. A safe `Any` registry holds `Arc<T>` allocations and checks their
concrete types when handles return to the pool. Transfers retain before sending;
failed encoding/delivery and ignored results release provisional leases. Local
clones share an endpoint lease, and in-flight `Arc`s independently retain data.

UI operation state owns weak cleanup/cancellation registrations. Progress is
coalesced on both sides. Stream credits cover accepted items in transit and at the
consumer; the worker channel preserves delivery order before terminal commit.
Abort discards pending output, wakes blocked sends and wins over an
uncommitted stream ending. Terminal job results win in UI event order.

Graceful closure stops admission and drains. Service timeout in a pool is isolated;
its running state remains until the method returns. Pool timeout, panic, owner
cleanup and explicit termination kill physical workers and invalidate leases.
Nothing retries a mutation automatically.

## Verification

`just test worker` checks native authoring diagnostics and an independent release
LTO browser consumer. `just test worker-pool` adds actual shared-memory execution,
compute placement, leases, cancellation, isolation, failure cleanup and CPU/I/O
separation. `just test worker-dev` verifies rebuilding and runtime replacement
through the normal dev server. Set `PLAYWRIGHT_BROWSERS=chromium,firefox,webkit`
for all engines.
