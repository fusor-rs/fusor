# Result streams

Deliver a sequence of results as they are produced, through a bounded buffer, instead of one
large reply.

## Define an async producer {#producer}

A result stream comes from an async function annotated with `#[fusor_worker::task(stream)]`
that returns `TaskResult<(), E>`. Its last parameter must be a `StreamSender<T>`, and an
optional `TaskContext<P>` goes directly before it. Fusor supplies both, so the page passes
only the ordinary arguments.

The generated entry point is `name::stream` rather than `name::run`. Each `send(item).await`
submits one item of type `T` and returns `Result<(), WorkerError>`. It waits while the
buffer has no capacity, and it fails if the stream has been cancelled, which includes the
consumer dropping the stream. It can also fail because the item can’t be encoded, is too
large, or contains an invalid shared handle. A successful `send` means the item was accepted
for delivery, not that the consumer has already received it. Making `T` a `Vec` of some row
type makes each item a batch of rows.

```rust source=tutorial/lessons/workers/streams.rs title=Rust · streams.rs
```

## Consume with next {#consume}

`ResultStream<T, E, P, M>` has an async `next(&mut self)` method. `T` is the item type, `E`
is the application error type, and `P` is the progress update type. `M` is the same
placement marker as on `Job`: a stream starts as `Unbound`, which lets you select a pool,
and `.on(&pool)` changes it to `Bound`. An `Unbound` stream can still run with default
placement. Awaiting `next()` gives an `Option<TaskResult<T, E>>`: `Some(Ok(value))` is an
item, `Some(Err(error))` is a failure, and `None` means the stream has finished. `next` is
an inherent method, so no extension-trait import is needed, and the stream also implements
`futures_core::Stream`.

Like any job, a stream starts on its first poll. Before consuming it, configure
`on_progress`, `cancel_on`, `scope`, and optionally `.on(&pool)`. `#[task(pool, stream)]`
makes the pool selection mandatory. Producers on a pool run on its coordinator, so a pooled
producer should use `ctx.compute` for long CPU work.

- [ResultStream reference](/docs/workers/api#result-stream)
- [StreamSender reference](/docs/workers/api#stream-sender)

## Limit outstanding batches {#buffer}

By default a stream allows four outstanding batches, each up to 1 MiB once encoded.
`.buffer(n)` changes the batch count and requires a nonzero value. `.max_batch_bytes(n)`
accepts values from 1 byte to 16 MiB. The 16 MiB limit also applies to the complete encoded
message that carries a batch, so a batch close to the maximum can still be rejected with
`PayloadTooLarge`. Invalid values produce `InvalidConfiguration`.

A batch counts against the capacity from the moment the producer’s `send` accepts it until
the consumer’s `next` yields it. When the buffer is full, `send` waits, and it continues
once `next` frees space. This bounds what is in transit, not the consumer’s total memory: a
consumer that keeps every item holds all of them. With large data, process and drop each
batch as it arrives.

## Completion, errors, and cancellation {#completion}

When the producer finishes successfully, the consumer first receives every batch that was
already accepted, then `None`. If the producer returns an error, the accepted batches are
still delivered, then the error appears once, followed by `None`. Local cancellation, owner
disposal, or a runtime failure discards pending output and yields a single error instead.

Dropping the stream cancels it and wakes any `send` that is waiting. Use progress for status
updates that can be replaced, and stream items for data whose order matters.

- [Cancellation and cleanup](/docs/workers/lifecycle#cancellation)
