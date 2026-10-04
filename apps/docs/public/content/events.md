# Events

Connect clicks, keyboard input, and JavaScript results to Rust. Learn what `on:` means,
where `event` comes from, and how a message reaches its handler.

## An event name connects HTML to Rust {#listen}

An event is a browser message that something happened: a click, a key press, or a value your
JavaScript library wants to report.

`on:NAME` listens for that exact name on the element where you write it. The attribute value
is Rust code, run when the event arrives.

```html title=HTML · The basic pattern
<button on:click="state.count.update(|count| *count += 1)">Add one</button>
```

> click is a standard browser event name. A name such as `library-result` is one you choose;
> fusor does not reserve it. Both use the same `on:` syntax. Write handlers on native HTML
> elements, including custom elements. `on:` is not a callback prop on a Rust component tag.

- [Compact on:event reference](/docs/html-and-rust/attributes#events)

## Start with a working Rust-only app {#try-it}

Use the app created by Installation. In its `Cargo.toml`, add `web-sys` = "=0.3.94" under
the existing `[dependencies]`; this provides the Rust name for browser event types.

Replace `src/app.rs` and `web/index.html` with the next two files. Keep the generated app’s
`src/lib.rs`, `build.rs`, and other files. Run `fusor dev` to try it.

> This first example requires no JavaScript module, npm package, or javascript feature.
> fusor supplies the browser connection. You can ignore the event object in a handler that
> only changes state.

- [Create and run the app](/docs/installation)

## Rust: receive the event and change state {#rust-state}

`App` holds two reactive values. `Signal<i32>` stores a count; `Signal<String>` stores text.

The `clicked` method receives a browser event as `web_sys::Event`. `event.type_()` returns
its name, such as `click`. `&self` means the method uses this `App` instance; `Signal` lets
it update reactive values through that shared reference.

```rust title=Rust · src/app.rs
use fusor::{signal, Signal};

struct App {
    count: Signal<i32>,
    last_event: Signal<String>,
}

impl App {
    fn new() -> Self {
        Self {
            count: signal(0),
            last_event: signal("No event yet".into()),
        }
    }

    fn clicked(&self, event: web_sys::Event) {
        self.count.update(|count| *count += 1);
        self.last_event.set(event.type_());
    }
}

fusor::template!("web/index.html");
```

> `template!` associates this Rust module with the HTML file below. The framework groups
> signal writes made by one event handler into a reactive batch, then updates the dependent
> HTML.

## HTML: choose when the method runs {#html}

`App::new()` creates the state object. Inside `App`, state refers to that object. fusor
supplies event while evaluating an `on:` handler; it is not a field you must declare.

Click Add one and the count increases while Last event becomes click.

```html title=HTML · web/index.html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Events in Fusor</title>
  <link rel="icon" href="data:,">
</head>
<body>
  <App state="{{ App::new() }}">
    <main>
      <button type="button" on:click="state.clicked(event)">
        <span>Add one</span>
      </button>
      <p>Count: <output>{{ state.count.get() }}</output></p>
      <p>Last event: <output>{{ state.last_event.get() }}</output></p>
    </main>
  </App>
</body>
</html>
```

> `state.clicked(event)` executes on a click, not during rendering. If your method needs an
> ID or another value, pass it explicitly in the same expression, such as
> `state.select(item.id)`; the framework does not guess method arguments. Use
> `type="button"` for a button that should not submit a surrounding form.

## The event object and more specific types {#event-object}

Every `on:` handler receives a `web_sys::Event`. Its methods include `type_()`, `target()`,
`current_target()`, `prevent_default()`, and `stop_propagation()`.

The target is where the event was dispatched. The current target is the element whose
listener is running. Clicking the inner `<span>` in this example can make that span the
target, while the `<button>` is the current target.

> For keyboard-specific fields such as `key()`, check for `web_sys::KeyboardEvent` with
> `JsCast`, as below. For input text, `bind` is usually simpler than reading the event
> target yourself.

- [Two-way input bindings](/docs/html-and-rust/attributes#bindings)

## Optional: read a pressed key {#keyboard}

Keep the generated app’s `wasm-bindgen` dependency. If it is missing, add
`wasm-bindgen = "=0.2.117"` under `[dependencies]`. Change the `web-sys` entry to
`web-sys = { version = "=0.3.94", features = ["KeyboardEvent"] }`.

Add `use wasm_bindgen::JsCast;` at the top of `src/app.rs`, then place the method below
inside `impl App`. Add
`<input aria-label="Try a key" on:keydown="state.key_pressed(event)">` inside the page’s
`<main>`.

```rust title=Rust · Add inside impl App
fn key_pressed(&self, event: web_sys::Event) {
    if let Some(keyboard) = event.dyn_ref::<web_sys::KeyboardEvent>() {
        self.last_event.set(keyboard.key());
    }
}
```

> `JsCast` supplies `dyn_ref`, a checked cast to a more specific browser type.
> `if let Some(...)` runs the body only when the cast succeeds. This extra dependency
> feature is needed only when you use KeyboardEvent.

## Custom events use the same connection {#custom-events}

The rest of this page is optional. It adds JavaScript to the working Rust example above.
First enable the JavaScript feature and build tooling using the linked setup instructions;
no third-party library is needed for this demonstration.

JavaScript sends an event named `library-result`; HTML listens with `on:library-result`.
Rust receives the event and reads its `detail` payload. The event name selects the listener,
and `detail` carries the data. There is no npm package or generated function named
`library-result`.

```text title=Text · One name, three connected steps
JavaScript: new CustomEvent("library-result", { detail: true })
                          ↓ dispatch on main
HTML:       <main on:library-result="state.accept_result(event)">
                          ↓ call the Rust method
Rust:       event_detail::<bool>(&event) → true
```

> Keep the Rust-only example above if you do not need JavaScript integration yet.

- [Enable JavaScript in this app](/docs/npm#existing-app)
- [How Fusor calls onMount](/docs/npm#mount-context)

## Rust: read the custom payload {#custom-rust}

Add `#[derive(fusor::JsInputs)]` immediately above `struct App` in `src/app.rs`. This gives
the JavaScript module its input interface. This example sends data only from JavaScript to
Rust, so neither Rust field needs the `#[js]` annotation used to expose a value to
JavaScript.

Add the method below inside the existing `impl App`.

```rust title=Rust · Add inside impl App
fn accept_result(&self, event: web_sys::Event) {
    if let Ok(value) = fusor::js::event_detail::<bool>(&event) {
        self.last_event.set(format!("Library result: {value}"));
    }
}
```

> event\_detail::`<bool>` requests a boolean and returns a Result. `if let Ok(value)` uses a
> valid payload; this example ignores invalid ones. JavaScript true is a boolean; the string
> "true" is not. Other supported payloads include `String`, numeric types, and `Option`/Vec
> combinations; arbitrary Rust structs are not decoded automatically.

- [Supported value types](/docs/npm#inputs)

## HTML: listen on the dispatch element {#custom-html}

Replace `web/index.html` with this version. The script belongs directly inside `<App>`. Its
native root is main, so JavaScript receives main as root.

Listening on that same main connects its `library-result` event to the new Rust method.

```html title=HTML · Replacement web/index.html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Events in Fusor</title>
  <link rel="icon" href="data:,">
</head>
<body>
  <App state="{{ App::new() }}">
    <script type="module" src="./app.js"></script>
    <main on:library-result="state.accept_result(event)">
      <button type="button" on:click="state.clicked(event)">
        <span>Add one</span>
      </button>
      <button type="button" data-send-result>Send JavaScript result</button>
      <p>Count: <output>{{ state.count.get() }}</output></p>
      <p>Last event: <output>{{ state.last_event.get() }}</output></p>
    </main>
  </App>
</body>
</html>
```

> The original click button still works. The second button will send a custom event from
> JavaScript; neither event changes the other’s name or payload.

## JavaScript: dispatch the result {#custom-javascript}

Create `web/app.js` with this code. fusor calls `onMount` with this component’s root and
lifetime `AbortSignal`. Click Send JavaScript result: Last event becomes Library result:
true.

In a real integration, put the same `dispatchEvent` call in your library’s result callback.

```javascript title=JavaScript · web/app.js
export function onMount({ root, signal }) {
  const button = root.querySelector("[data-send-result]");
  if (!button) throw new Error("Missing result button");

  button.addEventListener("click", () => {
    root.dispatchEvent(new CustomEvent("library-result", {
      detail: true,
    }));
  }, { signal });
}
```

> `{ signal }` removes this manually added click listener when the component is removed. The
> Rust `on:library-result` handler is managed by fusor. You do not need `#[js]` fields or
> input subscriptions merely to send an event back to Rust.

- [Types of the onMount context properties](/docs/npm#mount-types)
- [Use a library callback to report a result](/docs/npm#events)

## When an event needs to travel to a parent {#bubbling}

Dispatching on root reaches a listener on root directly. If JavaScript dispatches on a child
but HTML listens on a parent, set `bubbles: true` for the custom event.

Standard click events already bubble. `CustomEvent` defaults to not bubbling.

```javascript title=JavaScript · Alternative dispatch inside the click callback
button.dispatchEvent(new CustomEvent("library-result", {
  detail: true,
  bubbles: true,
}));
```

> Use this in place of `root.dispatchEvent` in the example to send from the button instead.
> Events travel through DOM ancestors, not unrelated components. For events originating
> inside a Web `Component`’s shadow tree, crossing its shadow boundary also requires
> `composed: true`; check the library’s documented event behavior.

## Default actions and propagation are separate {#default-behavior}

`prevent_default()` cancels a cancelable browser action, such as following a link.
`stop_propagation()` stops further travel through the DOM; it does not cancel the action.

Returning false from a Rust expression is not a cancellation API. Use the event methods
explicitly.

```html title=HTML · Explicit default-action cancellation
<!-- Add inside main to try this with the existing state. -->
<a href="#example-destination"
   on:click="{ event.prevent_default(); state.last_event.set(String::from(&quot;Link handled&quot;)); }">
  Handle this link in Rust
</a>
```

> Clicking this link changes Last event without navigating to the hash. &amp;quot; is an
> HTML escape for a double quote in the Rust expression. The current `on:` API does not
> expose capture/once/passive options or modifier suffixes; do not invent attributes such as
> on:`click.prevent`.

## Listener lifetime and common mistakes {#lifetime}

fusor removes `on:` listeners when their owning component is disposed. Adding a handler does
not require manual listener cleanup.

For listeners you add yourself in JavaScript, use the component’s `AbortSignal` or
`onCleanup`. If a handler does not run, check the exact event name, the element where it is
dispatched, and whether it must bubble to reach the listener.

> If Rust receives the event but ignores it, check the requested payload type. If it fires
> twice, check for listeners on both the child and an ancestor or duplicate manual
> registration. A native custom element uses its own documented event names; a Rust
> component can place its listener on an actual element in its template.

- [JavaScript cleanup](/docs/npm#cleanup)
- [Component ownership and lifetime](/docs/ownership)
- [Event attribute reference](/docs/html-and-rust/attributes#events)
