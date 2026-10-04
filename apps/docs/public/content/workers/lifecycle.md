# Lifetimes and errors

Decide what owns background work, how callers stop waiting on it, and how failures reach
your UI.

## Owners {#ownership}

Every `run`, `stream`, and `spawn` call takes the `OwnerHandle` of the component starting
the work. Disposing that owner cancels the operations it started and releases the runtimes
it owns. `.scope(&another_owner)` adds a further lifetime condition to an operation without
replacing the original owner. Extra scopes and cancellation tokens accumulate: adding one
never removes another.

A spawned service belongs to the owner passed to `spawn`. Scopes or tokens added to `Spawn`
affect initialization only, so add them to later method jobs as well if you want them there.
A pool has an owner of its own, and work placed on a pool can’t outlive it.

- [Owners and cleanup](/docs/ownership)
- [Service initialization and clients](/docs/workers/services#spawn)

## Cancelling a job {#cancellation}

`cancel_on(&token)` connects a job to a `fusor_async::CancellationToken`.
`cancellation_handle()` returns a cloneable handle with a `cancel()` method, which suits a
Cancel button. Cancelling more than once is harmless. The handle doesn’t keep the job alive:
it only lets you request cancellation while the job exists. Dropping a job that was never
polled starts nothing, and dropping a pending job requests its cancellation.

Cancelling settles the caller promptly, but it is only a request. It can’t roll back network
requests or state changes the worker has already made, and the worker’s capacity isn’t freed
until execution actually stops. Configure a job with its builder methods before the first
poll. Changing configuration after work has started produces `InvalidConfiguration`.

- [Progress and a cancellation handle in code](/docs/workers/tasks#progress)

## Letting work observe cancellation {#cooperation}

Cancelling a job stops running code only if that code cooperates by checking for
cancellation. In synchronous work, call `ctx.check_cancelled()` between chunks of CPU work.
Computations on a pool can read a cancellation flag shared with the pool while they run. On
an ordinary worker, a synchronous task can’t receive its queued cancellation message until
it returns, so repeated checks alone don’t interrupt a busy loop there.

Async work has to yield to the worker’s event loop before cancellation can be seen, and an
`await` that is immediately ready doesn’t guarantee a yield. Pass `ctx.cancellation_token()`
to I/O that supports cancellation, since such I/O cooperates by stopping when the token
fires. Terminating a whole runtime can stop work that has no explicit checks. A service
method that is already running keeps the service busy until it finishes, even if its caller
has cancelled.

## Application errors and runtime errors {#errors}

A task returns `TaskResult<T, E = NoError>`, an alias for `Result<T, JobError<E>>`.
`JobError` distinguishes two kinds of failure. `JobError::Application(E)` carries a failure
from your own logic, and `JobError::Worker` carries a `WorkerError` from Fusor. Using `?` on
a `WorkerError` produces the second one. `NoError` is a type with no values, for tasks that
have no application error. Those tasks can still fail with a runtime error.

`E` must be a `Message`, the same requirement that applies to other task values, but it
doesn’t have to implement `std::error::Error`; `String` works. `JobError<E>` implements
`Display` when `E` does, and implements `Error` when `E: Error + 'static`. `JobError` and
`WorkerError` are both non-exhaustive, so a `match` outside the crate needs a fallback arm.

```rust source=tutorial/lessons/workers/errors.rs title=Rust · errors.rs
```

- [Map a Fetch error](/docs/workers/tasks#fetch)
- [JobError reference](/docs/workers/api#job-error)

## WorkerError reference {#runtime-errors}

`Cancelled` and `OwnerDisposed` report that the caller cancelled or that the owner was
disposed. They don’t prove that the remote work has ended. `Closed`, `Terminated`, and
`CloseTimedOut` relate to shutdown. `QueueFull { capacity }`,
`PayloadTooLarge { limit, actual }`, and `InvalidConfiguration { message }` report exceeded
limits or invalid builder settings.

`Unsupported { capability }` means the browser lacks a capability the operation needs, and
`PoolRequired` means the operation needs a pool that wasn’t selected. Shared data can fail
with `WrongPool`, `StaleShared`, or `SharedTypeMismatch`. `Encode`, `Decode`, `Load`, and
`Crashed` carry a message. A poisoned worker progress slot or shared allocation registry is
reported as `Crashed`; cleanup still releases pending shared transfers.
`IncompatibleArtifact` means the packaged worker code and the runtime use incompatible
protocols, and the deployment guide covers load problems. Whether to retry is your
application’s decision, since Fusor doesn’t replay operations that may have changed state.

- [Diagnose startup and hosting failures](/docs/workers/deployment#troubleshooting)
- [WorkerError variants](/docs/workers/api#worker-error)

## Closing services and pools {#close}

`client.close()` and `pool.close()` return a lazy `Close` future. When it is first polled,
it stops admitting new calls and waits for the accepted operations to finish. Calling close
again joins the same closure, and dropping the `Close` future doesn’t reopen the runtime.
The wait has a five-second deadline by default, which you can change with
`.timeout(Duration::from_secs(10))` before polling.

When the deadline passes, affected callers receive `CloseTimedOut`. Closing a pooled service
leaves its pool running, and if a method is still running, its state is kept alive until the
method returns. Only `Pool` has `terminate()`, which stops the whole pool immediately and
invalidates its shared handles. Dropping the `Pool` has the same abrupt effect.

- [Close reference](/docs/workers/api#close)

## Recovering from a crash {#recovery}

A panic or Wasm trap invalidates the affected dedicated runtime, or the entire pool, and
callers see `Crashed`. Rebuilding during development also terminates the old runtimes, and
clients or shared handles created before the refresh don’t carry over to the new code.

To recover, create a new owner, pool, or service. Fusor doesn’t replay operations, because a
caller whose operation failed can’t tell whether the worker had already changed state.
