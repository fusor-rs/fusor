# Template attribute reference

Look up attributes and expression syntax used on HTML elements, component declarations, and
component tags. Built-in tags have their own reference.

## How to use this reference {#start}

This page covers attributes such as `rust:component`, `on:click`, and `hydrate`, plus
`{{ expression }}` interpolation. For `<App>`, `<ForEach>`, `<Await>`, and the other
compiler-provided tags, use the Built-in components reference.

Values in `rust:*` attributes are Rust expressions unless an entry says they are static.
Write those expressions without `{{ }}`. Interpolation in native text and attributes uses
`{{ expression }}`. Follow each entry’s guide link for complete working files.

A component input uses `{{ expression }}` for a Rust value, or a plain attribute string when
that input accepts one. Built-in tags document their own inputs on the separate reference
page.

> Examples below are syntax fragments, not standalone apps. Their state fields, types, and
> imports must exist in your Rust module. Follow the guide links for complete working files.

- [Built-in components reference](/docs/html-and-rust/built-in-components)
- [Connect a JavaScript module or npm package](/docs/npm)

## rust:component — associate HTML with a Rust type {#component}

Static Rust type path on a component declaration. On `<template>`, it defines reusable HTML
with exactly one native root element. On an ordinary element, it binds that existing root.

The type must be in the Rust module associated with the HTML; `template!(...)` supplies that
association.

A Rust component library can call `compile_app()` with its metadata entry pointing to a file
containing only named component templates and Rust scripts. Its browser code embeds those
templates, including discovered component files, so the consuming application imports the
Rust types without copying the library’s HTML into its own document. Ordinary document
entries keep their document templates and validation.

```html title=HTML
<template rust:component="Counter">
  <section>{{ state.count.get() }}</section>
</template>
```

> Application startup uses `<App state="{{ ... }}">` instead. Do not put `rust:component` on
> its native root.

- [Build Counter with both Rust and HTML](/docs/components)

## rust:module — associate an external Rust script {#module}

Static absolute Rust module path on `<script type="text/rust" src="...">`. `src` is relative
to the HTML file. Declare that module in Rust and place `bindings!(registered_name)` in it.
This is the explicit registration authoring style; script-free HTML uses `template!(...)`
instead.

```html title=HTML
<script type="text/rust" src="../src/app.rs"
        rust:module="crate::app"></script>
```

- [See the two file-connection styles](/docs/explicit-composition#authoring-styles)

## rust:if — include or remove an owned child {#if}

A boolean expression on a component tag. False removes the child and disposes its owner.
True mounts a fresh instance. It is not a general conditional attribute for ordinary HTML
elements.

```html title=HTML
<Watch lifecycle="{{ state.lifecycle.clone() }}" rust:if="state.watching.get()"></Watch>
```

> `hidden="{{ condition }}"` changes visibility but keeps the component and its work alive.
> Use `<If>` / `<Else>` for inline conditional HTML or `<Match>` / `<Case>` for Rust
> patterns.

- [Observe mounting and cleanup](/docs/ownership#mount)
- [Render conditional inline HTML](/docs/control-flow)

## rust:key — choose component identity {#key}

On a component tag, a changed key replaces the child; an unchanged key retains it. For
repeated HTML, use the key input on `<ForEach>` instead.

```html title=HTML
<Details id="{{ state.selected_id.get() }}" rust:key="state.selected_id.get()"></Details>
```

> A key controls replacement, not live input delivery. A changing key reruns construction;
> sharing a signal lets one retained child follow changes.

- [Mounts, replacement, and keyed rows](/docs/explicit-composition)

## rust:render — choose native or shared rendering {#render}

The static value `server` generates native HTML rendering. The value `shared` supports both
the native and browser delivery paths. Omit this attribute for a browser-only component.

This requires the server or islands build setup. Adding the attribute alone does not deploy
a server or create a browser bundle.

```html title=HTML
<template rust:component="CartView" rust:render="shared">
  <section>…</section>
</template>
```

> Component tags support server/shared rendering and coherent regions. Nested component
> types must implement the corresponding rendering contract. Use `<Children>` to place
> caller-supplied HTML.

- [Server rendering and selective activation](/docs/islands)

## hydrate — HTML now, interactivity later {#island}

Put `hydrate` on a delivery component tag inside a page rendered by native Rust. Its HTML is
available immediately; `hydrate` chooses when its browser code starts.

The tag names a Rust descriptor implementing `Island`. Registration links that descriptor to
a native view and a browser bundle. In this example, the descriptor is `Cart`; named tag
inputs populate its `Cart::Props` struct. Plain attribute text supplies strings, while
`{{ ... }}` passes Rust values.

The compiler checks missing, extra, and mistyped inputs. Follow Island setup to configure
the descriptor and delivery build.

```html title=HTML
<Cart hydrate="visible"
      product_id="{{ 42 }}" title="Your cart" quantity="1"></Cart>
```

> This renders a div boundary containing the registered view. Leave the tag empty: its
> renderer supplies the children. Inputs are serialized initial values, not live signals.
> Without `hydrate`, a component tag uses the ordinary component mounting API; it does not
> create a delivery boundary.

- [Complete island descriptor and delivery setup](/docs/islands/setup)

## hydrate:id — optionally name an instance {#activate}

fusor generates an instance ID when none is supplied. Use a nonempty static `hydrate:id`
when a button or active Rust code needs to find this particular instance. Two instances of
the same component can have different IDs and state.

An ordinary `id` attribute is a component input, not the boundary ID.

```html title=HTML
<Cart hydrate="visible" hydrate:id="cart-42"
      product_id="{{ 42 }}" title="Your cart" quantity="1"></Cart>
```

> Hydration policies are `load`, `visible`, `idle`, `interaction`, and `manual`. The
> `interaction` policy requires `hydrate:id` so an explicit target button can identify the
> instance. The Islands guide explains when each policy starts the browser code.

- [Choose an activation policy](/docs/islands)

## hydrate:prefetch — optionally download code earlier {#prefetch}

On a hydrated component, `none` (the default), `load`, `visible`, or `idle` schedules code
downloads independently of hydration. Downloading alone does not construct state, initialize
the unit, or start application reads.

```html title=HTML
<Cart hydrate="visible" hydrate:prefetch="idle"
      product_id="{{ 42 }}" title="Your cart" quantity="1"></Cart>
```

- [Delivery and activation behavior](/docs/islands)

## hydrate:target — hydrate from a button {#activate-target}

Put a static instance ID on a native button with `type="button"`. It must match the target
component’s `hydrate:id`. The loader handles mouse and keyboard activation before that
component’s Rust runs. This is an explicit request, not replay of arbitrary clicks or form
submissions.

```html title=HTML
<button type="button" hydrate:target="designer-42">Open designer</button>
<Designer hydrate="interaction" hydrate:id="designer-42"
          product_id="{{ 42 }}" title="Design a product"></Designer>
```

> Use `hydrate="interaction"` on the target. A later deliberate click retries failed
> activation.

- [Native content and activation buttons](/docs/islands#native)

## prop:name — pass a value to a Web Component {#properties}

Use an ordinary Rust expression on a custom HTML tag to assign a JavaScript property. Values
implement `Into<JsValue>`; strings, numbers, booleans and JavaScript object handles work
directly. Property names preserve their exact case.

Values wait for activation and custom-element definition; updates use the latest value and
skip `Object.is`-equal values. Attribute interpolation continues to produce strings.

```html title=HTML
<sl-animation prop:keyframes="state.frames.get()"
  prop:duration="80.0" prop:play="state.play.get()"
  on:sl-finish="state.play.set(false)">
  <div>Animated content</div>
</sl-animation>
```

> This property-only excerpt assumes frames: `Signal<JsValue>` and play: `Signal<bool>`. The
> native JavaScript guide explains module registration and the complete repository npm
> example supplies every state field. Properties are browser-only; custom elements remain
> unsupported inside coherent boundaries.

- [Native JavaScript and Web Components](/docs/npm#web-components)

## on:event — run a Rust event handler {#events}

An `on:` attribute runs its Rust expression when the named DOM event occurs. The expression
receives `state`, the component value, and `event`, a `web_sys::Event`.

For event-specific fields, use the appropriate `web_sys` event type with `JsCast`. Signal
writes in the handler update dependent bindings.

```html title=HTML
<button on:click="state.count.update(|n| *n += 1)">Add one</button>
```

- [Expressions, events, and Rust closure syntax](/docs/html-and-rust)
- [Events: browser handlers and custom messages](/docs/events)

## class:name — toggle one CSS class {#classes}

A boolean expression controlling one CSS class. Combine multiple `class:name` bindings with
a static class attribute. Do not combine `class:name` with interpolation of the entire class
attribute; use a static base class and a separate binding for each conditional class.

```html title=HTML
<p class="status" class:busy="state.loading.get()">Status</p>
```

- [Bind native HTML](/docs/html-and-rust)

## bind — connect a form control {#bindings}

`bind` keeps a form control and a Rust value in sync in both directions. Supply the signal
handle, not `signal.get()`. The markup decides what the control edits and the Rust type
decides what the edit means, so `rustc` checks the pairing at this attribute.

Text-like inputs, `<textarea>`, `number`, `range`, date and color inputs, and a single
`<select>` edit their value as text. Bind a `Signal<T>` where
`T: FromStr + Display + PartialEq`, such as `Signal<String>`, `Signal<u32>` or
`Signal<f64>`. Text that does not parse, such as a half-typed number, leaves the signal
unchanged. A `fusor-std` `TextField` also works; it keeps invalid drafts and validates them.

A checkbox follows a `Signal<bool>`, or adds and removes its `value` in a `Signal<Vec<T>>`.
A radio button needs a `value`: radios bound to the same signal act as a group, and the
signal holds the checked radio's value. A `<select multiple>` binds a `Signal<Vec<T>>` of
the selected options' values.

```html title=HTML
<input type="text" bind="state.name">
<input type="number" min="1" bind="state.seats">
<input type="checkbox" bind="state.enabled">
<label><input type="radio" value="small" bind="state.size"> Small</label>
<label><input type="radio" value="large" bind="state.size"> Large</label>
<select bind="state.size">
  <option value="small">Small</option>
  <option value="large">Large</option>
</select>
<textarea bind="state.notes"></textarea>
```

> `bind` owns the control's value, so leave out `value` and `checked` attributes, `selected`
> options and textarea contents; a checkbox or radio keeps its `value`, which may be
> interpolated. The input `type` must be static. File inputs are unsupported; read their
> files in an `on:change` handler.

- [Input binding examples](/docs/html-and-rust)

## {{ expression }} — display a Rust value {#interpolation}

Use in text or ordinary HTML attribute values. The expression is checked by `rustc` and its
signal reads are tracked. It inserts text or an attribute value, not raw executable HTML.
Boolean attributes such as `hidden` use boolean presence semantics.

```html title=HTML
<p hidden="{{ !state.visible.get() }}">Hello {{ state.name.get() }}</p>
```

- [Learn HTML interpolation](/docs/html-and-rust)

## data-fusor-link — navigate through the router {#link}

Opt a same-origin app link into the browser router. Give it a real `href` so normal link
behavior remains available. External links, downloads, targets, and modified clicks keep
native browser behavior.

```html title=HTML
<a data-fusor-link href="/articles/42">Read article</a>
```

- [Routing and link behavior](/docs/routing)
