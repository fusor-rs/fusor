# Worker API reference

Every type you’ll meet when you move work off the page: what it’s for, where it comes from,
and what you can do with it. Each entry starts with a short example, and the exact
signatures are one click away.

## Which type is which {#types}

Background work happens in two places. Page code, meaning your components and event
handlers, starts the work and gets the results back. Worker code, meaning the functions and
methods you annotate, does the work on another thread.

The lists group types by whether you use them on the page, in worker code, or on both
sides. Follow a link to open its reference.

### Start work from the page

- [`Job`](#job) (struct) — One background call. Await it for the result.
- [`ResultStream`](#result-stream) (struct) — A background call that sends results back in
  batches.
- [`spawn`](#spawn) (function) — Starts a worker that keeps its state between calls.
- [`Client`](#client) (generated type) — What you call a stateful worker’s methods on.

### Control it from the page

- [`Pool::new`](#pool-init) (function) — Starts a set of threads for heavy or parallel work.
- [`Pool`](#pool) (struct) — Running threads you can place work on.
- [`CancellationHandle`](#cancellation-handle) (struct) — Lets a Cancel button stop one
  call.
- [`Close`](#close) (struct) — Shuts a pool or a stateful worker down gracefully.
- [`capabilities`](#capabilities) (function) — Checks what the current browser supports.

### Receive in worker code

- [`TaskContext`](#task-context) (struct) — Progress, cancellation and pool access for async
  code.
- [`ComputeContext`](#compute-context) (struct) — The same tools for heavy synchronous code.
- [`StreamSender`](#stream-sender) (struct) — Sends a stream’s results back to the page.

### Use on both sides

- [`TaskResult`](#task-result) (type alias) — What every task returns: a value or an error.
- [`JobError`](#job-error) (enum) — Tells you whether your code or Fusor failed.
- [`WorkerError`](#worker-error) (enum) — Every failure that comes from Fusor.
- [`Shared`](#shared) (struct) — A small handle to a large value kept in a pool.
- [`Message`](#message) (trait) — What a value needs in order to cross between threads.

- [New to workers? Start with the overview](/docs/workers)
- [Write your first task](/docs/workers/tasks)

## Words you’ll see {#glossary}

These words come up throughout the entries. The guides explain each one in more depth.

- **Page code** — Code that runs on the browser’s main thread: your components, templates
  and event handlers. It’s the only code that can touch the page’s DOM.
- **Worker** — A separate browser thread that runs your Rust, compiled to WebAssembly. It
  can’t touch the page, so it sends its results back as messages.
- **Owner** — The `OwnerHandle` that Fusor gives each component to track its lifetime. Work
  you start is tied to an owner, and it’s cancelled when that component goes away.
- **Lazy** — Creating a job, stream or spawn doesn’t start anything. It starts the first
  time you await it, and until then you can still configure it.
- **Poll** — What `.await` does behind the scenes. When an entry says “the first poll”, read
  it as “the first time you await it”.
- **Message** — A value that can be copied to another thread: owned data such as numbers,
  `String`, `Vec<T>` or your own Serde types. See `Message`.
- **Progress update** — A small value that worker code reports while it runs, such as a
  count or a percentage. If updates arrive faster than the page can use them, only the
  newest is kept.
- **Pool** — A group of threads that share memory. One coordinator thread runs async code,
  and the compute threads run heavy synchronous work.
- **Placement** — Where an operation runs. By default that’s its owner’s own worker. You can
  choose a pool instead with `.on(&pool)`.
- **Cancellation** — A request to stop waiting. Your `.await` finishes with `Cancelled`
  straight away, but the worker stops only when its code checks, and nothing it has already
  done is undone.

## Reading the signatures {#reading}

Each entry begins with an example. The exact Rust signature for a method is one click away,
under “Signature”. If Rust syntax is new to you, here’s what the common pieces mean.

- **`self`** — The method takes the value and hands back a configured one, so you can chain
  calls: `job.cancel_on(&token).on_progress(show)`.
- **`&self`** — The method only borrows the value, so you can keep using it afterwards.
- **`&mut self`** — The method needs the value to itself for a moment, as `stream.next()`
  does.
- **`async fn … -> T`** — Returns a future. Nothing happens until you `.await` it, and
  awaiting gives you a `T`.
- **`Result<T, E>`** — Either `Ok(value)` or `Err(error)`. In a function that returns a
  `Result`, `?` passes an error straight back to the caller.
- **`impl FnMut(P) + 'static`** — Any closure that takes a `P`. `'static` means it can’t
  borrow local variables, so use a `move` closure with owned or cloned values.
- **`Send + 'static`** — Safe to move to another thread, and holding no borrowed references.
- **`Arc<T>`** — A shared, read-only pointer to a `T`. Cloning it doesn’t copy the `T`.

> Worker types come from `fusor_worker`. `OwnerHandle` is `fusor::OwnerHandle`,
> `CancellationToken` is `fusor_async::CancellationToken`, and `Arc` is `std::sync::Arc`.
> Declarations leave out bodies and private fields: they’re for looking things up, not for
> copying into a file.

## Job {#job}

**struct** · Page code

A `Job` is one background call that hasn’t run yet. When you await it, Fusor sends the call
to a worker and gives you the result.

Before you await it, you can ask for progress updates, connect a way to cancel, or choose a
pool.

**Where you get one**

Call a task’s generated `run` function, such as `sum::run(&owner, 100)`. Methods on a
service `Client` return jobs too.

```rust title=Rust · page code
// `sum` is a task that reports how many values it has added so far (a u32).
let total = sum::run(&owner, 100)                // creates the job; nothing runs yet
    .on_progress(move |done| progress.set(done)) // configure it…
    .cancel_on(&token)
    .await?;                                     // …then await: it runs and gives you a u64
```

> **Good to know**
>
> Nothing runs until you await. A job that you create and then drop never reaches a worker.

> **Watch out**
>
> Cancelling stops you waiting, not the work itself. The worker stops only when its code
> checks for cancellation, and anything it has already changed stays changed.

### Type parameters

- **`T`** — What you get when the job succeeds: the task’s `Ok` value.
- **`E`** — The task’s own error type, or `NoError` if it has none.
- **`P`** — The type of progress update, or `()` if the task doesn’t report progress.
- **`M`** — `Unbound` while you can still choose a pool, and `Bound` after that. See Bound
  and Unbound.

### Configure it before awaiting

#### `.on_progress(callback)` {#job-on-progress}

Returns `Job`.

Runs `callback` on the page with each progress update. Show the latest value rather than
counting calls, because an update can be skipped when a newer one arrives.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn on_progress(self, callback: impl FnMut(P) + 'static) -> Self
```

A second call replaces the first callback. The closure runs on the page, so it can capture
signals, but it can’t borrow local variables.

</details>

#### `.cancel_on(&token)` {#job-cancel-on}

Returns `Job`.

Cancels the job when `token` is cancelled. Your `.await` then gives you
`WorkerError::Cancelled`.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn cancel_on(self, token: &CancellationToken) -> Self
```

Each call adds another way to cancel the job. It never replaces the owner or a token you
added earlier.

</details>

#### `.scope(&owner)` {#job-scope}

Returns `Job`.

Also cancels the job when a second owner goes away, such as a dialog that started it. Your
`.await` then gives you `WorkerError::OwnerDisposed`.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn scope(self, owner: &OwnerHandle) -> Self
```

The owner you passed to `run` still applies. Scopes add up, like tokens do.

</details>

#### `.on(&pool)` {#job-on}

Returns `Job<…, Bound>`.

Runs the job on `pool` instead of on the owner’s own worker. You can choose a pool only
once.

<details>
<summary>Signature and details</summary>

```rust title=Rust
impl<T: Message, E: Message, P: Message> Job<T, E, P, Unbound> {
    pub fn on(self, pool: &Pool) -> Job<T, E, P, Bound>;
}
```

It’s available only while the job is `Unbound`, which is what `run` returns. It also ties
the job to the pool owner’s lifetime.

</details>

### Cancel it later

#### `.cancellation_handle()` {#job-cancellation-handle}

Returns `CancellationHandle`.

Gives you a handle your UI can keep, for a Cancel button, say. Getting one doesn’t start the
job.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn cancellation_handle(&self) -> CancellationHandle
```

It only borrows the job, so you can take a handle before or after you start awaiting.

</details>

### Run it

#### `.await` {#job-await}

Returns `TaskResult<T, E>`.

Starts the job and waits for the result. You get `Ok(value)`, or an error that says whether
your code or Fusor failed.

<details>
<summary>Signature and details</summary>

```rust title=Rust
impl<T: Message, E: Message, P: Message, M> Future for Job<T, E, P, M> {
    type Output = TaskResult<T, E>;
}
```

A job runs once. Dropping it before you await it runs nothing, and dropping it while it runs
asks the worker to cancel. Once a result has arrived, a later cancel can’t replace it.

A `Job` isn’t `Clone` or `Send`. Changing its settings after it has started fails with
`InvalidConfiguration`.

</details>

<details>
<summary>Full declaration</summary>

```rust title=Rust
pub struct Job<T, E = NoError, P = (), M = Bound> { /* private fields */ }

impl<T: Message, E: Message, P: Message, M> Job<T, E, P, M> {
    pub fn on_progress(self, callback: impl FnMut(P) + 'static) -> Self;
    pub fn cancel_on(self, token: &CancellationToken) -> Self;
    pub fn scope(self, owner: &OwnerHandle) -> Self;
    pub fn cancellation_handle(&self) -> CancellationHandle;
}

impl<T: Message, E: Message, P: Message> Job<T, E, P, Unbound> {
    pub fn on(self, pool: &Pool) -> Job<T, E, P, Bound>;
}

impl<T: Message, E: Message, P: Message, M> Future for Job<T, E, P, M> {
    type Output = TaskResult<T, E>;
}
```

</details>

- [When jobs start and where they run](/docs/workers/tasks#run)
- [Cancelling a job](/docs/workers/lifecycle#cancellation)

## Bound and Unbound {#placement}

`Job`, `ResultStream` and `Spawn` carry a marker type that answers one question: can you
still choose a pool for this operation? You’ll rarely write these names yourself, but you’ll
see them in signatures and compiler errors.

```rust title=Rust · types
Job<T, E, P, Unbound> // from name::run: a pool can still be chosen
Job<T, E, P, Bound>   // after .on(&pool), or from a service client method
Job<T>                // shorthand for Job<T, NoError, (), Bound>
```

- **`Unbound`** — Yes. `.on(&pool)` is available, and it gives you a `Bound` value. If you
  never choose, the work runs on the owner’s own worker. The exception is a task declared
  with `pool`, which fails with `PoolRequired` instead.
- **`Bound`** — No: placement is settled, so there’s no `.on`. Service method jobs are
  `Bound` because the service’s place was chosen when it was spawned, and that place might
  be a dedicated worker rather than a pool.

> The defaults differ: `Job` defaults to `Bound`, while `ResultStream` and `Spawn` default
> to `Unbound`. The marker doesn’t say whether an operation is running or finished.

- [Choose where each operation runs](/docs/workers/pools#placement)

## TaskContext {#task-context}

**struct** · Worker code

`TaskContext` is what Fusor gives async worker code while it runs. Use it to report
progress, notice cancellation, move heavy work onto a pool’s compute threads, and reach
shared data.

**Where you get one**

Add it as the last parameter of an async task, or of a service constructor or method (async
or not). In a stream task it goes just before the `StreamSender`. Fusor fills it in, so
callers never pass it.

```rust title=Rust · worker code
#[fusor_worker::task(pool)]
pub async fn summarize(values: Vec<u64>, ctx: TaskContext) -> TaskResult<u64> {
    // A long loop would hold up the pool’s async thread, so hand it to a compute thread.
    let total = ctx
        .compute(move |cpu| -> Result<u64, WorkerError> {
            let mut total = 0;
            for value in values {
                cpu.check_cancelled()?; // stop early if the caller cancelled
                total += value;
            }
            Ok(total)
        })
        .await??; // outer ?: Fusor’s error, inner ?: the closure’s
    Ok(total)
}
```

> **Note**
>
> A `TaskContext` stays on the thread that runs your operation. You can’t clone it or move
> it into a closure. The `compute` closure gets its own `ComputeContext` instead.

### Type parameters

- **`P`** — The type of progress update this code reports. It defaults to `()`, which means
  no progress.

### Report progress

#### `.report(update)` {#task-context-report}

Returns `()`.

Sends `update` to the page’s `on_progress` callback. It returns straight away and can’t
fail, so there’s no `?` and no `.await`.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn report(&self, update: P)
```

If an older update hasn’t been delivered yet, the new one replaces it. Updates sent after
cancellation are dropped. If an update can’t be encoded, the operation reports that error
when it finishes.

</details>

### Notice cancellation

#### `.check_cancelled()` {#task-context-check-cancelled}

Returns `Result<(), WorkerError>`.

Returns `Err(WorkerError::Cancelled)` once the caller has cancelled. Call it between chunks
of work, and pass the error on with `?`.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn check_cancelled(&self) -> Result<(), WorkerError>
```

It only checks: stopping is up to your code. An ordinary worker receives the cancel message
only when your code gives control back to it, so the check can keep returning `Ok` for a
while after the caller has cancelled.

</details>

#### `.cancellation_token()` {#task-context-cancellation-token}

Returns `CancellationToken`.

A token to pass to I/O that understands cancellation, such as
`fusor_async::fetch::get_text`, so the request stops when this operation is cancelled.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn cancellation_token(&self) -> CancellationToken
```

It’s for listening only and can’t cancel anything itself. To cancel from the page, use a
`CancellationHandle` or the token you gave to `cancel_on`. Only `TaskContext` has this
method.

</details>

### Use a pool’s compute threads

#### `.compute(work).await` {#task-context-compute}

Returns `Result<R, WorkerError>`.

Runs the closure `work` on one of the pool’s compute threads and gives you what it returns.
Use it for heavy loops, so the async code around them stays responsive.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub async fn compute<R, F>(&self, work: F) -> Result<R, WorkerError>
where
    F: FnOnce(ComputeContext<P>) -> R + Send + 'static,
    R: Send + 'static
```

It needs a pool, and fails with `PoolRequired` without one. The closure must own what it
uses (a `move` closure), so it can’t borrow `self`, the `TaskContext`, or page-only values
such as signals. It receives a `ComputeContext` for progress and cancellation.

When the closure returns a `Result` of its own, you get two layers of `Result`, and
`.await??` unwraps both. It can also fail with `Cancelled`, `QueueFull` or `Terminated`.

</details>

### Shared data (pools only)

#### `.share(value)` {#task-context-share}

Returns `Result<Shared<T>, WorkerError>`.

Stores `value` inside the pool and returns a small `Shared<T>` handle that you can send back
to the page.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn share<T: Send + Sync + 'static>(&self, value: T) -> Result<Shared<T>, WorkerError>
```

`T` must be `Send + Sync + 'static`. It doesn’t need Serde, because the value never leaves
the pool. Outside a pool this fails with `PoolRequired`.

</details>

#### `.resolve(&handle)` {#task-context-resolve}

Returns `Result<Arc<T>, WorkerError>`.

Turns a handle back into the data, as a read-only `Arc<T>`.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn resolve<T: Send + Sync + 'static>(&self, value: &Shared<T>) -> Result<Arc<T>, WorkerError>
```

It fails if the handle belongs to another pool, has expired, or holds a different type. See
`WorkerError`.

</details>

<details>
<summary>Full declaration</summary>

```rust title=Rust
pub struct TaskContext<P = ()> { /* private fields */ }

impl<P: Message> TaskContext<P> {
    pub fn report(&self, update: P);
    pub fn check_cancelled(&self) -> Result<(), WorkerError>;
    pub fn cancellation_token(&self) -> CancellationToken;
    pub async fn compute<R, F>(&self, work: F) -> Result<R, WorkerError>
    where
        F: FnOnce(ComputeContext<P>) -> R + Send + 'static,
        R: Send + 'static;
    pub fn share<T: Send + Sync + 'static>(&self, value: T) -> Result<Shared<T>, WorkerError>;
    pub fn resolve<T: Send + Sync + 'static>(&self, value: &Shared<T>) -> Result<Arc<T>, WorkerError>;
}
```

</details>

- [Where contexts are declared](/docs/workers/tasks#contexts)
- [Move CPU work to a compute thread](/docs/workers/pools#compute)

## ComputeContext {#compute-context}

**struct** · Worker code

`ComputeContext` is the context for heavy synchronous code. It has the same progress,
cancellation and shared-data tools as `TaskContext`, and you can clone it and hand the
copies to other threads.

**Where you get one**

Add it as the last parameter of a synchronous task, or take it as the argument of the
closure you pass to `TaskContext::compute`.

```rust title=Rust · worker code
#[fusor_worker::task]
pub fn sum(count: u32, ctx: ComputeContext<u32>) -> TaskResult<u64> {
    let mut total = 0;
    for value in 0..count {
        ctx.check_cancelled()?; // stop early if the caller cancelled
        total += u64::from(value);
        ctx.report(value + 1);  // tell the page how far we’ve got
    }
    Ok(total)
}
```

### Type parameters

- **`P`** — The type of progress update this code reports. It defaults to `()`.

### Methods

#### `.report(update)` {#compute-context-report}

Returns `()`.

Sends a progress update to the page, just as on `TaskContext`.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn report(&self, update: P)
```

</details>

#### `.check_cancelled()` {#compute-context-check-cancelled}

Returns `Result<(), WorkerError>`.

Returns `Err(WorkerError::Cancelled)` once the caller has cancelled.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn check_cancelled(&self) -> Result<(), WorkerError>
```

On a pool’s compute threads, cancellation shows up straight away through a flag shared with
the pool. There, checking between chunks really does stop the work early.

</details>

#### `.share(value)` {#compute-context-share}

Returns `Result<Shared<T>, WorkerError>`.

Stores a value in the pool and returns its handle. Pools only.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn share<T: Send + Sync + 'static>(&self, value: T) -> Result<Shared<T>, WorkerError>
```

</details>

#### `.resolve(&handle)` {#compute-context-resolve}

Returns `Result<Arc<T>, WorkerError>`.

Reads a shared value. Pools only.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn resolve<T: Send + Sync + 'static>(&self, value: &Shared<T>) -> Result<Arc<T>, WorkerError>
```

</details>

#### `.clone()` {#compute-context-clone}

Returns `ComputeContext<P>`.

Another context for the same operation, sharing its progress and cancellation. Move it into
threads that do part of the work.

<details>
<summary>Signature</summary>

```rust title=Rust
impl<P: Message> Clone for ComputeContext<P>
```

</details>

<details>
<summary>Full declaration</summary>

```rust title=Rust
pub struct ComputeContext<P = ()> { /* private fields */ }

impl<P: Message> Clone for ComputeContext<P> { /* … */ }

impl<P: Message> ComputeContext<P> {
    pub fn report(&self, update: P);
    pub fn check_cancelled(&self) -> Result<(), WorkerError>;
    pub fn share<T: Send + Sync + 'static>(&self, value: T) -> Result<Shared<T>, WorkerError>;
    pub fn resolve<T: Send + Sync + 'static>(&self, value: &Shared<T>) -> Result<Arc<T>, WorkerError>;
}
```

</details>

> `ComputeContext` has no `cancellation_token()` and no `compute()`. A synchronous service
> method gets a `TaskContext`, not a `ComputeContext`.

- [Letting work notice cancellation](/docs/workers/lifecycle#cooperation)

## CancellationHandle {#cancellation-handle}

**struct** · Page code

A `CancellationHandle` lets page code cancel one operation later, from a Cancel button for
example. It’s small, you can clone it, and it doesn’t keep the operation alive.

**Where you get one**

Call `.cancellation_handle()` on a `Job`, `ResultStream` or `Spawn`. On a `PoolInit`, the
handle cancels pool startup only.

```rust title=Rust · page code
let job = sum::run(&owner, 100);
// Keep a handle where the Cancel button can reach it,
// for example in a Signal<Option<CancellationHandle>>.
cancel.set(Some(job.cancellation_handle()));
let result = job.await; // Err(Cancelled) if the button called cancel()
```

> **Good to know**
>
> Dropping a handle cancels nothing, and holding one doesn’t keep the job alive. Once the
> job is gone, `cancel()` does nothing.

### Methods

#### `.cancel()` {#cancellation-handle-cancel}

Returns `()`.

Cancels the operation. It returns straight away, and the caller’s `.await` finishes with
`WorkerError::Cancelled`.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn cancel(&self)
```

It asks the worker to stop, but doesn’t wait for it to do so. It works before the job has
started, and calling it again is harmless.

</details>

#### `.clone()` {#cancellation-handle-clone}

Returns `CancellationHandle`.

Another handle for the same operation.

<details>
<summary>Signature</summary>

```rust title=Rust
impl Clone for CancellationHandle
```

</details>

<details>
<summary>Full declaration</summary>

```rust title=Rust
pub struct CancellationHandle(/* private field */);

impl Clone for CancellationHandle { /* … */ }

impl CancellationHandle {
    pub fn cancel(&self);
}
```

</details>

> Handles belong to the page: they aren’t `Send`.

- [A cancellation handle in a full example](/docs/workers/tasks#progress)

## spawn {#spawn}

**function** · Page code

`spawn` starts a stateful worker: an instance of your own type that lives on a worker thread
and keeps its state between calls.

It returns a `Spawn`. Await that to run your constructor and get a `Client`, which you call
methods on.

**Where you get one**

`fusor_worker::spawn::<Counter>(&owner, input)`, where `Counter` is your type, its `impl`
block is annotated with `#[fusor_worker::worker]`, and `input` is what its `new` constructor
takes.

```rust title=Rust · page code
let counter = fusor_worker::spawn::<Counter>(&owner, 0).await?; // runs Counter::new(0) on a worker
let value = counter.add(3).await?;                              // runs add(3) there and gives you 3
counter.close().await?;                                         // shuts the worker down
```

> **Note**
>
> Every `spawn` creates a new, independent instance. To share one instance, clone its
> client.

### Type parameters

- **`W`** — Your annotated type, such as `Counter`. Fusor works out the constructor’s input
  type (`W::Input`), error type (`W::InitError`) and progress type (`W::InitProgress`) from
  it.
- **`M`** — `Unbound` until you choose a pool with `.on(&pool)`, as on `Job`.

### Start

#### `spawn::<W>(&owner, input)` {#spawn-<w>}

Returns `Spawn<W>`.

Prepares a new instance, owned by `owner`. Nothing starts until you await.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn spawn<W: Worker>(owner: &OwnerHandle, input: W::Input) -> Spawn<W>
```

</details>

### Configure startup before awaiting

#### `.on(&pool)` {#spawn-on}

Returns `Spawn<W, Bound>`.

Runs the instance on `pool` instead of starting a worker for it. You must choose a pool when
the `impl` is annotated `#[worker(pool)]`.

<details>
<summary>Signature</summary>

```rust title=Rust
impl<W: Worker> Spawn<W, Unbound> {
    pub fn on(self, pool: &Pool) -> Spawn<W, Bound>;
}
```

</details>

#### `.on_progress(callback)` {#spawn-on-progress}

Returns `Spawn`.

Receives progress updates from your constructor, if it reports any.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn on_progress(self, callback: impl FnMut(W::InitProgress) + 'static) -> Self
```

</details>

#### `.cancel_on(&token)` {#spawn-cancel-on}

Returns `Spawn`.

Cancels startup when `token` is cancelled.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn cancel_on(self, token: &CancellationToken) -> Self
```

This setting, like `scope` and `cancellation_handle`, affects startup only. Once running,
the instance belongs to the owner you passed to `spawn`. To make individual method calls
cancellable, configure the jobs they return.

</details>

#### `.scope(&owner)` {#spawn-scope}

Returns `Spawn`.

Cancels startup when a second owner goes away.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn scope(self, owner: &OwnerHandle) -> Self
```

</details>

#### `.cancellation_handle()` {#spawn-cancellation-handle}

Returns `CancellationHandle`.

A handle that cancels startup.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn cancellation_handle(&self) -> CancellationHandle
```

</details>

### Run

#### `.await` {#spawn-await}

Returns `TaskResult<W::Client, W::InitError>`.

Starts a worker (or uses the pool you chose), runs your `new`, and gives you the client.

<details>
<summary>Details</summary>

Startup may have to wait for the worker to load, or for room in the pool, before your
constructor runs.

</details>

<details>
<summary>Full declaration</summary>

```rust title=Rust
pub fn spawn<W: Worker>(owner: &OwnerHandle, input: W::Input) -> Spawn<W>;

pub struct Spawn<W: Worker, M = Unbound> { /* private fields */ }

impl<W: Worker, M> Spawn<W, M> {
    pub fn on_progress(self, callback: impl FnMut(W::InitProgress) + 'static) -> Self;
    pub fn cancel_on(self, token: &CancellationToken) -> Self;
    pub fn scope(self, owner: &OwnerHandle) -> Self;
    pub fn cancellation_handle(&self) -> CancellationHandle;
}

impl<W: Worker> Spawn<W, Unbound> {
    pub fn on(self, pool: &Pool) -> Spawn<W, Bound>;
}

// Awaiting Spawn<W, M> gives TaskResult<W::Client, W::InitError>.
// Generated by the annotation, never implemented by hand:
//   W::Input, W::InitError, W::InitProgress: Message
//   W::Client: Clone + 'static
```

</details>

> `Worker` is the trait that the annotation implements for your type. You never implement it
> yourself. You only name it to write out the client type:
> `<Counter as fusor_worker::Worker>::Client`.

- [Spawn a service and get its client](/docs/workers/services#spawn)
- [Who owns a running service](/docs/workers/lifecycle#ownership)

## Client {#client}

**generated type** · Page code

The client is what you get when you await `spawn`. It has a method for each public method
you wrote, and every call runs on the worker against the same retained state.

**Where you get one**

Awaiting a `Spawn`. Fusor generates the type, so there’s nothing to import. To write it out,
use `<Counter as fusor_worker::Worker>::Client`.

```rust title=Rust · what you write and what you call
// You write, inside #[fusor_worker::worker] impl Counter:
pub fn add(&mut self, amount: u64) -> TaskResult<u64>

// You call, on the client:
counter.add(3)  // Job<u64>: configure it if you like, then await it
counter.close() // Close: await it to shut the worker down
counter.clone() // another client for the same Counter
```

> **Watch out**
>
> Dropping every client isn’t a reliable way to stop a service, because a pool can keep it
> alive. Call `close()`, or let the owner that spawned it go away.

### Your methods

#### `.your_method(input)` {#client-your-method}

Returns `Job<T, E, P, Bound>`.

Has the same name and input as the method you wrote. It returns a job, so you can add
progress or cancellation to each call before you await it.

<details>
<summary>Details</summary>

Your method takes `&mut self`, but the client’s takes `&self`, so any clone can call it. A
`TaskContext` parameter is left out, because Fusor supplies it. `T` and `E` come from your
`TaskResult<T, E>`, and `P` from your `TaskContext<P>`.

Calls to one instance run one at a time, in the order they arrive.

</details>

### Built in

#### `.close()` {#client-close}

Returns `Close`.

Stops accepting calls, lets the calls already accepted finish, then releases the instance.

<details>
<summary>Details</summary>

`close` is reserved, so none of your own methods can use that name. Closing a service that
runs on a pool leaves the pool running.

</details>

#### `.clone()` {#client-clone}

Returns `Client`.

Another client for the same instance. It doesn’t copy the state.

<details>
<summary>Full declaration</summary>

```rust title=Rust
// Schematic: the real type is generated from your impl block.
impl Clone for Client { /* … */ }

impl Client {
    // One method for each public method you wrote, for example:
    pub fn add(&self, amount: u64) -> Job<u64, NoError, (), Bound>;

    pub fn close(&self) -> Close;
}
```

</details>

- [Methods run one at a time](/docs/workers/services#methods)
- [Release the service](/docs/workers/services#close)

## Pool::new {#pool-init}

**function** · Page code

`Pool::new` starts building a pool: a fixed set of threads that share memory, for heavy or
parallel work. It returns a `PoolInit`. Set limits on it if you want to, then await it to
get the running `Pool`.

**Where you get one**

`Pool::new(&owner)`. Pools need a threaded build, and a page served with cross-origin
isolation: see Building and hosting workers.

```rust title=Rust · page code
let pool = Pool::new(&owner)
    .max_threads(2) // two compute threads (the default is up to 4)
    .await?;        // starts every thread, then gives you the Pool
```

> **Note**
>
> A bad limit such as `max_threads(0)` doesn’t fail on the spot. It comes back as
> `InvalidConfiguration` when you await.

### Set limits before awaiting

#### `.max_threads(count)` {#pool-init-max-threads}

Returns `PoolInit`.

How many compute threads to start. The default is 4, or fewer on a device that reports fewer
cores. It must be at least 1.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn max_threads(self, count: usize) -> Self
```

</details>

#### `.max_async_jobs(count)` {#pool-init-max-async-jobs}

Returns `PoolInit`.

How many operations may run at once on the coordinator thread. The default is 4, and it must
be at least 1.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn max_async_jobs(self, count: usize) -> Self
```

This counts every operation that runs on the coordinator: async tasks, stream producers, and
service constructors and methods, even synchronous ones.

</details>

#### `.queue_capacity(count)` {#pool-init-queue-capacity}

Returns `PoolInit`.

How many operations may wait for a free slot before new ones fail with `QueueFull`. The
default is 64, and `0` means nothing waits.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn queue_capacity(self, count: usize) -> Self
```

The same number separately limits how many `compute` calls may wait.

</details>

### Cancel startup

#### `.cancel_on(&token)` {#pool-init-cancel-on}

Returns `PoolInit`.

Stops startup when `token` is cancelled.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn cancel_on(self, token: &CancellationToken) -> Self
```

Neither this nor the handle affects a pool that has already started.

</details>

#### `.cancellation_handle()` {#pool-init-cancellation-handle}

Returns `CancellationHandle`.

A handle that stops startup.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn cancellation_handle(&self) -> CancellationHandle
```

</details>

### Run

#### `.await` {#pool-init-await}

Returns `Result<Pool, WorkerError>`.

Starts the coordinator and every compute thread, and finishes when all of them are ready.

<details>
<summary>Details</summary>

It fails with `Unsupported` when the page can’t use shared memory. Check `capabilities()`
first if you want to explain that to the user.

</details>

<details>
<summary>Full declaration</summary>

```rust title=Rust
impl Pool {
    pub fn new(owner: &OwnerHandle) -> PoolInit;
}

pub struct PoolInit { /* private fields */ }

impl PoolInit {
    pub fn max_threads(self, count: usize) -> Self;
    pub fn max_async_jobs(self, count: usize) -> Self;
    pub fn queue_capacity(self, count: usize) -> Self;
    pub fn cancel_on(self, token: &CancellationToken) -> Self;
    pub fn cancellation_handle(&self) -> CancellationHandle;
}

// Awaiting PoolInit gives Result<Pool, WorkerError>.
```

</details>

- [Create and configure a pool](/docs/workers/pools#create)
- [Prepare a threaded build](/docs/workers/deployment#threads)

## Pool {#pool}

**struct** · Page code

A `Pool` is a running set of threads: one coordinator for async code, and several compute
threads for heavy synchronous work, all sharing one memory. Put jobs, streams and services
on it with `.on(&pool)`.

**Where you get one**

Awaiting `Pool::new(&owner)`.

```rust title=Rust · page code
let total = summarize::run(&owner, values).on(&pool).await?; // runs in this pool
let threads = pool.threads();                                // for example, 2
pool.close().await?;                                         // let work finish, then stop
```

> **Watch out**
>
> Keep the `Pool` value for as long as you use the pool. Dropping it, or its owner going
> away, stops the pool at once, just like `terminate()`.

### Methods

#### `.threads()` {#pool-threads}

Returns `usize`.

How many compute threads started, not counting the coordinator.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn threads(&self) -> usize
```

</details>

#### `.close()` {#pool-close}

Returns `Close`.

A graceful shutdown: refuse new work, let accepted work finish, then stop.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn close(&self) -> Close
```

</details>

#### `.terminate()` {#pool-terminate}

Returns `()`.

Stops the pool immediately. Work that’s still running fails, and the pool’s shared handles
stop working.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn terminate(&self)
```

</details>

<details>
<summary>Full declaration</summary>

```rust title=Rust
pub struct Pool { /* private fields */ }

impl Pool {
    pub fn threads(&self) -> usize;
    pub fn close(&self) -> Close;
    pub fn terminate(&self);
}
```

</details>

> You can’t clone a `Pool`: lend it with `&pool`. Work placed on a pool can’t outlive it.

- [Queues, limits, and pool lifetime](/docs/workers/pools#budget)

## Shared {#shared}

**struct** · Page and worker code

`Shared<T>` is a small handle to a value kept inside a pool. Pass the handle to later calls,
instead of sending a large value every time.

**Where you get one**

Call `ctx.share(value)` in pool code, and return the handle to the page. Read the value back
in pool code with `ctx.resolve(&handle)`.

```rust title=Rust · page code
let data = dataset::run(&owner, 100).on(&pool).await?;         // builds the data inside the pool
let a = summarize::run(&owner, data.clone()).on(&pool).await?; // sends only the handle
let b = summarize::run(&owner, data).on(&pool).await?;         // reuses the same data
```

> **Note**
>
> The page can hold, clone, drop and pass on a handle, but it can’t read the data. Only pool
> code can do that.

> **Watch out**
>
> Pass a handle on its own: as a task argument, a result or a stream item. It can’t go
> inside a struct, tuple or `Vec` that you send, because it has no Serde implementation.

### Type parameters

- **`T`** — The type of the stored value. It must be `Send + Sync + 'static`, and it doesn’t
  need Serde.

### On the page

#### `.clone()` {#shared-clone}

Returns `Shared<T>`.

Another handle to the same value. The data isn’t copied, and `T` doesn’t have to be `Clone`.

<details>
<summary>Signature and details</summary>

```rust title=Rust
impl<T: Send + Sync + 'static> Clone for Shared<T>
```

The value stays in the pool while any handle or resolved `Arc<T>` exists and the pool is
still running.

</details>

### In pool code

#### `ctx.share(value)` {#shared-share}

Returns `Result<Shared<T>, WorkerError>`.

Stores a value and returns its handle.

#### `ctx.resolve(&handle)` {#shared-resolve}

Returns `Result<Arc<T>, WorkerError>`.

Reads the value, as a read-only `Arc<T>`.

<details>
<summary>Details</summary>

To change shared data, put a `Mutex` or an atomic inside `T`, and don’t hold a lock across
an `.await`.

A handle from another pool fails with `WrongPool`, an expired handle with `StaleShared`, and
a handle of the wrong type with `SharedTypeMismatch`.

</details>

<details>
<summary>Full declaration</summary>

```rust title=Rust
pub struct Shared<T: Send + Sync + 'static> { /* private fields */ }

// Implements Clone (without needing T: Clone), Debug and Message.
impl<T: Send + Sync + 'static> Clone for Shared<T> { /* … */ }

// Created and read through a context:
//   ctx.share(value)     -> Result<Shared<T>, WorkerError>
//   ctx.resolve(&handle) -> Result<Arc<T>, WorkerError>
```

</details>

- [Shared data guide](/docs/workers/shared)

## ResultStream {#result-stream}

**struct** · Page code

A `ResultStream` is a background call that sends back many results, a batch at a time,
instead of one big reply at the end. Read it in a loop with `next().await`.

**Where you get one**

Call a stream task’s generated `stream` function, such as `batches::stream(&owner, 100)`.

```rust title=Rust · page code
let mut output = batches::stream(&owner, 100);
while let Some(batch) = output.next().await { // None once the stream has finished
    values.extend(batch?);                    // each item is Ok(batch) or an error
}
```

> **Good to know**
>
> Dropping the stream cancels it, and the producer’s next `send` fails, so it stops
> producing.

### Type parameters

- **`T`** — The type of each item. It’s often a `Vec` of rows, so that each item is a batch.
- **`E`** — The producer’s own error type, or `NoError`.
- **`P`** — The type of progress update, or `()`.
- **`M`** — `Unbound` until you choose a pool, as on `Job`.

### Read results

#### `.next().await` {#result-stream-next}

Returns `Option<TaskResult<T, E>>`.

Gives you the next item: `Some(Ok(item))`, `Some(Err(error))`, or `None` when the stream has
finished. The first call starts the producer.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub async fn next(&mut self) -> Option<TaskResult<T, E>>
```

When the producer finishes, you get every item it had already sent, then `None`. When it
returns an error, you get the items it had sent, then the error once, then `None`.
Cancellation, the owner going away, or a crash discards any pending items and gives you a
single error.

No trait import is needed. The type also implements `futures_core::Stream`, if you prefer
stream combinators.

</details>

### Flow control before reading

#### `.buffer(batches)` {#result-stream-buffer}

Returns `ResultStream`.

How many items may be waiting for you before the producer pauses. The default is 4, and it
must be at least 1.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn buffer(self, batches: usize) -> Self
```

</details>

#### `.max_batch_bytes(bytes)` {#result-stream-max-batch-bytes}

Returns `ResultStream`.

The largest size one item may have once encoded. The default is 1 MiB, and the maximum is 16
MiB.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub fn max_batch_bytes(self, bytes: usize) -> Self
```

An invalid value comes back as `InvalidConfiguration` from the first `next()`.

</details>

### Configure before reading

#### `.on_progress(callback)` {#result-stream-on-progress}

Returns `ResultStream`.

Receives progress updates, as on `Job`.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn on_progress(self, callback: impl FnMut(P) + 'static) -> Self
```

</details>

#### `.cancel_on(&token)` {#result-stream-cancel-on}

Returns `ResultStream`.

Cancels the stream when `token` is cancelled.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn cancel_on(self, token: &CancellationToken) -> Self
```

</details>

#### `.scope(&owner)` {#result-stream-scope}

Returns `ResultStream`.

Also cancels the stream when a second owner goes away.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn scope(self, owner: &OwnerHandle) -> Self
```

</details>

#### `.on(&pool)` {#result-stream-on}

Returns `ResultStream<…, Bound>`.

Runs the producer on `pool`. You must choose a pool for a task annotated
`#[task(pool, stream)]`.

<details>
<summary>Signature</summary>

```rust title=Rust
impl<T: Message, E: Message, P: Message> ResultStream<T, E, P, Unbound> {
    pub fn on(self, pool: &Pool) -> ResultStream<T, E, P, Bound>;
}
```

</details>

#### `.cancellation_handle()` {#result-stream-cancellation-handle}

Returns `CancellationHandle`.

A handle your UI can keep, to cancel the stream later.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn cancellation_handle(&self) -> CancellationHandle
```

</details>

<details>
<summary>Full declaration</summary>

```rust title=Rust
pub struct ResultStream<T, E = NoError, P = (), M = Unbound> { /* private fields */ }

impl<T: Message, E: Message, P: Message, M> ResultStream<T, E, P, M> {
    pub async fn next(&mut self) -> Option<TaskResult<T, E>>;
    pub fn buffer(self, batches: usize) -> Self;
    pub fn max_batch_bytes(self, bytes: usize) -> Self;
    pub fn on_progress(self, callback: impl FnMut(P) + 'static) -> Self;
    pub fn cancel_on(self, token: &CancellationToken) -> Self;
    pub fn scope(self, owner: &OwnerHandle) -> Self;
    pub fn cancellation_handle(&self) -> CancellationHandle;
}

impl<T: Message, E: Message, P: Message> ResultStream<T, E, P, Unbound> {
    pub fn on(self, pool: &Pool) -> ResultStream<T, E, P, Bound>;
}

// Also implements futures_core::Stream<Item = TaskResult<T, E>>.
```

</details>

- [Produce and consume result streams](/docs/workers/streams)
- [Completion, errors, and cancellation](/docs/workers/streams#completion)

## StreamSender {#stream-sender}

**struct** · Worker code

`StreamSender` is how a stream task sends its results. Each `send` delivers one item to the
page’s `ResultStream`.

**Where you get one**

Add it as the last parameter of an async function annotated `#[fusor_worker::task(stream)]`.
Fusor provides it: the page never passes it.

```rust title=Rust · worker code
#[fusor_worker::task(stream)]
pub async fn batches(count: u32, mut output: StreamSender<Vec<u32>>) -> TaskResult<()> {
    let mut batch = Vec::with_capacity(32);
    for value in 0..count {
        batch.push(value);
        if batch.len() == 32 {
            output.send(std::mem::take(&mut batch)).await?; // waits if the page is behind
        }
    }
    if !batch.is_empty() {
        output.send(batch).await?;
    }
    Ok(())
}
```

> **Good to know**
>
> To send many small results efficiently, collect them into a `Vec` and send that as one
> item.

### Type parameters

- **`T`** — The item type. It must be a `Message`.

### Methods

#### `.send(item).await` {#stream-sender-send}

Returns `Result<(), WorkerError>`.

Sends one item, waiting first if the page’s buffer is full. `Ok` means the item was
accepted, not that the page has read it yet.

<details>
<summary>Signature and details</summary>

```rust title=Rust
pub async fn send(&mut self, item: T) -> Result<(), WorkerError>
```

It fails with `Cancelled` when the page has cancelled or dropped the stream: pass that on
with `?` to stop producing. It also fails when the item can’t be encoded, is too large, or
holds an invalid shared handle.

</details>

<details>
<summary>Full declaration</summary>

```rust title=Rust
pub struct StreamSender<T> { /* private fields */ }

impl<T: Message> StreamSender<T> {
    pub async fn send(&mut self, item: T) -> Result<(), WorkerError>;
}
```

</details>

> A stream task returns `TaskResult<()>`, because its results go through the sender rather
> than the return value. An optional `TaskContext` parameter goes just before the sender.

- [Define an async producer](/docs/workers/streams#producer)

## Close {#close}

**struct** · Page code

`Close` is a graceful shutdown for a pool or a stateful worker. When you await it, new calls
are refused, calls already accepted are allowed to finish, and then it completes.

**Where you get one**

`pool.close()` or `client.close()`.

```rust title=Rust · page code
pool.close()
    .timeout(Duration::from_secs(10)) // wait up to 10 seconds (the default is 5)
    .await?;
```

> **Good to know**
>
> Closing a service that runs on a pool leaves the pool running. To stop a pool immediately,
> call `pool.terminate()`.

### Methods

#### `.timeout(duration)` {#close-timeout}

Returns `Close`.

How long to wait for accepted work to finish. The default is five seconds. Set it before you
await.

<details>
<summary>Signature</summary>

```rust title=Rust
pub fn timeout(self, duration: std::time::Duration) -> Self
```

</details>

#### `.await` {#close-await}

Returns `Result<(), WorkerError>`.

Finishes once accepted work is done, or with `CloseTimedOut` if the deadline passes first.

<details>
<summary>Details</summary>

Calling close again joins the same shutdown, and dropping a started `Close` doesn’t reopen
anything. If a service method on a pool is still running at the deadline, callers stop
waiting, but the service’s state is kept until that method returns.

</details>

<details>
<summary>Full declaration</summary>

```rust title=Rust
pub struct Close { /* private fields */ }

impl Close {
    pub fn timeout(self, duration: std::time::Duration) -> Self;
}

// Awaiting Close gives Result<(), WorkerError>.
```

</details>

- [Closing services and pools](/docs/workers/lifecycle#close)

## TaskResult {#task-result}

**type alias** · Page and worker code

`TaskResult<T, E>` is the return type of every task, service method and constructor. It’s
also what you get when you await a job.

It’s an ordinary Rust `Result`: either your value, or a `JobError` that says whether your
code or Fusor failed.

**Where you get one**

You write it as the return type of your tasks and service methods.

```rust title=Rust · worker code
#[fusor_worker::task]
pub fn total(values: Vec<u64>) -> TaskResult<u64> {           // has no error of its own
    Ok(values.into_iter().sum())
}

#[fusor_worker::task]
pub fn parse_count(text: String) -> TaskResult<u32, String> { // fails with a String
    text.parse::<u32>()
        .map_err(|error| JobError::Application(error.to_string()))
}
```

> **Note**
>
> `NoError` is an enum with no values, so it can never happen. A `TaskResult<T>` can still
> fail with a `WorkerError`, for example when the job is cancelled.

### Type parameters

- **`T`** — The value on success.
- **`E`** — Your own error type. Leave it out, as in `TaskResult<u64>`, when your code has
  no error of its own. It then defaults to `NoError`.

<details>
<summary>Full declaration</summary>

```rust title=Rust
pub type TaskResult<T, E = NoError> = Result<T, JobError<E>>;

// An error that can never happen.
// Implements Debug, Display, Error and Serde’s traits.
pub enum NoError {}
```

</details>

- [Application errors and runtime errors](/docs/workers/lifecycle#errors)

## JobError {#job-error}

**enum** · Page and worker code

`JobError` is the error side of a `TaskResult`. It tells you whose failure it was: your
code’s, or Fusor’s.

**Where you get one**

The `Err` side of every `TaskResult`.

```rust title=Rust · page code
match parse_count::run(&owner, text).await {
    Ok(count) => show_count(count),
    Err(JobError::Application(message)) => show_error(message),    // your task’s own error
    Err(JobError::Worker(error)) => show_error(error.to_string()), // from Fusor
    Err(_) => {}                                                   // required: more variants may come
}
```

> **Watch out**
>
> `JobError` is `#[non_exhaustive]`, so a `match` on it needs a final `_` arm.

### Type parameters

- **`E`** — Your task’s own error type, or `NoError`.

### Variants

#### `Application(E)` {#job-error-application}

An error your own code returned, of your type `E`.

<details>
<summary>Details</summary>

Create one explicitly, usually inside `map_err`: `JobError::Application(error)`. There’s no
automatic conversion from `E`, because it would conflict with the one below.

</details>

#### `Worker(WorkerError)` {#job-error-worker}

A failure from Fusor: the job was cancelled, the worker crashed, a value was too large, and
so on.

<details>
<summary>Details</summary>

`?` turns a `WorkerError` into this variant for you.

</details>

<details>
<summary>Full declaration</summary>

```rust title=Rust
#[non_exhaustive]
pub enum JobError<E> {
    Application(E),
    Worker(WorkerError),
}

impl<E> From<WorkerError> for JobError<E> { /* … */ }
```

</details>

> `E` must be a `Message`, but it doesn’t have to implement `std::error::Error`: `String`
> works. `JobError<E>` implements `Display` when `E` does and `Error` when
> `E: Error + 'static`, and it always implements `Debug` and Serde’s traits.

- [Application errors and runtime errors](/docs/workers/lifecycle#errors)

## WorkerError {#worker-error}

**enum** · Page and worker code

`WorkerError` covers every failure that comes from Fusor rather than from your code. The
groups below say what happened and what you can usually do about it.

**Where you get one**

Inside `JobError::Worker`, and directly from methods such as `check_cancelled`, `compute`
and `send`.

```rust title=Rust · page code
match summarize::run(&owner, values).on(&pool).await {
    Ok(total) => show(total.to_string()),
    Err(JobError::Worker(WorkerError::Cancelled)) => {} // the user cancelled: nothing to show
    Err(JobError::Worker(WorkerError::QueueFull { .. })) => show("Busy, try again".into()),
    Err(error) => show(error.to_string()),
}
```

> **Watch out**
>
> `WorkerError` is `#[non_exhaustive]`, so a `match` on it needs a final `_` arm.

> **Good to know**
>
> Fusor never retries a failed call for you, because it may already have changed something.
> Whether to retry is your decision.

### Cancelled or gone

#### `Cancelled` {#worker-error-cancelled}

The caller cancelled, through a token, a `CancellationHandle`, or by dropping the job.
There’s usually nothing to show the user.

<details>
<summary>Details</summary>

The worker may still have been running when this was reported.

</details>

#### `OwnerDisposed` {#worker-error-ownerdisposed}

The owner, or an extra scope, went away: for example, because the component was removed.

### Shut down

#### `Closed` {#worker-error-closed}

The pool or service was closed before this call was accepted.

#### `Terminated` {#worker-error-terminated}

The pool was terminated or dropped.

#### `CloseTimedOut` {#worker-error-closetimedout}

A graceful close reached its deadline while this call was still running.

### Setup and placement

#### `PoolRequired` {#worker-error-poolrequired}

This work needs a pool. Add `.on(&pool)`.

#### `Unsupported { capability }` {#worker-error-unsupported { capability }}

The browser lacks something the operation needs, such as shared memory. `capability` says
which.

<details>
<summary>Details</summary>

For pools, check that the page is served over HTTPS (or from localhost) with the
cross-origin isolation headers.

</details>

#### `IncompatibleArtifact` {#worker-error-incompatibleartifact}

The worker files and the page come from different builds. Deploy one complete build.

### Shared data

#### `WrongPool` {#worker-error-wrongpool}

The handle belongs to a different pool.

#### `StaleShared` {#worker-error-staleshared}

The handle no longer points at live data, for example because its pool was closed or
restarted.

#### `SharedTypeMismatch` {#worker-error-sharedtypemismatch}

The handle holds a different type from the one the code expected.

### Limits and settings

#### `QueueFull { capacity }` {#worker-error-queuefull { capacity }}

Too many operations are already waiting. Try again later, or raise `queue_capacity`.

#### `PayloadTooLarge { limit, actual }` {#worker-error-payloadtoolarge { limit, actual }}

A value was bigger than a size limit. Both numbers are in bytes.

#### `InvalidConfiguration { message }` {#worker-error-invalidconfiguration { message }}

A builder setting was invalid, or was changed after the operation had started.

### Runtime failures

#### `Encode { message }` {#worker-error-encode { message }}

A value couldn’t be turned into a message.

#### `Decode { message }` {#worker-error-decode { message }}

A message couldn’t be turned back into a value.

#### `Load { message }` {#worker-error-load { message }}

The worker’s files couldn’t be loaded. Check the browser’s network tab and your content
security policy.

#### `Crashed { message }` {#worker-error-crashed { message }}

Worker code panicked or trapped. Its state is gone, so create a new pool or service.

<details>
<summary>Full declaration</summary>

```rust title=Rust
#[non_exhaustive]
pub enum WorkerError {
    Cancelled,
    OwnerDisposed,
    Closed,
    Terminated,
    CloseTimedOut,
    Unsupported { capability: Capability },
    PoolRequired,
    IncompatibleArtifact,
    WrongPool,
    StaleShared,
    SharedTypeMismatch,
    QueueFull { capacity: usize },
    PayloadTooLarge { limit: usize, actual: usize },
    InvalidConfiguration { message: String },
    Encode { message: String },
    Decode { message: String },
    Load { message: String },
    Crashed { message: String },
}
```

</details>

> `WorkerError` implements `Clone`, `Debug`, `PartialEq`, `Eq`, `Display` and `Error`.

- [What each WorkerError means](/docs/workers/lifecycle#runtime-errors)
- [Diagnose startup failures](/docs/workers/deployment#troubleshooting)

## Message {#message}

**trait** · Page and worker code

`Message` is the rule for anything that travels between the page and a worker: arguments,
results, errors, progress updates and stream items.

You never implement it yourself. Fusor implements it for every type that qualifies.

**Where you get one**

Automatic, for owned types that implement Serde’s `Serialize` and `Deserialize` and are
`Send + 'static`.

```rust title=Rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Report {   // a Message: owned data that Serde can encode
    pub title: String,
    pub rows: Vec<u32>,
}
```

> **Note**
>
> Deriving the Serde traits needs `serde = { version = "1", features = ["derive"] }` in your
> `Cargo.toml`.

> **Watch out**
>
> `Shared<T>` is a `Message` without Serde, so it can’t be nested inside a type that you
> derive Serde for.

- **Can send** — Owned data: numbers, `bool`, `String`, `Vec<T>`, `Option<T>`, your own
  structs and enums that derive `Serialize` and `Deserialize`, and `Shared<T>` handles.
- **Can’t send** — Borrowed data such as `&str`, plus signals, owners, DOM elements,
  callbacks and closures. Those belong to the page.
- **Size limit** — Each value, and each whole request or reply, can be at most 16 MiB once
  encoded.

<details>
<summary>Full declaration</summary>

```rust title=Rust
// Sealed: implemented by Fusor, never by hand.
pub trait Message: Send + 'static {}

impl<T: Serialize + DeserializeOwned + Send + 'static> Message for T {}
// Shared<T> is also a Message, without Serde.
```

</details>

- [Use owned message types](/docs/workers/tasks#messages)

## capabilities {#capabilities}

**function** · Page code

`capabilities()` tells you what the current browser supports, so you can explain a missing
feature before you try to start a pool.

**Where you get one**

`fusor_worker::capabilities()`. It takes no arguments and returns straight away.

```rust title=Rust · page code
let support = fusor_worker::capabilities();
if !support.shared_memory {
    // Explain why pools are unavailable, or fall back to an ordinary task.
}
```

> **Watch out**
>
> `true` doesn’t guarantee that startup will work: loading, memory or your content security
> policy can still get in the way. Always handle the result of `Pool::new` too.

### Fields of Capabilities

#### `dedicated_workers` {#capabilities-dedicated-workers}

Returns `bool`.

Ordinary workers are available.

#### `shared_memory` {#capabilities-shared-memory}

Returns `bool`.

Pools can run: workers are available, the page is cross-origin isolated, and shared memory
exists.

#### `hardware_parallelism` {#capabilities-hardware-parallelism}

Returns `usize`.

How many threads the device suggests. It’s a hint, not a promise that those cores are free.

### Related

#### `Capability` {#capabilities-capability}

An enum that names one feature, `DedicatedWorkers` or `SharedMemory`. It’s what
`WorkerError::Unsupported` reports as missing.

<details>
<summary>Full declaration</summary>

```rust title=Rust
pub fn capabilities() -> Capabilities;

pub struct Capabilities {
    pub dedicated_workers: bool,
    pub shared_memory: bool,
    pub hardware_parallelism: usize,
}

// Implements Clone, Copy, Debug, PartialEq, Eq and Serde’s traits.
pub enum Capability {
    DedicatedWorkers,
    SharedMemory,
}
```

</details>

> On native (non-browser) targets it reports `false`, `false` and `1`, and background jobs
> never quietly run locally instead.

- [Check browser support](/docs/workers/deployment#capabilities)
