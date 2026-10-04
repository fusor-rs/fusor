# Tasks

Annotate a function, call it from the page with owned arguments, and await the result while
the page stays responsive.

## Add the package to your app {#setup}

Add `fusor-worker` to the dependencies of an existing Fusor app. Keep the `<App>` entry
point and the usual `fusor_build::compile_app()` build script, and put annotated functions
in ordinary Rust modules that your app includes.

Then build as usual with `fusor dev` or `fusor build`. The CLI finds the annotated functions
in the compiled code and generates the worker assets. You don’t write a worker entry point,
register functions, supply worker URLs, or add a Cargo target.

Use the same release for `fusor-worker` and any other optional Fusor packages as for your
app’s core and CLI.

```toml title=TOML · additions to Cargo.toml
[dependencies]
fusor-worker = "=0.1.5"
```

- [Create an application first](/docs/installation)
- [How automatic packaging works](/docs/workers/deployment#packaging)

## Define a task and call it {#task}

`#[fusor_worker::task]` leaves your function usable as an ordinary function and also
generates a companion `name::run(&owner, args...)` for calling it from the UI. Calling the
original function directly runs it on the current thread. Awaiting the generated call runs
it in a worker.

The arguments you give `run` are the ones you wrote in the function signature, minus any
context or sender parameter that Fusor injects. Awaiting the call resolves to the function’s
result. A task must be a nongeneric, safe function returning `TaskResult<T, E>`, where `T`
is the success value and `E` is the application error type. Omit `E` when the task has no
application error of its own. The example defines a task and an async helper that calls it;
the helper takes the `OwnerHandle` of the component it belongs to.

```rust source=tutorial/lessons/workers/tasks.rs title=Rust · tasks.rs
```

- [Find your component’s owner](/docs/ownership/mounting#owner-variable)
- [Handle application and runtime errors](/docs/workers/lifecycle#errors)

## Start a job with .await {#run}

Jobs start when they are first polled. Using `.await` on the `Job` returned by `run` polls
it, submits the work to a worker, and waits for the result without blocking the UI thread.

Without a pool, all of an owner’s tasks share one dedicated worker and run in first-in,
first-out order, async tasks included. A slow task therefore delays the tasks queued behind
it. This includes an async task that is only waiting on I/O: it occupies the dedicated
worker’s single operation slot until it completes, so later tasks wait for it. `.on(&pool)`
returns a job bound to a pool and can be called once. Choose a pool when you want a fixed
budget of compute threads or want independent computations to overlap.

- [Choose where work executes](/docs/workers/pools#placement)
- [Cancel a job](/docs/workers/lifecycle#cancellation)
- [Job methods and signatures](/docs/workers/api#job)

## Contexts that Fusor supplies {#contexts}

A task can ask Fusor for a context object by declaring it as its final parameter.
Synchronous tasks take a `ComputeContext<P>` and async tasks take a `TaskContext<P>`. `P` is
the type of progress update the task reports and defaults to `()`. Fusor supplies this
argument itself, so the generated `run` call leaves it out.

Both contexts have `report(update)`, which sends a progress update and returns `()`, and
`check_cancelled()`, which returns `Result<(), WorkerError>`. It returns
`Err(WorkerError::Cancelled)` once cancellation has reached that context. A caller’s
cancellation doesn’t reach a busy ordinary worker until the worker processes the
cancellation message, so the check can succeed for a while after the caller has cancelled.
Both also have `share(value)` and `resolve(&handle)` for shared data, which requires a pool.
Only `TaskContext` has `cancellation_token()` and the async `compute(work)`, which moves CPU
work onto a pool’s compute threads and also requires a pool. A `ComputeContext` can be
cloned and moved into CPU work, while a `TaskContext` stays in the worker’s async context.

```rust title=Rust · signature shapes; Input, Output, and Progress are your types
// Authored synchronous task:
fn calculate(input: Input, ctx: ComputeContext<Progress>) -> TaskResult<Output>
// Authored asynchronous task:
async fn load(input: Input, ctx: TaskContext<Progress>) -> TaskResult<Output>

// Generated UI calls omit ctx:
calculate::run(&owner, input)
load::run(&owner, input)
```

- [Move CPU work out of an async operation](/docs/workers/pools#compute)
- [TaskContext methods](/docs/workers/api#task-context)
- [ComputeContext methods](/docs/workers/api#compute-context)

## Report progress to a UI callback {#progress}

Choose an owned type for progress updates, call `ctx.report(update)` from inside the task,
and install a UI callback with `on_progress` on the job. Progress is coalesced: if a newer
update arrives before an older one has been delivered, the newer one replaces it. Treat
progress as a status display rather than a complete log of events.

A job also hands out a `CancellationHandle` through `cancellation_handle()`, which the page
can keep so that something like a Cancel button can call `cancel()`. Holding the handle
doesn’t keep the job alive. Progress display and cancellation controls belong to the UI, so
they stay on the page rather than being sent to the worker. Cancellation reaches the task
cooperatively. A busy synchronous task on an ordinary worker can’t process the cancellation
message until it returns, while a computation on a pool can see cancellation through a flag
it shares with the pool.

```rust source=tutorial/lessons/workers/progress.rs title=Rust · progress.rs
```

- [Cancellation and yielding](/docs/workers/lifecycle#cancellation)
- [Play an engine that reports its progress](/docs/showcase/game)

## Make network requests from async tasks {#fetch}

An async task can await Fetch and other browser APIs that are available in workers. Passing
`TaskContext::cancellation_token()` to a request ties it to the job’s cancellation. The
example uses the `fusor_async::fetch::get_text` helper and converts its failure into a
serializable application error. Relative URLs given to this helper resolve against the app’s
base URL, including when the app is deployed under a subpath.

The helper requires `fusor-async = { version = "=0.1.5", features = ["browser"] }` in your
dependencies. Being async doesn’t move synchronous code off the worker’s event loop: code
between awaits still runs there. In a task placed on a pool, hand substantial CPU work to
`ctx.compute`.

```rust source=tutorial/lessons/workers/fetch.rs title=Rust · fetch.rs
```

- [Fetch requests and cancellation](/docs/async-data/resource#request)
- [Combine async work and CPU work](/docs/workers/pools#compute)

## Use owned message types {#messages}

Task inputs, outputs, application errors, and progress updates are all messages. An ordinary
type qualifies when it satisfies Serde’s serialization and deserialization requirements and
is `Send + 'static`. Fusor then implements its `Message` trait for it automatically, and you
can’t implement the trait by hand. `Shared<T>` is a supported exception: it is a message
even though it has no Serde implementation. Use owned types such as `String`, `Vec<T>`, or
your own structs deriving Serde’s traits, which requires
`serde = { version = "1", features = ["derive"] }`.

Each argument is encoded separately as JSON. Every encoded argument or value must be at most
16 MiB, and so must the complete encoded request or reply. Several large arguments, or the
small overhead of encoding, can therefore exceed the overall limit even when each argument
fits on its own. `PayloadTooLarge` reports which limit was exceeded. Borrowed data, owners,
signals, callbacks, DOM objects, and closures can’t be sent. For data that should stay
inside a pool instead of being sent repeatedly, use `Shared<T>`.

- [Avoid repeatedly encoding a pool dataset](/docs/workers/shared)
- [Choose an application error type](/docs/workers/lifecycle#errors)

## Connect a task to a resource {#ui}

A `Resource` is the usual way to feed a worker result into the UI. It loads data
asynchronously for a component and exposes loading, ready, and error states that the
template can render. To connect a task, keep the `Resource` in the component, whose owner
you pass to the generated call. Have the resource’s loader return the job, and pass the
resource’s cancellation token to the job’s `cancel_on`. When the resource’s key changes or
the component is disposed, the old request is invalidated and cancelled, so a stale result
is never published. Because the worker does the slow part, the page stays responsive while
the resource is loading.

To run the example, execute `cargo fusor dev -p fusor-workers --port 8091` from the
repository root and open `http://127.0.0.1:8091/`. The Rust and HTML shown here are the
example’s actual source files.

```rust source=../../examples/workers/src/lib.rs title=Rust · examples/workers/src/lib.rs
```

- [Understand Resource loading state](/docs/async-data)
- [See the matching HTML](/docs/workers/tasks#template)

## Render the worker result {#template}

The template renders the resource’s state, so it needs a place to show each of loading,
ready, and error, and a control that changes the key when a new request should start. The
`<App>` element creates the component and passes it its owner, which is the owner the
generated call is tied to. Any UI that doesn’t depend on the worker stays interactive while
the job runs, because the worker does the slow work on another thread.

```html source=../../examples/workers/web/index.html title=HTML · examples/workers/web/index.html
```
