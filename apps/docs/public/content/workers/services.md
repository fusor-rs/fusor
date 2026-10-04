# Stateful workers

Keep a Rust value alive in a worker, and call its methods from the page through a generated,
typed client.

## Annotate the implementation {#worker}

A task starts from its arguments every time it runs. A stateful worker keeps a Rust value
alive across calls, which is worthwhile when rebuilding that state for each call would be
wasteful. `#[fusor_worker::worker]` on a nongeneric inherent `impl` block generates a client
for the type.

The `impl` needs a constructor named `new` that takes one owned input and returns
`TaskResult<Self, E>`. Each public method takes `&mut self` and one owned input: use `()`
when there is nothing to pass, or a struct or tuple to carry several values. Private helper
methods remain ordinary Rust methods and don’t appear on the client. The name `close` is
reserved for the generated client’s shutdown method.

```rust source=tutorial/lessons/workers/services.rs title=Rust · services.rs
```

## Spawn the service and get its client {#spawn}

`spawn::<T>(&owner, input)`, where `T` is your annotated type, returns a lazy `Spawn<T>`.
Awaiting it initializes the service by running the constructor with the input, and yields a
cloneable typed client. By default the service gets its own dedicated worker, which is
started for it. If you place the service on a pool, it uses that existing pool instead of
starting a worker. You keep using your own type as the type parameter, and there is no
separate client type to declare or import. When you need to name the client type explicitly,
use `<T as fusor_worker::Worker>::Client`.

Each public method on the client returns a job for that service instance. Clones of a client
address the same state, and spawning again creates an independent instance. Before awaiting
`Spawn`, you can set `on_progress`, `cancel_on`, or `scope` for initialization. Those
settings apply only to initialization. The running service belongs to the owner that was
passed to `spawn`.

- [Understand the service’s owner](/docs/workers/lifecycle#ownership)
- [Spawn signatures](/docs/workers/api#spawn)
- [Generated client methods](/docs/workers/api#client)

## Methods run one at a time {#methods}

Constructors and methods can be synchronous or async. Either can declare a final
`TaskContext<P>` parameter, which Fusor supplies, so callers leave it out. Documentation
comments, deprecation attributes, and conditional compilation on public methods carry over
to the generated client.

Calls to one instance run serially, including across awaits inside a method. Cancelling a
call doesn’t change that: a method that is already running finishes before the next call
starts. If a cancellation is processed while a call is still queued, that call doesn’t run.
A busy ordinary worker may start the call before it gets to the cancellation message,
though, so a cancelled call can still occasionally run. A client method returns a `Job` that
is already bound, so you can set progress, cancellation, and scopes on it per call. Pool
placement is chosen when the service is spawned, not on individual method jobs.

## Choose a dedicated worker or a pool {#placement}

By default, each service gets its own dedicated worker. To place a service in a pool, select
the pool on the spawn: `spawn::<T>(&owner, input).on(&pool).await`. Writing
`#[fusor_worker::worker(pool)]` makes that selection mandatory.

On a dedicated worker, a service’s constructor and methods run on that worker’s event loop,
even when they are synchronous. In a pool, they run on the pool’s coordinator thread, the
one that runs async code. Either way, long CPU work would hold up that thread, and only an
explicit `ctx.compute` moves work elsewhere. So make the method async and call
`ctx.compute(move |cpu| ...)` to run the work on the pool’s compute threads. `compute` needs
a pool: on a service in an ordinary dedicated worker it returns `PoolRequired`. Move owned
data into the closure instead of borrowing `self`, because the closure runs on another
thread.

- [Use TaskContext::compute](/docs/workers/pools#compute)

## Release the service {#close}

Await `client.close()` when you are done with a service, or let the owner that spawned it
dispose it. Don’t rely on dropping client clones to stop a service. A dedicated service may
end once nothing references its runtime any more, but a service on a live pool can be
retained by the pool. Only `close()` or disposal of the owner shuts it down dependably.
Closing one service in a pool leaves the pool’s other services and tasks running.

A graceful close lets calls that were already admitted finish. If a close deadline passes
while a pooled method is still running, callers stop waiting, but Fusor keeps the service’s
state alive until that method returns.

- [Close deadlines and termination](/docs/workers/lifecycle#close)
