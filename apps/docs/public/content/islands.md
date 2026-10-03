# Islands

Display HTML first, then start each interactive part when it is needed. Learn where the HTML
comes from, what `hydrate` does, and how island state behaves.

## What is an island? {#start}

An island is a part of a page that already has HTML, but starts its Rust behavior
separately.

Imagine a product page: the description is readable immediately, while a cart becomes
interactive when the visitor reaches it. The cart is the island.

Hydration is the step that starts its browser behavior. You can keep using a normal fusor
SPA without islands.

> Use islands when useful content can appear before its interactive code is needed. A normal
> SPA is still a good fit when most of the screen needs to be interactive together.

- [Ordinary reusable components](/docs/components)

## Where does the HTML come from? {#html}

In our catalog example, Rust runs on your development machine or build service and writes
the initial HTML into `index.html`. You deploy that file together with the JavaScript loader
and Wasm assets.

Static hosting sends those files to visitors; it does not execute your Rust. No running Rust
server is required for this build-time approach.

```text title=Text
BUILD
Rust renderer → index.html + browser assets

VISIT
Static hosting sends index.html → browser displays the page

HYDRATE
Loader obtains the island’s browser code → Rust/Wasm makes it interactive
```

> Some guides call this server rendering because the HTML is produced outside the browser.
> For this example, “HTML generated at build time” is more precise.
>
> The renderer can also be used by a Rust server to generate HTML per request; you would
> integrate that server yourself. The islands build command does not create an HTTP backend.

## Choose when it becomes interactive {#host}

This `Cart` tag renders the registered cart view’s HTML immediately. `hydrate="visible"`
asks the loader to start its browser behavior when its boundary enters the viewport.

product\_id, title, and quantity are the initial inputs; `{{ 42 }}` is a Rust integer
expression, while the quoted text supplies string values.

```html title=HTML · inside the catalog’s native page template
<Cart hydrate="visible"
      product_id="{{ 42 }}" title="Your cart" quantity="1"></Cart>
```

> `Cart` is a Rust type imported by the page. Its registration connects it to `CartView` and
> that view’s HTML file; the setup guide shows every part. Leave this tag empty because the
> registered view supplies its contents. fusor wraps it in a div and generates an instance
> ID.

- [Set up the native renderer and browser bundles](/docs/islands/setup)
- [The HTML rendered by Cart](/docs/source/catalog-views.html.txt)

## Does an island have normal reactivity? {#reactivity}

Yes. Once hydrated, its browser component tree uses the same signals, computed values, event
handlers, reactive lists, and owned cleanup as other fusor components. It can also load data
asynchronously and use coherent async rendering.

Before hydration, there is no running browser Rust state for this island: its text is
readable and native links and forms can work, but Rust handlers and reactive updates have
not started.

```html title=HTML · excerpt from the cart’s shared view
<input name="quantity" bind="state.quantity">
<p>{{ state.quantity.get() }}</p>
```

> In the example, `state.quantity` is a `Signal<String>` created by `CartView::new`. Before
> hydration, typing edits the native input but does not update the paragraph. Hydration
> adopts the edited value; later edits update the signal and paragraph normally. Shared HTML
> needs resolved initial data; browser-only async startup uses the preview path described
> below.

- [CartView state and constructor](/docs/source/catalog-views.rs.txt)
- [Complete cart HTML](/docs/source/catalog-views.html.txt)
- [Async data loading](/docs/async-data)
- [Coherent async rendering](/docs/coherent-async)

## What can islands share? {#boundaries}

Components inside one island can share signals and state normally. Separate island instances
have separate state and ownership.

Their tag inputs are initial serialized values, not live connections:
product\_id="`{{ 42 }}`" sends the value 42, not a shared Rust signal. Updating one island
does not automatically update another.

```text title=Text
Within an island:
Shared signal → its components update

Between islands:
Explicit serialized event → receiving island updates its own state
```

> Separate Wasm bundles do not share Rust memory, signals, `Rc` handles, or closures. Use
> the island event APIs for messages between active islands. An inactive island has no Rust
> listener running; events are not a queued state store. Multiple instances can share a
> downloaded bundle while keeping their own state.

- [Communicating between active islands](/docs/islands/setup#communication)

## Choose one hydration policy {#policies}

Choose `hydrate="load"` to start as soon as the loader registers the instance; "visible"
when it enters the viewport; "idle" when the browser has idle time (with a two-second
timeout); "interaction" when the visitor uses an explicit target button; or "manual" when
active Rust requests it.

These choices control when behavior starts, not whether the initial HTML is displayed.

```html title=HTML · optional early download
<Cart hydrate="visible" hydrate:prefetch="idle"
      product_id="{{ 42 }}" title="Your cart" quantity="1"></Cart>
```

> Usually `hydrate` alone is enough. Add `hydrate:prefetch` only to download code earlier:
> none (the default), load, visible, or idle. Prefetch does not construct state or start
> data reads.
>
> Instances share downloaded code while keeping their own state. Without `hydrate`, a tag
> follows the ordinary component mounting API; it does not become an island.

## Use an explicit activation button {#interaction}

Name the instance with `hydrate:id` and point a native `type="button"` button at it with
`hydrate:target`. The button works with mouse or keyboard before the target’s Rust runs. A
second deliberate click retries failed activation.

These controls belong to the delivery loader; they do not require an already-running `App`.

```html title=HTML · an explicit button and its target island
<button type="button" hydrate:target="designer-42">Open designer</button>
<Designer hydrate="interaction" hydrate:id="designer-42"
          product_id="{{ 42 }}" title="Design a product"></Designer>
```

> `hydrate:id` is optional for load, visible, idle, and manual, and required for
> interaction. It is a static, unique instance name. Use it for a manual instance when
> active Rust needs to look it up. An ordinary id input belongs to your props, not to the
> generated boundary.

## What happens to the visible HTML? {#native}

The default attach mode reuses the existing HTML nodes and connects their Rust behavior.
Bound inputs preserve edits made before hydration. If a visitor is composing text, hydration
waits until composition ends.

Use preview mode when the browser view differs from the initial HTML. The static preview
stays visible while the browser view prepares. Once its initial coherent reads finish, the
prepared view replaces the preview in one step.

> Native links and forms only work when their URLs and endpoints exist. Static hosting can
> serve the HTML, but an action such as submitting an order still needs a backend. `Island`
> hydration does not supply that backend.

- [Connect the native and browser views](/docs/islands/setup#renderer)

## Handle a slow or failed activation {#failure}

A failed initial preview read keeps the native preview and rejects activation. Explicit
retry creates a fresh attempt. Removal or cancellation disposes pending work.

A transient Wasm download can be retried; a module evaluation or initialization failure
requires a corrected build and reload. Keep fallback content meaningful while these
operations are pending.

## Choose meaningful feature boundaries {#delivery-cost}

Separate Wasm units can duplicate runtime support when several are loaded. The catalog
compares separate and grouped builds of the same views.

Prefer independently used features to one unit per button.

This release does not support nested islands, server async streaming, or coherent regions
across units. These are delivery tradeoffs to measure for your app.

> Start with meaningful features such as a cart or product designer. Keep tightly connected
> reactive components inside the same island. Each island can contain ordinary nested
> components; nested independent islands are not supported in this release.

## Build your first island {#next}

The working catalog includes all the files needed to connect the initial HTML to its browser
code. Continue to `Island` setup to build it and follow the Rust types, HTML templates,
registrations, and Cargo configuration. Adding `hydrate` to an arbitrary SPA component alone
does not create a separate browser bundle.

- [Set up the native renderer and browser bundles](/docs/islands/setup)
