# Shared data

Keep a large value inside a pool, and pass a small typed handle to the operations that need
it.

## What a handle refers to {#handles}

Passing a large value to a worker normally means encoding and copying it on every call.
`Shared<T>` avoids that by keeping the value inside a pool and giving the page a handle to
it. The handle is an opaque lease on the pool’s allocation, and `T` must be
`Send + Sync + 'static`. `T` needs no Serde implementation, because the value itself never
travels as a message.

Inside pool code, `ctx.share(value)` creates the allocation and returns a handle, and
`ctx.resolve(&handle)` returns an `Arc<T>` for it. Both are available on either context
type. The page can clone, hold, drop, and forward the handle, but it can’t dereference it or
read the data. Despite the similar name, `Shared<T>` has nothing to do with the browser’s
SharedWorker API: it belongs to one Fusor pool, not to a worker shared between tabs.

- [Shared&lt;T&gt; reference](/docs/workers/api#shared)

## Create once and reuse {#example}

A typical setup has two kinds of task, both run on the same pool with `.on(&pool)`. One
builds the value inside the pool and returns a `Shared<T>` handle. The other takes a handle
as an argument and calls `ctx.resolve` to reach the data. Any number of later calls can
resolve the same allocation from a retained handle.

Only the handle is encoded for each call, so reusing the value doesn’t re-encode it. If the
initial data is owned by the page, those bytes still have to be sent once to create the
allocation.

```rust source=tutorial/lessons/workers/shared.rs title=Rust · shared.rs
```

## Passing handles as arguments and results {#messages}

A shared handle can be a direct task argument, a direct result, or a stream item. It can’t
be nested inside a Serde struct, tuple, or collection, because `Shared<T>` deliberately has
no Serde implementation. When a task needs a handle together with ordinary settings, give it
several direct parameters instead of bundling them into one value.

An `Arc<T>` gives read-only access to `T`. To mutate shared data, put a thread-safe type
such as an atomic or a `Mutex` inside `T`, and avoid holding a lock across an await.

## How handle lifetimes work {#lifetime}

The allocation stays retained for as long as any relevant handle or `Arc<T>` exists,
provided the pool is still alive. Cloning a handle keeps that same allocation retained.
Sending a handle also retains it until the transfer is delivered or fails. An `Arc<T>` held
by a running computation keeps the allocation alive even if the page has dropped its handle.

Fusor validates a handle whenever it is used. A handle from a different pool fails with
`WrongPool`, an expired handle with `StaleShared`, and a handle of a different type than
expected with `SharedTypeMismatch`. Types are checked against the actual allocation when a
handle is received or resolved inside its pool. The page holds an opaque handle, so it can
use a different Rust toolchain from the threaded worker. Closing or terminating a pool
invalidates its handles, and calls made afterwards may fail with `Closed`, `Terminated`, or
`OwnerDisposed` before validation is reached. During a graceful close, admitted work can
still finish using its data before the handles are invalidated.

- [Understand pool shutdown](/docs/workers/lifecycle#close)
