# Coherent async behavior

The detailed contract behind “keep the old view until the next one is complete.” Read this
when choosing loading, error, and interaction behavior.

## AsyncBoundary::coherent() {#boundary}

Creates an optional handle for `<Async boundary="{{ state.view }}">`. Use it for status and
retry controls; plain `<Async>` creates one automatically.

`browser::read` declares each `AsyncValue` and `<Await>` consumes its successful result. The
parent does not have to gather its children’s futures.

- [browser::read arguments and return type](/docs/async-data/resource#read)

## status() and retry() {#status}

`boundary.status()` reactively reports a `BoundaryStatus`. Before a region attaches, it is
`Detached`. While a candidate view loads, it is `Pending`; a complete published view is
`Ready`.

`Error(fusor::coherence::Error)` carries an evaluation, validation, or loader failure.
`Faulted` carries the same error type when applying a prepared view fails. `error.kind()`
distinguishes contract, read, and renderer failures; `error.downcast_ref::<E>()` retrieves
a retained loader error of type `E`. Display formats its diagnostic. `Disposed` means the
boundary’s lifetime ended.
DOM failures retain `fusor::dom::coherent::DomError`, whose `.0` holds the browser exception.

Keep status and retry controls outside the region so they remain usable while that region
blocks interactions.

> `retry()` retries failed reads while keeping compatible successful results. A new
> selection supersedes the previous attempt. Faulted is distinct from a loader error;
> inspect the reported diagnostic and correct the failed setup.

## Failure and interaction states in detail {#failure}

On the first load, unresolved bindings show their authored placeholders until a complete
view is ready. On later loads, the last complete view remains.

Pending, failed, and faulted regions block stale interactions. A failed read sets the
boundary to Error; the external Retry button retries failed reads while keeping compatible
successes.

New selection inputs supersede the previous attempt.

> For native HTML that should remain useful before first activation, islands can use an
> explicit preview. Arbitrary server HTML is not automatically treated as fulfilled async
> data.

- [Native content during activation](/docs/islands#native)

## Use the first version within its boundaries {#constraints}

Use coherent regions for generated text, attributes, classes, events, and owned children or
`<ForEach>` lists.

Constructors and render calculations must stay pure apart from declared reads; ordinary
effects in prepared descendants wait for publication.

Keep editable controls outside the region. Nested boundaries, router outlets, foreign widget
DOM, custom elements, and regions spanning independent islands are unsupported.

> This is a display-publication guarantee, not a database transaction or general rollback
> mechanism. Your loader, cache, authentication, and server remain application choices.
