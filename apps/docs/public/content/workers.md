# Background tasks and workers

Workers run your Rust code on separate browser threads, so heavy computation, long-lived
state, and parallel work don’t compete with the UI for time.

## The types at a glance {#map}

You mostly work with a few types. A `Job` is one operation’s pending result, and a
`ResultStream` is a sequence of results. A `Pool` is a reusable set of threads that jobs can
be placed on. A generated client is your stateful service as the page sees it. `Shared<T>`
is a handle to data kept inside a pool.

Inside worker code, Fusor hands you a context (`TaskContext` or `ComputeContext`) for
progress, cancellation, and shared data, and a `StreamSender` for producing stream items.
The page never passes these.

The API reference lists every type with its methods and signatures. The guides below explain
when to use each.

- [Worker API reference](/docs/workers/api)
- [Which type is which](/docs/workers/api#types)

## Run work off the UI thread {#tasks}

A browser page runs rendering, event handling, and your UI code on a single thread. Anything
slow on that thread delays clicks and repaints. A worker is another browser thread that runs
Rust compiled to WebAssembly (Wasm), so it can do the slow work while the page stays
responsive. Workers run on the user’s device, not on your backend, and Fusor packages the
worker’s Wasm and JavaScript together with your application.

The simplest way to use a worker is to annotate a Rust function with
`#[fusor_worker::task]`. Fusor generates a `run` function that sends the arguments to a
worker and returns a future for the result. Tasks suit any computation that would otherwise
hold up the page, such as parsing, filtering, or image processing.

- [Write and call your first task](/docs/workers/tasks)

## Data crosses threads as messages {#messages}

Arguments and results travel between the page and a worker as messages: owned values that
Serde encodes on one side and decodes on the other. Things that only make sense on the page,
such as signals, owners, DOM elements, and UI callbacks, stay on the page. A worker returns
data, and page code applies it to the UI.

Workers can make network requests, but they have no direct access to the page’s DOM.

- [Choose message types](/docs/workers/tasks#messages)
- [Make a request from a worker](/docs/workers/tasks#fetch)

## Tie work to a component {#lifetime}

Every task and service is started with an `OwnerHandle`, the handle Fusor uses to track the
lifetime of a component or view. When that owner is disposed, the work it started is
cancelled. A running job can also report progress through callbacks, be cancelled with a
token, and be tied to additional owners through scopes.

Cancelling stops the caller from waiting for a result. It does not undo anything the worker
has already done.

- [Understand ownership, cancellation, and errors](/docs/workers/lifecycle)
- [Where does owner come from?](/docs/ownership/mounting#owner-variable)

## Keep state between calls {#services}

The task API models independent function calls: each call starts from its arguments. When
you want a Rust value to stay alive in a worker, because building it again for every call
would be wasteful, annotate an `impl` block with `#[fusor_worker::worker]` and start it with
`spawn`. The page receives a typed client whose methods all operate on the same retained
value. Calls to one service run one at a time.

- [Create a stateful worker](/docs/workers/services)
- [See one search a million books](/docs/showcase/search)

## Use several compute threads {#pools}

A `Pool` is a fixed-size set of worker threads that share one Wasm memory. Select a pool
with `.on(&pool)` to run synchronous tasks on those threads. Async worker code can hand
CPU-heavy closures to the pool with `ctx.compute`. More threads let independent computations
overlap, but a single sequential computation doesn’t become parallel by itself.

- [Choose a pool and place work](/docs/workers/pools)
- [Watch a pool paint a fractal](/docs/showcase/fractal)

## Reuse a large value {#shared}

`Shared<T>` keeps a value inside one pool and gives the page a small handle to it. The page
can hold the handle and pass it to later operations on that pool. Worker code turns the
handle into an `Arc<T>` when it needs the data, so the value doesn’t have to be encoded and
copied on every call.

- [Create and reuse shared data](/docs/workers/shared)

## Return results in batches {#streams}

A `ResultStream` delivers results as the worker produces them, without waiting for the whole
operation to finish. It holds a limited number of undelivered items, and a producer that
gets ahead of its consumer waits. Streams suit output that arrives in pieces, such as
progressively parsed data, search results, or chunked output.

- [Produce and consume result streams](/docs/workers/streams)

## Finish work deliberately {#shutdown}

Await `close()` on a service client or a pool to stop it accepting new calls and let the
work it has already accepted finish. To stop an entire pool immediately, call
`Pool::terminate()`.

Failures are reported to the caller. Fusor never retries an operation on its own, because an
operation that failed may already have changed state.

- [Close, terminate, and recover](/docs/workers/lifecycle#close)

## Build and deploy with the app {#hosting}

In an application built around `<App>`, the annotations are all that ordinary workers need
to be packaged. Pools additionally need a threaded build toolchain and a page served with
cross-origin isolation, a browser security mode that enables shared memory. The deployment
guide covers what Fusor sets up during development and which response headers your
production host has to send.

- [Build and host workers](/docs/workers/deployment)
