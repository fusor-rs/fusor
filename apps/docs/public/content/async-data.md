# Async data loading

Fetch data, show loading and errors, and retry from HTML. Request lifetime follows the
component that owns it.

## Try an actual HTTP read {#run}

Run the companion and open http://127.0.0.1:8091/. Its Async data section fetches
`/data/1.txt` and displays its text.

Choose issue 2 to load the other file, Try a missing issue to see an HTTP 404 error, and
Reload / retry to make a new attempt.

These are real browser requests to static files under `public/data`, so no backend or
credentials are needed.

```sh title=Terminal
# From the fusor repository root, after Installation:
fusor dev --manifest-path apps/docs/tutorial/Cargo.toml --port 8091
```

> In DevTools, slow the network to make pending states easy to see. The same mechanism can
> call your API; this tutorial uses text files to keep the example self-contained. This
> guide uses the separate companion’s explicit mounts and external-script registration. Run
> it as supplied; copying `reader.rs` alone into `my-app` is not the full setup.

- [Companion source and setup](/docs/explicit-composition#run)
- [Resource and request API reference](/docs/async-data/resource)

## Add the async crate to your own app {#dependency}

The runnable companion already declares `fusor-async` with the browser feature.

The line below shows the dependency to add under `[dependencies]` if adapting this code in
the `my-app` from Installation. It is only the dependency step: use the complete file-wiring
checklist at the end of this page before copying the companion’s files.

```toml title=Cargo.toml · add under [dependencies]
fusor-async = { version = "=0.1.5", features = ["browser"] }
```

## First, read the three arguments {#resource-call}

`browser::resource` comes from `fusor-async`. Its three arguments answer three questions:
whose lifetime owns this request, which key should load, and how should it load?

Here `owner` is a parameter of `Reader::new`. The framework first passes the child’s
`OwnerHandle` to `Reader::from_inputs`, which forwards it to
`Reader::new(owner, inputs.selected_id)`. Borrowing it as `&owner` ties the resource to the
reader’s lifetime.

The resource evaluates your key function, then passes its result and a `CancellationToken`
to your loader. The file imports `browser::resource` and `fetch::get_text`, so the example
can call them simply `resource(...)` and `get_text(...)`.

```rust title=Rust · inside Reader::new
use fusor_async::{browser, fetch};

// Inside Reader::new(owner: OwnerHandle, selected_id: Signal<u32>):
let data = browser::resource(
    &owner,                              // whose lifetime?
    move || Some(selected_id.get()),     // which request?
    |id, cancel| async move {            // how to load it?
        fetch::get_text(&format!("/data/{id}.txt"), &cancel).await
    },
);
```

- [Resource API: arguments, return value, and state](/docs/async-data/resource#resource)
- [What is owner, and where does it come from?](/docs/ownership/mounting#owner-variable)

## Create and read a Resource in one component {#resource}

`Resource<K, T, E>` describes three types: the request key, successful value, and error.
This reader uses `u32`, `String`, and `FetchError`.

The key closure reads `selected_id` synchronously, so changes to that signal start a new
request. The loader receives the selected ID and a `CancellationToken` named `cancel`. Work
begins after the component’s owner activates.

The `status()` and `text()` methods read the current resource state with `.with()`. Calling
them from HTML tracks updates. Their `&self` parameter borrows the reader instead of taking
ownership; handles such as signals and resources can still update their shared state through
that reference.

```rust source=tutorial/src/reader.rs title=src/reader.rs · complete file
```

> The key function returns `Some(id)` to load a request, or `None` to disable the resource
> and clear previous data. Only this synchronous key function tracks changing inputs; signal
> reads inside the async loader do not become dependencies.
>
> The loader’s `async move { … }` block creates a Rust future that owns its captured ID.
> Unlike creating a JavaScript promise, creating a Rust future does not start it. The
> framework runs it after the owner activates.

## Render loading, data, and retry in HTML {#display}

This template supplies `Reader`’s section. The status paragraph announces loading, success,
and error. The body uses the successful data’s own key in its label.

The button invokes `refresh()`, which retries the current key.

`Reader` is a normal component: register reader = "`web/reader.html`", declare `mod reader`
in `lib.rs`, and import `Reader` in `app.rs`.

```html source=tutorial/web/reader.html title=web/reader.html · complete file
```

> `is_loading()` reports whether an attempt is pending. data() returns the available
> successful payload, whose key identifies the request that produced it and whose value is
> the returned text.

- [Try loading, errors, and retry](/docs/showcase/loading)

## Keep the reader while its selection changes {#mount}

`App` owns selected\_id: `Signal<u32>` initialized to 1 and show\_reader: `Signal<bool>`
initialized to true. This mount has no changing rust:key: it retains the same `Reader` while
passing a shared selection signal.

A new id changes the resource’s request key. Toggle reader removes the entire component and
its request; showing it again creates a new reader.

```html title=web/index.html · inside App
<button on:click="state.selected_id.set(1)">Select issue 1</button>
<button on:click="state.selected_id.set(2)">Select issue 2</button>
<button on:click="state.show_reader.update(|v| *v = !*v)">Toggle reader</button>
<Reader selected_id="{{ state.selected_id.clone() }}" rust:if="state.show_reader.get()"></Reader>
```

- [Complete companion App state](/docs/source/tutorial-app.rs.txt)
- [Complete companion HTML](/docs/source/tutorial-index.html.txt)
- [Companion manifest and registration](/docs/source/tutorial-Cargo.toml.txt)

## Label the data you are actually showing {#previous}

While issue 2 loads, `ResourceState::Loading` can retain the last successful result for
issue 1. data() returns that previous result during loading or an error.

This example therefore keeps “Issue 1: …” visible while status says “Loading issue 2…”. It
never labels old content as issue 2.

If you prefer an empty loading view, match only `ResourceState::Ready` in text().

> Idle means no selected key; Loading means an attempt is running; Ready holds data and its
> key; Error holds a failed key and error; Disposed is terminal. Refresh is explicit; there
> is no automatic retry or shared cache.

## Observe cancellation and stale-result protection {#cancel}

With network throttling enabled, select issue 2 and then issue 1 before the request
finishes. The obsolete attempt loses permission to publish, even if it completes late.

`get_text` also aborts the browser fetch. Toggle reader while a read is pending: disposal
cancels that read.

Cancellation stops local work where supported; it cannot undo changes a server has already
made.

With the `fusor-std` actions feature, `Action::new(owner, load, spawn)` admits one command
at a time. The browser feature provides `actions::browser::action(owner, load)`.
Pending commands reject another dispatch with `AdmissionError::Busy`.
Forms retain failed response mappings as
`FormError` in `form.publication_error()` and `action.state().publication_error`; display
the error for recovery guidance and reconcile the confirmed server state before retrying.

- [Why removal cancels owned work](/docs/ownership)

## Keep your data layer in your application {#choose}

A loader can return any compatible Rust future. You can use your own client, deserialize
JSON, supply authentication, or read a cache.

Put every changing request input in the synchronous key closure. To keep cancellation, pass
`cancel.abort_signal()` to your own Fetch, or use `cancel.on_cancel` for other transports.

Use `Resource` when this panel may publish independently. Use `AsyncValue` with a coherent
region when several results and their labels must become visible together.

- [Coordinate several reads](/docs/coherent-async)
- [Mount a reader from a route](/docs/routing)
- [Compare async and coherent async side by side](/docs/showcase/comparison)

## Copy this reader into your generated app {#adapt}

To copy this companion reader into the generated `my-app`, first add the async dependency
above. Copy `reader.rs` to `src/reader.rs`, `reader.html` to `web/reader.html`, and the two
text files into `public/data/`. Add the Cargo registration below.

In `src/lib.rs`, keep the existing modules and `include!(env!("FUSOR_MODULE"));`, then add
`mod reader;`. Keep `bindings!(reader)` in `reader.rs` and the script in `reader.html`:
together they select the registered module.

In `src/app.rs`, import `crate::reader::Reader`. Add `selected_id: Signal<u32>` initialized
with `signal(1)` and `show_reader: Signal<bool>` initialized with `signal(true)`. Keep the
entry’s `template!("web/index.html")` call.

Finally, place the HTML from “Keep the reader while its selection changes” inside the
existing `<App>` root.

```toml title=Cargo.toml · add this table, or add reader to the existing one
[package.metadata.fusor.components]
reader = "web/reader.html"
```

> These paths and URLs assume the generated app’s / base path. If you prefer the script-free
> style, put `reader.html` under `web/components/`, remove its script, and replace
> `bindings!(reader)` with `template!("web/components/reader.html")`.
>
> In that variant there is no reader Cargo registration or generated root include; still
> declare `mod reader` and use the same explicit mount.

- [Understand both file-connection styles](/docs/explicit-composition#authoring-styles)
