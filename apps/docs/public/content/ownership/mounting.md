# Owners, constructors, and Rust mounting

The framework owns a view’s lifetime. Your constructors are ordinary Rust functions, and you
choose which arguments to pass.

## What is owner, and where does it come from? {#owner-variable}

`OwnerHandle` is the public type for a handle to a view’s lifetime. With component tags, the
framework supplies it as the second argument to `FromInputs::from_inputs`.

In your Rust implementation, `owner` is a normal parameter you declare and may rename. It is
not a global variable or a value created by importing `OwnerHandle`.

```rust title=Rust · Watch’s input conversion
impl fusor::dom::FromInputs for Watch {
    type Error = fusor::dom::JsValue;
    type Inputs = WatchInputs;
    fn from_inputs(
        inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(owner, inputs.lifecycle))
    }
}
```

> `WatchInputs` contains a field declared as `pub lifecycle: Signal<String>`. The framework
> calls this method; your implementation chooses which arguments to pass to `Watch::new`.
> The corresponding HTML is `<Watch lifecycle="{{ state.lifecycle.clone() }}"></Watch>`.

- [See the complete Watch constructor using this parameter](/docs/ownership#cleanup)
- [Which expressions receive which values?](/docs/ownership/mounting#supplied)

## What an OwnerHandle actually gives you {#owner}

`OwnerHandle` is a weak handle to a framework lifetime. It lets work follow one mounted
component: start after successful activation and stop on disposal.

The framework retains the actual owner. Cloning or storing the handle does not keep a
removed component alive. It is separate from Rust’s ownership and borrowing rules.

> You normally receive a handle from the framework. Creating a new unrelated Owner inside a
> component would give that work a separate lifetime.

## How many parameters can new have? {#constructors}

Your own constructors remain ordinary Rust functions with any parameters. Component tags use
one fixed entry point: `FromInputs::from_inputs(inputs, owner)` -&gt;
`Result<Self, Self::Error>`. Implement it to call any constructor, perform context lookup,
or return an error.

For straightforward fields, derive `FromInputs` instead; the components guide explains
`#[input]` and #\[local(init = ...)\]. Use a manual implementation when initialization needs
the owner.

```rust title=Rust · signatures only
// Your constructors can have different signatures:
fn new() -> Self
fn new(owner: OwnerHandle, count: Signal<i32>) -> Self
fn new(owner: OwnerHandle, id: u32, title: String) -> Result<Self, JsValue>

// Inside your FromInputs implementation, forward a fallible result:
Self::new(owner, inputs.id, inputs.title)
```

> The framework does not inspect new or guess its arguments. Named HTML inputs build the
> Inputs struct, and your implementation controls initialization.

- [Use the simpler component-tag pattern](/docs/components)

## Which values are supplied, and where? {#supplied}

Inside template bindings, `state` refers to the enclosing component. The `<App>` state
expression instead receives `owner`, because it is still constructing the value that will
become `state`. A child component receives its own owner through `FromInputs::from_inputs`.

`<ForEach>` makes `item: Memo<T>` and `index: Memo<usize>` available inside its child HTML.
Its key closure receives an ordinary `&T` argument. `<Await>` exposes an `Rc<T>` result
under the name chosen by `let`.

An `on:` handler receives `event: web_sys::Event`. These names exist only in their
documented expressions or callbacks; they are not globals in a Rust file.

> Inside an ordinary Rust file, owner must be a function argument, local variable, or
> closure parameter. Receiving it in `FromInputs` is the usual way to start owned component
> work.

- [Look up each expression context](/docs/html-and-rust/attributes)

## What happens when you write &lt;Counter&gt;? {#tags}

Component tags use a fixed trait contract instead of calling new. The framework builds
`CounterInputs` from the tag’s named attributes, then calls
`Counter::from_inputs(inputs, owner)`.

The owner belongs to that new `Counter`. `#[derive(FromInputs)]` generates this
implementation for simple input/local fields; it does not generate a new method.

```rust title=Rust · FromInputs trait method
fn from_inputs(
    inputs: Self::Inputs,
    owner: OwnerHandle,
) -> Result<Self, Self::Error>
```

> The derive has no implicit `owner` or `inputs` variable in #\[local(init = ...)\].
> Implement `FromInputs` manually when initialization depends on the owner or on another
> input.

- [See the manual constructor alternative](/docs/components#manual-inputs)

## Prepare → activate → dispose {#lifecycle}

Preparation validates the template and constructs the state. Activation starts owned work
after mounting succeeds; ancestors must also be active.

Disposal ends that lifetime and its descendants, including when a key changes or a route is
replaced. Ordinary signal updates keep the same instance. `hidden` only changes visibility.

- [Complete activation and cleanup example](/docs/ownership#cleanup)

## Keep callback registrations alive {#registrations}

`owner.on_activate(callback)` and `owner.on_cleanup(callback)` each return a `Registration`
guard. Store those guards in the component for the callbacks to remain registered. Dropping
a guard early unregisters the callback without invoking it.

Built-in resources and generated listeners already arrange their own lifetime integration.

- [Watch stores both guards in its Rust struct](/docs/ownership#cleanup)

## Context is explicit lookup, not constructor injection {#context}

`owner.provide`::`<Key>`(value) stores a value under a typed `ContextKey` and can fail.
`owner.context`::`<Key>`() returns `Option<Rc<Key::Value>>` from the nearest provider,
including the current owner.

`None` means no provider is available. `Rc` shares that service value; it does not keep the
owner active.

Lookup itself is untracked; put a `Signal` in the service for reactive changes.

- [Run the complete provider/consumer lesson](/docs/context)

## Mount a compiled component from Rust {#rust-mount}

Use this lower-level API when embedding a component into an existing browser UI. Normal
fusor apps use `<App>` and component tags, which retain scopes for you.

For a reusable template, `Counter::prepare(&parent.owner()`, factory) creates a child
`Scope` without activating it. `parent.mount_child(target, child)` inserts, commits, and
retains it.

Keep the returned parent `Scope` alive; dropping it stops its bindings and removes the
mounted child.

```rust title=Rust · integration function
use fusor::{dom::{Component, Scope}, signal};
use wasm_bindgen::JsValue;

// Counter is the Rust state type linked to its compiled HTML template.
// Its count field must be accessible from this module.
fn mount_counter() -> Result<Scope, JsValue> {
    let mut parent = Scope::at("#counter-host")?;
    let child = Counter::prepare(&parent.owner(), |_owner| {
        Ok(Counter { count: signal(0) })
    })?;
    parent.mount_child(":scope", child)?;
    Ok(parent) // caller retains this guard for the UI's lifetime
}
```

> This assumes a `<div id="counter-host"></div>`, a compiled `Counter` template, and
> initialized browser/Wasm code. Calling it and immediately dropping its result would
> immediately remove the child. The complete optional lesson below includes the startup and
> retention code.

- [Complete Rust-side mounting lesson](/docs/ownership/mounting#manual-app)

## Try the complete low-level mounting lesson {#manual-app}

Start from `my-app` generated in Installation. Replace `src/app.rs` and `web/index.html`
with these two files; keep `lib.rs` as generated, along with `build.rs`, Cargo dependencies
and metadata. The generated counter module may remain; this lesson defines its own `Counter`
in `app.rs`.

Run `fusor dev` `--port` 8090. The page shows Count: 0; Add one increments it.

This lesson deliberately uses a Wasm start function and retains `Scope` in a thread-local
cell, showing the lifecycle work `<App>` normally performs for you.

```rust source=tutorial/lessons/mounting/app.rs title=src/app.rs · complete optional lesson
```

## The compiled HTML for that Rust mount {#manual-html}

There is no `<App>` here: the Rust start function chooses #counter-host as the insertion
point.

The reusable `Counter` template supplies the section that appears inside it.
`template!("web/index.html")` associates the template with `Counter` in `app.rs`.

```html source=tutorial/lessons/mounting/index.html title=web/index.html · complete optional lesson
```

## Other mounting methods and cleanup {#scope}

`component.mount()` and `Component::mount_with(factory)` return an already committed
`Scope`. For reusable templates, that root starts detached; these methods do not pick a DOM
container.

`mount_with` supplies its new owner to the factory; `try_mount_with` accepts a fallible
factory. Prefer prepare followed by insertion for integrations where work must start only
after insertion succeeds.

> For manual control, `scope.attach(&element)` inserts and marks the root for removal on
> drop; `scope.try_commit()` activates it and returns setup errors.
>
> Keep the scope. `scope.dispose()` stops owned work immediately but does not itself remove
> the root. Dropping an attached scope removes it; dropping a scope bound to pre-existing
> markup leaves that markup in place.

## JavaScript libraries follow the component lifetime {#widgets}

Put library setup in the component module’s `onMount` function. fusor calls it after
activation and aborts its `signal` when the component is disposed.

Register destruction, observer disconnection, and unsubscribe functions with `onCleanup` as
soon as you acquire them. Setup errors also run registered cleanup.

Library instances update through optional input subscriptions; `signal` updates do not rerun
`onMount`.

```javascript title=JavaScript · lifecycle shape; library methods vary
export function onMount({ root, inputs, signal, onCleanup }) {
  // Initialize the library with its normal JavaScript API.
  // onCleanup(() => instance.destroy());
  // onCleanup(inputs.value.subscribe(value => instance.update(value)));
}
```

> `signal` here is a browser `AbortSignal`. It is not a Rust `Signal` or `OwnerHandle`.
> Check `signal.aborted` before publishing a late asynchronous result. You still need to
> release resources created by the library.

- [onMount context and parameter types](/docs/npm#mount-types)
- [Connect any browser library](/docs/npm)
