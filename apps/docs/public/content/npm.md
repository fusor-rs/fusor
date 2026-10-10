# Native JavaScript and npm

Bring a browser library into your app using its normal JavaScript API. Learn where the code
lives, how Rust and JavaScript exchange values, and who cleans up when a component
disappears.

## The connection, independent of the library {#model}

fusor compiles your Rust state and HTML into a browser application. A JavaScript module can
run alongside that component. Rust owns the application state; JavaScript calls the library.

Rust sends selected values through read-only inputs, and JavaScript reports results through
ordinary DOM events. You do not need a Rust wrapper for each package.

JavaScript is optional: a utility can run without Rust inputs or events, and a widget can
run without reactive state.

```text title=Text · Data flow
Rust signal → JavaScript input → library call
Library result or event → DOM CustomEvent → Rust handler → updated HTML
```

- [New to Fusor? Start with a browser app](/docs/installation)
- [A signal is a value the page watches](/docs/reactivity)

## First, check the library’s browser API {#compatibility}

A function library can be imported and called directly. A chart, map, editor, or other
stateful widget fits when its browser API lets you choose a host element and release its
resources.

An existing Web `Component` fits through its native custom tag.

Look for the package’s browser import, initialization, update, events, and destroy/dispose
instructions; those remain the library’s API, not fusor APIs.

> Installing an npm package does not make every package browser-compatible. Node-only APIs
> such as fs are not provided.
>
> React-only components still require React’s own runtime and mounting API inside a
> dedicated host; they do not become fusor component tags.
>
> This integration currently targets client-rendered browser components. Server-rendered
> modes and coordinated `<Async>` regions are outside its scope; see the full release
> boundaries below for exact restrictions and supported npm layouts.

- [Existing Web Components](/docs/npm#web-components)
- [Full release boundaries](/docs/npm#bounds)

## Enable JavaScript once per application {#setup}

After installing the fusor CLI and its browser tools, create an app with --javascript.

The flag enables the Rust feature, adds the pinned esbuild build tool, and connects
`web/app.js` to the app HTML.

Run these commands wherever you keep projects; the command creates the `my-app` directory.
If you already have an app, follow the next section instead.

```sh title=Terminal · New application
fusor new my-app --javascript
cd my-app
fusor dev
```

> Node 22+ is required for JavaScript browser builds. Commit `package.json` and the
> version-3 `package-lock.json`; use `npm ci` on subsequent installations. Cargo does not
> install npm packages. Native `cargo check` remains Node-free.

- [Install the CLI and browser tools](/docs/installation)

## Already have an app? {#existing-app}

Run `fusor add javascript` in your app to enable the Rust feature and prepare pinned esbuild
dependencies. Existing sources and features are preserved. Use Node 22+ for JavaScript
browser builds. Commit the updated manifests and lockfiles.

Create `web/app.js`. Add `<script type="module" src="./app.js"></script>` directly inside
the existing `<App>`, beside its native root, as shown below. Keep `build.rs` calling
`fusor_build::compile_app()`.

```sh title=Terminal · Existing application
fusor add javascript
```

> A Cargo feature enables optional Rust code in a dependency. You enable this feature once
> per app, not once per library. If this checkout uses a local path dependency, preserve
> that path.

- [Complete HTML and module placement](/docs/npm#complete-app)

## Repeat this pattern for each library {#recipe}

Install the package from your app’s Cargo directory, beside `Cargo.toml`: npm install
--save-exact PACKAGE (replace PACKAGE with the package name). Import it in your module using
the import path documented by its author. Then choose only the connections you need:

```text title=Text · Integration checklist
1. Put per-instance setup in onMount.
2. If the library draws UI, give it a dedicated host inside root.
3. If it needs Rust state, expose a signal and subscribe to that input.
4. If Rust needs a result, dispatch a CustomEvent on root.
5. If it creates resources, register their cleanup immediately.
```

> Create a stateful library instance once inside `onMount`, then update that instance in
> subscriptions. Creating it inside every subscription callback would recreate it on each
> change. One component module can import multiple libraries and helper modules; a separate
> wrapper component is optional.

## What onMount receives {#mount-context}

fusor calls `onMount` with one context object when this component instance becomes active in
the page and its Rust event handlers are ready.

The `{ root, inputs, signal, onCleanup }` syntax is ordinary JavaScript object
destructuring: it takes four named properties from that one argument. You can take only the
properties you need; you do not construct or pass the context yourself.

```javascript title=JavaScript · The framework entry point
export function onMount(context) {
  const { root, inputs, signal, onCleanup } = context;
  // Equivalent to onMount({ root, inputs, signal, onCleanup }).
  // Instance-specific variables belong inside this function.
}
```

> This is a fusor calling convention for an ordinary named JavaScript export. Top-level
> imports/code run once per module URL; `onMount` runs separately for each instance. Rust
> `signal` changes and keyed moves do not rerun it. You can omit `onMount` for a module that
> only imports a Web `Component` registration.

- [Context property types](/docs/npm#mount-types)
- [Use the generated types in your editor](/docs/npm#imports)

## The types inside the context {#mount-types}

`root` is the component’s native DOM `Element`, such as its `<main>` or `<section>`. It is
not the `<App>` marker or a Rust value.

`inputs` contains the read-only Rust values this component exposes to JavaScript. Each
connection provides `get()` for a snapshot and `subscribe()` for updates.

`signal` is a browser `AbortSignal` tied to the component’s lifetime. It is unrelated to the
Rust `Signal<T>` type.

`onCleanup` accepts a function with no arguments and registers it to run during cleanup. It
returns nothing. The `onMount` function must be synchronous and cannot return a `Promise`.
For deferred work, use the async setup pattern later in this guide.

The generated type contract below shows all four properties together.

```typescript title=TypeScript · Generated contract, not code you need to write
export interface ReadonlyInput<T> {
  get(): T;
  subscribe(callback: (value: T) => void): () => void;
}

export interface ComponentMountContext<I> {
  readonly root: Element;
  readonly inputs: I;
  readonly signal: AbortSignal;
  onCleanup(callback: () => void): void;
}

// For the App with #[js] text: Signal<String> shown below:
export interface AppInputs {
  readonly text: ReadonlyInput<string>;
}
export type AppMountContext = ComponentMountContext<AppInputs>;
// onMount returns nothing, or one cleanup function:
export type OnMount = (context: AppMountContext) => void | (() => void);
```

> `T` is an input’s value type; `I` is this component’s exposed input object. Here
> `inputs.text.get()` returns a string. The callback passed to `subscribe` receives a
> string; `subscribe` returns an unsubscribe function. Unmarked Rust fields are absent;
> `inputs` is empty when no fields are exposed.
>
> `readonly` prevents replacing context properties. The DOM element itself remains mutable.
> Since `querySelector` can return `null`, check `instanceof HTMLCanvasElement` before
> passing a selected node to a canvas API.
>
> Use `signal.aborted` to detect removal, or pass `signal` to `fetch` and
> `addEventListener`. Register destruction with `onCleanup(() => widget.destroy())`.

- [Input values and subscriptions](/docs/npm#inputs)
- [When cleanup runs](/docs/npm#cleanup)
- [Import your generated context type](/docs/npm#imports)

## A small complete connection {#files}

The following example uses a tiny function package so you can see Rust → JavaScript → Rust
without chart configuration.

In your generated app, run npm install --save-exact is-lower-case@2.0.2. Add `web-sys` =
"=0.3.94" under `Cargo.toml`’s existing `[dependencies]`; it supplies the Rust browser Event
type used by the return handler.

Replace the three files below. Keep the generated app’s `src/lib.rs`, `build.rs`, and other
files unchanged.

```text title=Text · Files you work with
my-app/
  Cargo.toml          Rust dependencies and JavaScript feature
  package.json        npm library and esbuild
  package-lock.json   committed npm versions
  src/app.rs          Rust state and return-event handler
  web/index.html      HTML and module association
  web/app.js          ordinary library import and call
```

> `web-sys` is needed here because Rust receives a browser event; a JavaScript-only library
> call does not need this Rust event handler. This example does not require a child
> component or a `FromInputs` implementation.

## Rust: choose which state JavaScript can read {#rust-state}

`App` is an ordinary Rust struct holding the page’s state. `Signal<String>` is a reactive
text value; `Signal<bool>` holds true or false.

`#[derive(JsInputs)]` asks Rust to generate the JavaScript input interface. Only the `#[js]`
field text is exposed. `is_lowercase` stays in Rust and receives the checked result.

`new()` supplies initial values; `"rust".into()` creates an owned Rust `String`.

```rust title=Rust · src/app.rs
use fusor::{JsInputs, Signal, signal};

#[derive(JsInputs)]
struct App {
    #[js]
    text: Signal<String>,
    is_lowercase: Signal<bool>,
}

impl App {
    fn new() -> Self {
        Self {
            text: signal("rust".into()),
            is_lowercase: signal(false),
        }
    }

    fn accept_result(&self, event: web_sys::Event) {
        if let Ok(value) = fusor::js::event_detail::<bool>(&event) {
            self.is_lowercase.set(value);
        }
    }
}

fusor::template!("web/index.html");
```

> event\_detail::`<bool>`(&amp;event) asks for a boolean payload. `Result` represents
> success or failure; `if let Ok(value)` handles only a valid payload.
> `template!("web/index.html")` associates this Rust module with that HTML file. The event
> connection is shown next.

- [Rust modules and template association](/docs/project-structure)

## HTML: place the module at its component boundary {#complete-app}

The script is a direct child of `App`, next to its one native root, main. Its `src` is
relative to this HTML file.

In a reusable component, put the script directly inside
`<template rust:component="YourType">` in the same way.

fusor extracts the script and calls its lifecycle export; it does not leave an inert script
inside each cloned template.

```html title=HTML · web/index.html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Rust and a JavaScript library</title>
  <link rel="icon" href="data:,">
</head>
<body>
  <App state="{{ App::new() }}">
    <script type="module" src="./app.js"></script>
    <main on:library-result="state.accept_result(event)">
      <label>Text <input bind="state.text"></label>
      <p>All lowercase: <output>{{ state.is_lowercase.get() }}</output></p>
    </main>
  </App>
</body>
</html>
```

> `App::new()` constructs the Rust value, and state means that value inside this HTML.
> `bind` keeps the input and `state.text` in sync. `on:library-result` calls the Rust method
> when main receives an event named `library-result`; event is supplied by the framework.
> JavaScript root is this same main element.

## JavaScript: call the package normally {#javascript}

The import and isLowerCase(text) call come from the package’s JavaScript API. subscribe
receives the current text immediately, then later Rust changes.

The callback reports the boolean on root; Rust updates `is_lowercase`, and the HTML output
changes.

Save all three files, then run `fusor dev`. The initial word rust shows true; type Rust to
see false.

```javascript title=JavaScript · web/app.js
import { isLowerCase } from "is-lower-case";

export function onMount({ root, inputs }) {
  inputs.text.subscribe((text) => {
    const result = isLowerCase(text);
    root.dispatchEvent(new CustomEvent("library-result", {
      detail: result,
    }));
  });
}
```

> For a different function library, change the import and call. Match the exposed Rust input
> and result type to its data. This utility owns no timer or UI instance to destroy; the
> framework stops its input subscription when the component is removed. No cleanup stack or
> custom adapter is required for this case.

## Rust → JavaScript: snapshots and subscriptions {#inputs}

`inputs.text.get()` reads the current value once; it does not keep a variable updated.
`inputs.text.subscribe(callback)` immediately delivers that value and then follows changes.

It returns a function you can call to unsubscribe early. All subscriptions stop when this
component is disposed (removed or replaced).

Notifications follow a Rust reactive batch: for example, the grouped state writes made by
one event handler. JavaScript inputs have no `set` method.

```javascript title=JavaScript · Inside onMount
const currentText = inputs.text.get();
const stop = inputs.text.subscribe((text) => {
  console.log("Current Rust text:", text);
});
// When this particular subscription is no longer needed:
stop();
```

> Supported `Signal` values are `bool`, `String`, `f64`, `i32`, `u32`, and nested
> `Option`/Vec combinations. `Vec` is a Rust array-like collection, sent as a JS array
> snapshot; `Option` is an optional value, with `None` sent as null. Mutating a snapshot
> does not write back to Rust.
>
> Advanced callers may expose `JsValue` handles, which retain JS identity. Arbitrary Rust
> structs and 64-bit integers are not converted automatically.
>
> Subscribe to separate inputs separately; this is not an atomic snapshot of several fields.

## JavaScript → Rust: match the event name and payload {#events}

Choose your own event name. `new CustomEvent("library-result", { detail: result })` matches
`on:library-result` in HTML.

The Rust handler receives a `web_sys::Event`, then event\_detail::`<bool>`(&amp;event)
validates its payload. Change `bool` to a supported type such as `String` when your library
returns text.

Invalid types return an error; application-specific checks, such as whether an ID exists,
are still yours.

```javascript title=JavaScript · Dispatch from a library callback
root.dispatchEvent(new CustomEvent("library-result", {
  detail: true,
}));
```

> Dispatching on `root` reaches the listener on that same element; bubbling is unnecessary.
> Dispatching from a descendant requires `bubbles: true` to reach a `root` listener. Use the
> library’s callback/event API to trigger this dispatch. A module cannot call an arbitrary
> Rust method by its JavaScript name; the HTML event handler makes that connection explicit.

- [Events: browser handlers and custom messages](/docs/events#custom-events)

## Give a widget its own empty element {#widget-host}

For a chart, map, or editor, add an empty host inside main alongside your fusor controls.

A data attribute gives JavaScript a way to find that element within this component instance,
without a global ID. Use a canvas instead if the library requires one.

```html title=HTML · Optional widget host
<!-- Inside the component’s main element -->
<div data-library-host></div>
```

> fusor owns the host element and the surrounding page; the library owns the contents it
> creates inside the host. Keep fusor child components and interpolations outside that
> library-managed subtree.

## For a widget, separate DOM ownership and lifetime {#library-lifetime}

Give a library an empty host element, or a canvas, inside your component’s root. fusor
renders the surrounding controls; the library manages that host’s contents.

Do not put fusor-managed children or interpolations inside a subtree the library will
replace.

Find the host with `root.querySelector`, initialize the library once, update it from
subscriptions, and call its documented destroy/dispose method through `onCleanup`.

```javascript title=JavaScript · Use with the host above
export function onMount({ root, onCleanup }) {
  const host = root.querySelector("[data-library-host]");
  if (!host) throw new Error("Missing library host");

  // A native browser resource illustrates the same lifetime rule.
  const observer = new ResizeObserver(([entry]) => {
    console.log("Host width:", entry.contentRect.width);
  });
  onCleanup(() => observer.disconnect());
  observer.observe(host);
}
```

> `ResizeObserver` is a browser API, not an npm package or a fusor wrapper. Replace this
> resource with your library’s documented initialization and release calls. There is no
> universal `destroy()` method. The linked `Chart.js` and `Three.js` modules demonstrate
> actual library APIs.

- [Chart.js: create, update, destroy](/docs/showcase/chartjs)
- [Three.js: renderer, animation and GPU cleanup](/docs/showcase/threejs)

## What cleanup does automatically {#cleanup}

When a component is removed, its JavaScript input subscriptions close, `signal` aborts, and
registered cleanups run in reverse registration order while `root` is still available.

Pass `{ signal }` to native addEventListener or fetch to tie supported work to that
lifetime.

Register explicit cleanup immediately for timers, observers, animation loops, and library
resources; aborting does not destroy those automatically. A returned cleanup function is
also allowed.

> Registering cleanup after disposal runs it immediately. If synchronous `onMount` throws,
> already registered cleanups still run; one failing cleanup does not skip the rest.
>
> fusor reports the error, but does not automatically retry or undo arbitrary library DOM
> changes. A component prepared but never activated does not run `onMount`.

## Optional: load a library only when needed {#async-setup}

Start with a static import unless you want deferred loading. You can replace `web/app.js`
with the version below to defer this utility until the component mounts.

`onMount` must remain synchronous: an async `onMount` or returned `Promise` is an error.

Start `Promise` work inside it, check `signal.aborted` after awaiting/loading, and catch
failures. Dynamic import itself cannot be cancelled; the check prevents installing work
after removal.

```javascript title=JavaScript · Alternative web/app.js
export function onMount({ root, inputs, signal }) {
  void import("is-lower-case").then(({ isLowerCase }) => {
    if (signal.aborted) return;
    inputs.text.subscribe((text) => {
      root.dispatchEvent(new CustomEvent("library-result", {
        detail: isLowerCase(text),
      }));
    });
  }).catch((error) => {
    if (!signal.aborted) console.error(error);
  });
}
```

> For async setup that acquires a chart, renderer, or several resources, also release
> partially acquired work if the `Promise` fails. Framework cleanup catches synchronous
> setup failure; it cannot catch arbitrary future `Promise` callbacks. The showcases explain
> this extra case with a local idempotent cleanup stack used both in catch and `onCleanup`.
> That stack is application JavaScript, not required boilerplate for every library.

- [Async initialization and rollback in a real widget](/docs/showcase/chartjs)

## Imports, TypeScript, and editor help {#imports}

Use external `.js`, `.mjs`, or `.ts` modules, or an inline module body. Relative imports
resolve from that module; bare package imports resolve installed npm dependencies. Follow
the package’s documentation for import names and options. CSS and file assets can be
imported too.

Dynamic imports split JavaScript into separate files. Linked CSS loads with the page. Apps
with component JavaScript currently rebuild and reload on source edits, including HTML and
CSS changes; ordinary reactive input updates keep the existing library instance.

A browser CLI build generates `.fusor/types/` declarations for the associated Rust state.
Build once before adding the annotation, and rebuild after changing `#[js]` fields. Import
the generated type so its inputs stay aligned with your Rust fields. The CLI records
generated files in `.fusor/types/.generated.json` and removes only those declarations when
they become obsolete. An unreadable or corrupt index stops the command with an error; repair
the index before retrying.

Declaration discovery follows inline Rust modules. Keep the component struct and its
`template!` in the associated Rust source; unresolved or ambiguous discovery stops the
build with a source diagnostic. A component without `JsInputs` exposes an empty input shape.

When the runtime crate has another name, put one `#[js_inputs(crate = path)]` attribute on
the struct. Empty or duplicate overrides and overrides placed on fields are rejected.

The JSDoc annotation and `@ts-check` enable editor diagnostics without adding runtime code.
For TypeScript, rename `app.js` to `app.ts`, update the HTML `src`, and use
`import type { AppMountContext } from "../.fusor/types/web-index-html-App";`. Annotate the
argument as `context: AppMountContext` and the return as `void | (() => void)`.

`esbuild` transpiles TypeScript but does not type-check it. Configure and run `tsc --noEmit`
separately if desired. Native `cargo check` does not resolve JavaScript imports.

```javascript title=JavaScript · Optional annotation in web/app.js
// @ts-check
/**
 * @param {import("../.fusor/types/web-index-html-App").AppMountContext} context
 * @returns {void | (() => void)}
 */
export function onMount(context) {
  const { root, inputs, signal, onCleanup } = context;
  // root: Element; signal: AbortSignal
  // inputs.text.get(): string
  // onCleanup: (callback: () => void) => void
}
```

## Existing Web Components use their native tags {#web-components}

An existing custom element is already a browser component; it does not need a Rust struct or
a fusor PascalCase wrapper.

Install its package, add the author’s registration import to your existing module, and use
its native HTML tag.

For example, after npm install --save-exact @shoelace-style/shoelace@2.20.1, add import
"@`shoelace-style/shoelace/dist/components/button/button.js`"; to `web/app.js` and put this
tag inside main:

```html title=HTML · Inside the example’s main
<sl-button prop:disabled="{{ state.is_lowercase.get() }}">
  An existing Web Component
</sl-button>
```

> Here `disabled` is a JavaScript property: `true` disables the button. Attribute
> interpolation is text; `prop:name` passes an actual value and waits for custom-element
> definition. Event/property suffixes preserve case. `Registration` is shared, while each
> native tag is its own element. There is still only one component module script; add
> imports to it. If the library documents a required stylesheet, import it in the same
> module too.

- [Attributes versus properties](/docs/html-and-rust/attributes#properties)
- [Native event handlers](/docs/html-and-rust/attributes#events)

## Troubleshooting and current boundaries {#bounds}

If an import is missing, install the package in this app’s Cargo directory and run the
browser CLI build.

If `inputs` lacks a field, check the `javascript` feature, `JsInputs` derive, and `#[js]`
annotation, then rebuild. If a value changes only once, use `subscribe` instead of reading
once with `get`.

If Rust never receives an event, check its exact name, dispatch target, bubbling, and
payload type. If a widget resets on every update, create it once in `onMount` rather than
inside the subscription callback.

> Current scope: client-rendered browser templates, one native root and at most one module
> declaration per component, one JavaScript-enabled fusor app per page. Server/shared/island
> and coherent templates reject component JavaScript.
>
> Module discovery covers the application package, not dependent Cargo crates.
>
> Builds require package-local npm manifests, a version-3 lock and installed dependencies;
> npm workspaces, linked packages and other package-manager layouts are not supported.
> Rust-only apps do not need Node or this bridge.

## Place a module in a reusable component {#reusable-module}

A reusable component can own its JavaScript module just as the application root does. Put
the module script directly inside its `template` declaration, beside the native root. The
module receives that instance’s root and lifetime in `onMount`.

The `LibraryPanel` Rust type must be associated with this HTML and implement the normal
component construction contract. Its caller then uses `<LibraryPanel></LibraryPanel>`
wherever the integration is needed.

```html title=HTML · LibraryPanel is your component type
<template rust:component="LibraryPanel">
  <script type="module" src="./library-panel.js"></script>
  <div class="library-panel"></div>
</template>
```

> Use `onCleanup` to destroy library instances and release subscriptions. `JsInputs` is
> needed when the module receives Rust inputs; DOM events carry results back to Rust.

- [Define the Rust state and reusable HTML](/docs/components)
- [Types supplied to onMount](/docs/npm#mount-types)

## Make it reusable when your app needs it {#next-steps}

The example used `App` to avoid component plumbing. To reuse an integration, move its Rust
state, HTML host and module into a normal fusor component and use its tag wherever needed.

`JsInputs` exposes state to that component’s JavaScript; `FromInputs` is the separate
contract for values passed by its Rust parent. Neither is a library-specific adapter API.

Start directly in `App`, or make a component immediately when reuse is useful.

- [Connect a reusable Rust component to its HTML](/docs/components#connection)
- [Complete Chart.js component, all three source files](/docs/showcase/chartjs)
- [Complete Three.js component, all three source files](/docs/showcase/threejs)
