# Built-in components

Compiler-provided tags for application startup, lists, conditions, nested content, async
views, and routing. Learn their inputs and what HTML they render.

## Built-in tags and your own components {#start}

Built-in tags are understood by the HTML compiler. They need no per-tag Rust import, struct,
or template registration. The HTML file containing them still needs its normal association
with a Rust module, such as `template!("web/index.html")`. Use their exact names and casing.
Generated applications already include the `fusor-components` dependency; individual
features may also need the async or router crate described in their guide.

These tags control the HTML inside them without adding wrapper elements. Some require a
single native root or a dedicated container; check the entry below before nesting them.

Your own tags, such as `<Counter>`, resolve to Rust types imported in the template’s
associated module. They use `FromInputs` to construct state and `rust:component` to select
reusable HTML. They are separate from the compiler-provided tags listed here.

Expression inputs such as `condition`, `items`, and `value` use `{{ expression }}`. Static
inputs such as `let` and `pattern` do not. Custom component inputs follow the same
distinction: a Rust expression uses braces, while a plain attribute supplies a static
string.

> The examples are syntax fragments. Follow their guide links for complete Rust state,
> imports, HTML files, and setup.

- [Build your own reusable component](/docs/components)
- [Template attribute reference](/docs/html-and-rust/attributes)

## App — start and own the application {#app}

Use one `<App>` in the entry HTML. Its required `state` input contains exactly one
`{{ Rust expression }}`. The returned value becomes `state` in the child HTML. The
expression may use `owner`, the application’s `OwnerHandle`.

`<App>` emits no DOM element. It requires exactly one native child element, which becomes
the application root.

```html title=HTML
<App state="{{ App::new() }}">
  <main>…</main>
</App>
```

> Put HTML attributes and bindings on the native root, not on `<App>`. A fallible
> constructor uses `?`. This tag is for browser startup and cannot be nested in a component
> or combined with a manual Wasm start function.

- [Read the full App guide](/docs/html-and-rust/app)

## If and Else — choose a boolean branch {#if}

`<If>` evaluates its required `condition` expression as a Rust `bool`. It mounts the child
HTML when true, or the optional final `<Else>` when false. Without `<Else>`, a false
condition renders nothing.

Place these tags inside a native HTML container. A selected branch may contain several
elements and components. Switching branches disposes the old branch; changing other state
while the same branch remains selected preserves its component instances.

```html title=HTML
<If condition="{{ state.visible.get() }}">
  <p>Visible</p>
  <Else><p>Hidden</p></Else>
</If>
```

> `<Else>` must be a direct, final child of `<If>`. Inactive branches do not mount
> components or start their owned work. Use the native `hidden` attribute when you want to
> keep a view alive but invisible.

- [Conditions and pattern matching](/docs/control-flow)

## Match and Case — select a Rust pattern {#match}

`<Match>` reads its required `value` expression and selects the first matching `<Case>`.
Each `pattern` contains an ordinary Rust pattern without `{{ }}`. Rust checks that the cases
cover every possible value.

Captured values become read-only `Memo<T>` handles inside that case. Read a capture with
`.get()` or share its reactive handle with `.clone()`. An update selecting the same case
preserves its DOM and local component state; selecting a different case disposes the old
one.

Patterns may destructure enum variants, structs, and tuples, or use literals, ranges, `_`,
and or-patterns. Or-patterns must bind the same names and types on both sides. Captures must
implement `Clone + PartialEq + 'static`. Match guards, `ref`/`mut` captures, reference
patterns, and pattern macros are not supported in this release.

Use `snake_case` for captures. Qualify application variants and constants where possible.
Imported capitalized variants such as `None` also work; lowercase constants need a qualified
path so they are not parsed as captures.

```html title=html
<Match value="{{ state.selected.get() }}">
  <Case pattern="Some(title)">
    <h2>{{ title.get() }}</h2>
  </Case>
  <Case pattern="None">
    <p>Choose an item.</p>
  </Case>
</Match>
```

> Here `state.selected` is `Signal<Option<String>>`, so `title` is `Memo<String>`. `<Case>`
> must be a direct child of `<Match>`. The guide explains supported patterns, capture
> naming, nesting, and HTML context restrictions.

- [Complete conditions and pattern matching guide](/docs/control-flow)
- [Supported Rust patterns](/docs/control-flow#patterns)

## ForEach — repeat inline HTML {#each}

`<ForEach>` repeats its child HTML for each value in its required `items` expression. That
expression returns `Vec<T>`. The required `key` expression is a Rust closure receiving `&T`
and returning a stable, unique identity.

Inside each row, `item` and `index` are read-only reactive values. No separate Rust row
struct is required. Place `<ForEach>` in its own native container, with one native row root
or reusable component root.

```html title=HTML
<ul>
  <ForEach items="{{ state.items.get() }}" key="{{ |item| item.id }}">
    <li>{{ index.get() + 1 }}. {{ item.get().title }}</li>
  </ForEach>
</ul>
```

> Use stable, unique keys from the data. `rust:key` is a separate attribute for replacing a
> single custom component instance.

- [Build your first inline list](/docs/components/for-each)

## ForEach item and index bindings {#row}

Inside a row, the framework supplies `item: Memo<T>` and `index: Memo<usize>`. Read them
with `.get()`. Use `item="task"` and `index="position"` on `<ForEach>` to choose different
local names.

The key closure’s parameter is a separate ordinary Rust argument. It receives `&T`, not a
`Memo<T>`.

```html title=HTML
<ForEach items="{{ state.items.get() }}" key="{{ |task| task.id }}"
         item="task" index="position">
  <li>{{ position.get() + 1 }}. {{ task.get().title }}</li>
</ForEach>
```

> Place this fragment inside its own native container, such as `<ul>`. The reactive `index`
> starts at zero and changes on reorder; it is not the row’s identity.

- [Understand these local names](/docs/components/for-each#scope)

## Children — place nested HTML {#children}

Inside a reusable component’s native root, `<Children></Children>` places the HTML supplied
between that component’s opening and closing tags.

Bindings use the caller’s state. There is no wrapper element, import, or input field.

```html title=HTML
<template rust:component="Panel">
  <section><Children></Children></section>
</template>

<!-- In the caller’s template -->
<Panel><p>Hello {{ state.name.get() }}</p></Panel>
```

Use `<Children name="footer"></Children>` to place a named fragment supplied by
`<template slot="footer">…</template>` directly inside the component invocation:

```html title=HTML
<Panel>
  <p>Body content</p>
  <template slot="footer"><button>Close</button></template>
</Panel>
```

Ordinary children fill the unnamed slot. Named fragments can contain text and several
sibling elements; the template adds no wrapper. Names are case-sensitive, nonempty literals
containing ASCII letters, digits, `_` or `-`.

> Use one placement per slot, including forwarding, and no fallback body. Missing slots
> render nothing; unused slots never mount. The same slot may appear in mutually exclusive
> branches. Duplicate supplied names are rejected.

- [Complete Panel and caller files](/docs/nested-content)

## Async — publish a complete view {#async}

A built-in component that coordinates descendant `<Await>` reads and generated bindings.
`<Async>` creates and owns its boundary automatically. Supply `boundary="{{ state.view }}"`
only when you need a Rust handle for status or retry controls.

Put exactly one native HTML root inside; `<Async>` emits no wrapper element.

```html title=HTML
<Async>
  <section>
    <h3>Product {{ state.product.get() }}</h3>
    <Price product="{{ state.product.clone() }}"></Price>
    <Stock product="{{ state.product.clone() }}"></Stock>
  </section>
</Async>
```

> While new data loads, the previous complete view stays visible and noninteractive. Keep
> selection, status, and retry controls outside the region.

- [Two reads, one complete view](/docs/coherent-async)

## Await — display an async result {#await}

The required `value` input takes an `AsyncValue` declared with `browser::read`. The required
`let` attribute chooses the local name for its successful result.

Outside `<Async>`, this view updates independently. Inside `<Async>`, its read joins the
enclosing group, including across nested components. `<Await>` emits no wrapper and requires
one native HTML root.

```html title=HTML
<Await value="{{ state.quote }}" let="price">
  <p>Price: {{ price.as_str() }}</p>
</Await>
```

> Here `price` is `Rc<T>` and is available only inside this `<Await>`. This is not Rust
> `.await`; it does not accept arbitrary futures or `Resource`. Pending or failed reads
> preserve the previous successful view. Before the first success, authored static
> placeholders remain.

- [See the read and its complete HTML](/docs/coherent-async#await)

## Router and Route — select a page {#router}

`<Router>` selects among its direct `<Route>` children. A route’s `path` matches an
application-relative URL path. A segment such as `:slug` captures a decoded `String`, and
`let` names the local object holding those captures.

A final `/*` delegates the remaining path to nested routers. A route with `fallback` handles
paths unmatched by its siblings. Only the selected route’s HTML is mounted; leaving it
disposes its owned work. Neither tag creates a DOM wrapper.

Add `fusor-router` with its `browser` feature. The routing guide shows complete setup and
nesting across component files.

```html title=HTML · Article is an application component
<Router>
  <Route path="/articles/:slug" let="params">
    <Article slug="{{ params.slug }}"></Article>
  </Route>
  <Route fallback><p>Page not found</p></Route>
</Router>
```

- [Complete routing and nested-router guide](/docs/routing)

## Custom component tags — where they differ {#mount}

A custom tag such as `<Details>` resolves to an imported Rust type, unlike the reserved
built-in names above. Its named inputs construct the type’s input struct.

The framework creates a child owner, calls `FromInputs::from_inputs(inputs, owner)`, and
inserts the HTML associated with that Rust type. No wrapper element is emitted. Construction
runs once per identity; pass a `Signal<T>` or `Memo<T>` when a retained child should follow
changes.

```html title=HTML
<Details id="{{ state.selected_id.get() }}"></Details>
```

> In this fragment, `DetailsInputs` has an `id: u32` field, and
> `<template rust:component="Details">` supplies the rendered HTML. The linked guide
> provides both files. Use `rust:key` to replace an instance when a snapshot input changes.

- [See the Details HTML](/docs/explicit-composition#details-template)
- [What is owner, and where does it come from?](/docs/ownership/mounting#owner-variable)
