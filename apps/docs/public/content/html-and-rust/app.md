# The App boundary

Choose the Rust state for your application and the HTML it owns. The framework handles
startup and lifetime.

## One boundary, one native root {#start}

Start with the generated application from Installation. Replace `src/app.rs` and
`web/index.html` with the complete files below; keep the generated `lib.rs`, `build.rs`, and
manifest.

`<App>` is a reserved built-in HTML tag. It does not refer to an imported Rust component
named `App`.

> Generated applications include `fusor-components` with `features = ["browser"]`. Existing
> applications using `App` need that dependency and feature too.

## 1. Create ordinary Rust state {#state}

`Dashboard` is your own Rust struct. create\_dashboard is your own function. Neither name is
prescribed by the framework. `template!` associates the HTML with this Rust module, so
expressions can access private fields and normal imports.

```rust source=tutorial/lessons/app/app.rs title=src/app.rs · complete file
```

## 2. Place the state around its HTML {#html}

The state expression runs once when the application starts. Its `Dashboard` result becomes
state in the child HTML. `<App>` disappears from the delivered page; main remains the native
root with normal browser semantics.

```html source=tutorial/lessons/app/index.html title=web/index.html · complete file
```

## 3. Run the application {#try-it}

Run `fusor dev`. The page shows Count: 0. Add one updates the output. There is no
handwritten startup function, root mount call, or type registration.

```sh title=Terminal
fusor dev
```

## When your constructor needs owner {#owner}

The framework supplies `owner: OwnerHandle` inside the `state` expression. Pass `owner`
explicitly if your constructor needs lifecycle callbacks, context, requests, or router
setup.

`state` is not available in that expression because the constructor is still creating it.

```html title=HTML
<App state="{{ create_dashboard(owner)? }}">
  <main>…</main>
</App>
```

> This variant assumes your function accepts `OwnerHandle` and returns
> `Result<Dashboard, JsValue>`. Use ? to propagate failure. Your constructor may take
> further explicit arguments; `App` supplies no other arguments or dependency injection.

- [Where owner comes from and what it controls](/docs/ownership/mounting#owner-variable)

## What you can customize {#customize}

Your state expression controls construction. The resulting state can contain services,
signals, routing setup, and retained cleanup registrations.

Keep data loading in your application using the async or query APIs you choose. `<App>`
supplies lifetime and startup machinery; it is not a base class, router, data loader, or
asynchronous component.

> The state type does not receive a generated `Component` implementation from `App`.
> Reusable types still associate their HTML through `rust:component` templates and support
> the manual `Component` mounting APIs.

## Startup, failure, and cleanup {#lifetime}

The framework checks the native root, prepares the state and children, retains their scope,
and activates their owned work. If startup runs twice, the second attempt fails before
calling your constructor again.

Failed initialization releases prepared work and allows a later retry. In an ordinary app,
generated startup runs once.

> Embedding code can explicitly tear down the running app with
> `fusor::dom::application::unmount()`. Removing HTML manually does not call this lifecycle
> API.

- [Owners and cleanup](/docs/ownership)

## Keep the boundary explicit {#rules}

Use at most one `<App>` in the entry document, with exactly one native HTML root. Put DOM
attributes, handlers, and layout on that root.

Keep `<script type="text/rust">` registration scripts outside `<App>`. A JavaScript
component module, `<script type="module">`, instead belongs directly inside `<App>`, beside
the native root. See Native JavaScript and npm for that setup.

`<App>` handles browser startup. It is not supported in server/shared templates or nested
components, and cannot be combined with a manual `wasm_bindgen(start)` entry point.

> Reusable components use `rust:component` to connect their Rust type to HTML. They are
> mounted through their own component tags inside the application root.

- [Where a JavaScript module belongs](/docs/npm#complete-app)
