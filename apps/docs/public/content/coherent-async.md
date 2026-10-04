# Coherent async views

Use `<Await>` to display an async result. Add `<Async>` when several results must change
together.

## See why a boundary helps {#run}

The companion’s “Two reads, one complete view” section loads price and stock separately for
product A.

Choose B: the old A view stays visible while either B read is pending, then the product
name, price, and stock change together. Use browser network throttling to observe the
transition.

This guide builds on components, owners, and the async crate from Async data loading.

```sh title=Terminal
# From the fusor repository root, after Installation:
fusor dev --manifest-path apps/docs/tutorial/Cargo.toml --port 8091
```

- [Async dependency and request basics](/docs/async-data#dependency)
- [Try the side-by-side comparison first](/docs/showcase/comparison)
- [Detailed coherent async behavior](/docs/coherent-async/semantics)

## Keep the selection in Rust {#boundary-state}

The parent needs the selected product. `Price` and `Stock` each declare their own request.
`<Async>` will create the coordinator automatically, so this basic version needs no boundary
field.

```rust title=src/app.rs · state excerpt
use fusor::{signal, Signal};

pub struct App {
    product: Signal<String>,
}
impl App {
    fn new() -> Self {
        Self { product: signal("A".into()) }
    }
}
```

> The complete companion also keeps an explicit boundary handle for its status and retry
> controls, shown below.

- [Complete App with the boundary fields](/docs/source/tutorial-app.rs.txt)

## Let a view update independently {#independent}

The `Price` and `Stock` components below each use `<Await>` in their template. Render them
outside `<Async>` and each updates as soon as its own read succeeds.

They use the same Rust read declaration in both modes; the surrounding HTML chooses whether
to coordinate them.

```html title=HTML · independent views
<section>
  <Price product="{{ state.product.clone() }}"></Price>
  <Stock product="{{ state.product.clone() }}"></Stock>
</section>
```

> For a direct read on the current state, write
> `<Await value="{{ state.quote }}" let="price"><p>{{ price.as_str() }}</p></Await>`.
> `Price` and `Stock` encapsulate that same pattern.

## Choose which elements change together {#boundary}

Wrap the product label, price, and stock in `<Async>`. They will switch from A to B
together.

`<Async>` creates its own boundary; there is no `state.view` to declare for this version.
Keep selection controls outside so they remain usable while loading.

```html title=web/index.html · inside App
<button on:click='state.product.set("A".into())'>Choose A</button>
<button on:click='state.product.set("B".into())'>Choose B</button>
<Async>
  <section>
    <h3>Product {{ state.product.get() }}</h3>
    <Price product="{{ state.product.clone() }}"></Price>
    <Stock product="{{ state.product.clone() }}"></Stock>
  </section>
</Async>
```

> Each tag constructs a separate child. `Price::from_inputs` and `Stock::from_inputs`
> receive their own `OwnerHandle` and pass it to the constructor that creates
> `browser::read`. Their product input is a shared `Signal`, so the existing children follow
> the selected product.

- [Watch two reads publish together](/docs/showcase/coherent)
- [Compare async and coherent async side by side](/docs/showcase/comparison)
- [What is owner, and where does it come from?](/docs/ownership/mounting#owner-variable)

## Let each child declare its own read {#read-declaration}

Each child calls `browser::read` with three arguments: its owner, a function that reads the
selected product, and an async loader.

The result is `AsyncValue<String, String, String>`: product code, response text, and error
message. Declaring the read does not display it; the child’s HTML uses `<Await>` below.

```rust source=tutorial/src/pricing.rs title=src/pricing.rs · complete file
```

## Put each resolved value in its template {#await}

This HTML file defines the `Price` and `Stock` templates. In the companion’s `Cargo.toml`,
it is registered as `pricing = "web/pricing.html"` under
`[package.metadata.fusor.components]`; `src/lib.rs` declares `mod pricing;`.

Within each `<Await>`, `let="result"` gives the successful value the local name `result`.
Its type is `Rc<String>`: a shared pointer to the loaded string. Call `result.as_str()` to
display its text. This local name is available only inside that `<Await>`.

The companion serves its test data from `public/data/price/{A,B}.txt` and
`public/data/stock/{A,B}.txt`.

```html source=tutorial/web/pricing.html title=web/pricing.html · complete file
```

- [See the template registration](/docs/source/tutorial-Cargo.toml.txt)

## Add status and retry controls when needed {#controls}

For access from Rust, add view: `AsyncBoundary` to `App` and initialize it with
`AsyncBoundary::coherent()`. Pass that handle to `<Async>`.

This is what the runnable companion does; automatic `<Async>` works without it. A boundary
can attach to only one live region.

```html title=HTML with an explicit boundary
<p role="status">{{ format!("{:?}", state.view.status()) }}</p>
<button on:click="state.view.retry()">Retry</button>
<Async boundary="{{ state.view }}">
  <section>
    <h3>{{ state.product.get() }}</h3>
    <Price product="{{ state.product.clone() }}"></Price>
    <Stock product="{{ state.product.clone() }}"></Stock>
  </section>
</Async>
```

> Import `fusor_async::AsyncBoundary` in `App`’s Rust module. For controls on just one
> independent result, put only that `<Await>` inside its own explicit `<Async>`. Keep status
> and retry controls outside the controlled region.

## Know what the reader sees on failure {#failure}

While B loads, keep showing the complete A view. Once both B requests succeed, switch the
label and both results together.

If a request fails, keep the last complete view and let the reader retry using the outside
button.

Before any successful load, the region shows its authored placeholders. Old content cannot
be interacted with while the replacement is pending or failed.

> Standalone `<Await>` follows the same retention rule for its own result. Separate
> `<Await>` views can show results from different selections; use `<Async>` if that would be
> misleading. Use `Resource` when you need to build your own loading, error, and editing UI.

- [Precise loading and failure behavior](/docs/coherent-async/semantics#failure)

## Where to use this today {#constraints}

Use `<Async>` and `<Await>` for read-only result views with generated bindings, component
tags, `<Children>`, and `<ForEach>`. Each built-in wraps one native HTML root without
creating an element.

`<Await>` may nest to express dependent reads.

Nested `<Async>` boundaries, editable controls, router outlets, external widgets, and
independent islands inside these views are not supported. A browser view may use these
components inside an activated island.

> `<Await>` takes `AsyncValue` from `browser::read`, not `Resource` or an arbitrary future.
> Each read declaration belongs to one live boundary; create separate declarations for
> independent views. This coordinates what appears on screen, not an atomic snapshot of your
> backend.

- [Detailed constraints and lifecycle rules](/docs/coherent-async/semantics#constraints)
