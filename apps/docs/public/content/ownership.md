# Owners and cleanup

An owner ties work to a visible component’s lifetime. The framework supplies it when
mounting the component and disposes it when that component is removed.

## Start with the behavior in your page {#lifetime}

Open the companion introduced in Component identity. Its Owned work section starts with a
visible panel and “Panel mounted”.

Click Toggle owned panel: the child disappears and the parent’s status becomes “Panel
removed; cleanup ran”. Click again: a fresh instance mounts. This is the lifecycle your own
subscriptions and resources can follow.

```sh title=Terminal
# From the fusor repository root, after Installation:
fusor dev --manifest-path apps/docs/tutorial/Cargo.toml --port 8091
```

> The `Watch` tag receives its lifecycle input from `App`. `FromInputs` receives the child
> owner separately from the framework.

- [The companion app and source files](/docs/explicit-composition#run)
- [OwnerHandle, constructor arguments, and Rust-side mounting](/docs/ownership/mounting)

## Where does owner come from? {#mount}

The `<Watch>` tag passes one named input: `lifecycle`.

Before constructing `Watch`, the framework creates its child `owner` and calls
`Watch::from_inputs(inputs, owner)`. That ordinary Rust function receives
`owner: OwnerHandle` and passes it to `Watch::new`. The HTML does not need to name `owner`.

```html title=web/index.html · inside App
<button on:click="state.watching.update(|v| *v = !*v)">Toggle owned panel</button>
<p role="status">{{ state.lifecycle.get() }}</p>
<Watch lifecycle="{{ state.lifecycle.clone() }}" rust:if="state.watching.get()"></Watch>
```

> `App` supplies watching: `Signal<bool>` and lifecycle: `Signal<String>`. The child’s
> `FromInputs` implementation is included in `watch.rs` below. It receives a different
> `owner` from `App`’s `owner`; removing `Watch` stops only `Watch`’s owned work.

- [What is owner, and where does it come from?](/docs/ownership/mounting#owner-variable)

## Keep lifecycle registrations in the component {#cleanup}

In the Rust file below, `owner` exists because `Watch::new` declares an `owner: OwnerHandle`
parameter. `Watch::from_inputs` receives the framework’s handle and passes it to that
parameter. You could rename the Rust parameter to lifetime without changing the tag.

`Watch` uses it to register two callbacks: `on_activate` runs when the prepared view becomes
active, and `on_cleanup` runs once when its `owner` is disposed.

Each returns a `Registration` guard. Store those guards to keep the callbacks registered;
dropping one early unregisters its callback without invoking it.

```rust source=tutorial/src/watch.rs title=src/watch.rs · complete file
```

> For application resources, use activation to start work that should begin only after
> mounting succeeds, and cleanup to stop it. Built-in DOM listeners and `Resource` already
> integrate with owners; they do not need a second manual cleanup callback.

- [Watch owned work start and stop](/docs/showcase/lifecycle)

## Give Watch its HTML {#template}

`Watch` is still a normal reusable component. This template supplies its paragraph. The
companion registers watch = "`web/watch.html`", declares mod watch in `lib.rs`, and imports
`Watch` in `app.rs`. Its `FromInputs` implementation receives the child owner from the
framework and passes it to the constructor.

```html source=tutorial/web/watch.html title=web/watch.html · complete file
```

## What prepare, activate, and dispose mean {#timeline}

Preparation creates state and checks that the view can mount. Activation commits a
successfully prepared view and starts its owned work.

Disposal stops the owner and its descendants when the child is removed, replaced by a new
key, or its route is left.

A hidden attribute only hides pixels; `rust:if` on a mount removes the component. Holding an
`OwnerHandle` elsewhere does not keep a removed view active.

> Despite the name, an owner is a framework object that scopes a mounted view’s lifetime and
> cleanup; it is not Rust’s general ownership-and-borrowing rules, which apply to every
> value in the file regardless.
>
> After this page’s panel has been removed, its elements are gone, its registered cleanup
> has run, and its owned work has stopped. The removed view no longer reacts.
>
> These phases describe framework behavior. Low-level `Owner::new`/commit/dispose APIs exist
> for integrations and tests; normal app authors use the supplied handle.

- [See this cancel a request](/docs/async-data#cancel)

## Try typed context in your generated app {#context}

Constructor arguments are the simplest way to pass direct inputs. Context is useful when
many descendants need the same service.

This optional, complete lesson starts from the generated my-app: replace `src/app.rs` and
`web/index.html` with the two files below. Keep its other generated files, including
`lib.rs` and `build.rs`. It uses the existing fusor and `wasm-bindgen` dependencies.

`Theme` is a typed key; `App` provides one signal, and `Badge` retrieves the nearest `Theme`
provider.

```rust source=tutorial/lessons/context/app.rs title=src/app.rs · complete optional context lesson
```

> `Rc` is Rust’s reference-counted shared pointer: cloning it hands out another reference to
> one value. Context stores the provided value behind an `Rc`, so the child’s
> `Rc<Signal<String>>` follows the same signal its parent provided.
>
> A missing provider is an explicit error; providing a context is not itself a reactive
> operation.

- [Try shared context in a live app](/docs/showcase/context)

## Mount the provider and consumer {#provide-context}

Both constructors above return `Result<Self, JsValue>`. The `<App>` state expression uses ?;
`Badge::from_inputs` forwards its constructor’s Result. The root receives the app owner;
`Badge` receives a child owner whose context lookup reaches `App`.

The same entry HTML contains both the `App` root and `Badge` template, associated with this
Rust module by `template!("web/index.html")`.

Run `fusor dev --port 8090` in my-app: the badge says “Theme: dark”. Click Use light theme:
its text and `data-theme` attribute become light.

```html source=tutorial/lessons/context/index.html title=web/index.html · complete optional context lesson
```

> Use `App::new(owner)`? when your constructor is fallible. Use `App::new(owner)` for one
> returning `App` directly. This is ordinary Rust error propagation, not a different
> component API.
