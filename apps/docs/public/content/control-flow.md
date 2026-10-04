# Conditions and pattern matching

Choose which HTML exists with `<If>` and `<Else>`. Use `<Match>` to select Rust enum
variants, extract their data, and render the right component for each state.

## Choose the right tool {#choose}

Use `<If>` for a boolean decision and `<Match>` when your state has several possible shapes,
such as an enum, `Option`, or `Result`. Both select inline HTML: you do not need a separate
component for every branch.

These are built-in tags; no Rust import or registration is required. They create no wrapper
elements.

```html title=HTML
<If condition="{{ state.show_help.get() }}">
  <p>Here is some help.</p>
  <Else><p>Help is closed.</p></Else>
</If>
```

> `<Else>` is optional. It must be the final direct child of `<If>`. Without `<Else>`, a
> false condition renders nothing. The `state.show_help` signal is defined in the complete
> example below.

## Start with a generated application {#start}

Follow Installation to generate an app. Replace `src/app.rs` and `web/index.html`, then
create `web/components/dashboard.html` using the three complete files below. Keep the
generated `src/lib.rs`, `build.rs`, and `Cargo.toml`. The generated app already includes
`fusor-components`.

This example uses component inputs. Read the first two steps of Reusable HTML first if
`#[input]` and `#[local]` are new to you.

```text title=Text
src/
  lib.rs                  # keep the generated file
  app.rs                  # App, Session, User, and Dashboard
web/
  index.html              # chooses which HTML renders
  components/
    dashboard.html        # Dashboard’s reusable HTML
```

- [Generate an application](/docs/installation)
- [Understand component inputs first](/docs/components#typed-inputs)

## 1. Define the states and component input {#state}

`Session` is an enum: the application is either `Guest` or `Authenticated` with a `User`.
`Signal<Session>` stores the current choice. `Clone` lets `.get()` return owned data;
`PartialEq` lets reactive values detect changes.

`Dashboard` receives a `Memo<User>`, a read-only reactive handle, and owns a separate click
counter.

```rust source=tutorial/lessons/control-flow/app.rs title=src/app.rs · complete file
```

> `#[input]` declares a value supplied by the component tag. `#[local(init = ...)]`
> initializes private state when `Dashboard` mounts. Both structs are in the same Rust
> module, so `Dashboard` needs no use import here.

- [Understand component inputs and imports](/docs/components)

## 2. Choose HTML with native Rust patterns {#html}

`<Match>` evaluates its `value` expression and selects the first matching `<Case>`. The
`pattern` attribute contains ordinary Rust pattern syntax, without `{{ }}`.

The pattern `Session::Authenticated { user }` extracts the `user` field and exposes it as a
reactive handle only inside that case. The name `state` still refers to the enclosing `App`.

```html source=tutorial/lessons/control-flow/index.html title=web/index.html · complete file
```

> Rust checks that the cases cover every possible value. Add a variant to `Session` without
> adding a case and the application fails to compile.

## 3. Give Dashboard its HTML {#dashboard}

`Dashboard` refers to the Rust struct in `src/app.rs`. Its `template!` declaration connects
that type to this file. Inside this template, `state` is `Dashboard`, so `state.user` is its
input and `state.clicks` is its private counter.

```html source=tutorial/lessons/control-flow/dashboard.html title=web/components/dashboard.html · complete file
```

## 4. Observe updates and branch lifetime {#try}

Run `fusor dev`. Click Toggle help to show and hide the help text; this exercises the `<If>`
branch.

Sign in, increment the dashboard counter, then press Change name. Both headings change to
Grace and the counter stays the same: the authenticated case retained its DOM and component
state.

Sign out and back in. The dashboard is recreated, and its counter starts at zero.

```sh title=Terminal
fusor dev
```

> A case is identified by its position in `<Match>`, not by the user’s id or the whole
> value. If you want a nested component recreated when an id changes, use `rust:key` on that
> component.

- [Reset a component with a key](/docs/explicit-composition)

## What exactly is user? {#captures}

The pattern binds a `User` in Rust. Inside the `<Case>` HTML, the compiler exposes it as
`Memo<User>`. `user.get()` reads a snapshot and tracks updates. `user.clone()` shares the
reactive handle; it does not copy the current user.

Captures must implement `Clone + PartialEq + 'static`, and cannot borrow temporary values.
This is the same read-only handle used for `<ForEach>` items.

```html title=HTML
<!-- Reactive input: Dashboard stores Memo<User>. -->
<Dashboard user="{{ user.clone() }}"></Dashboard>
```

> Passing `user.get()` to a component that accepts a plain `User` supplies a
> construction-time snapshot. It does not make that field reactive. Use `Memo<User>` when a
> retained component must follow later changes. Update the original `state.session` signal
> to change the data.

- [Signals and read-only derived values](/docs/reactivity)
- [The same contract for list items](/docs/components/for-each#scope)

## Match Option, Result, and other Rust patterns {#patterns}

`<Match>` also accepts destructuring, literals, ranges, wildcards, and or-patterns. Here
`state.selected` is `Signal<Option<String>>`. `Some(title)` exposes `title: Memo<String>`;
`None` renders the empty state.

```html title=HTML
<Match value="{{ state.selected.get() }}">
  <Case pattern="Some(title)"><h2>{{ title.get() }}</h2></Case>
  <Case pattern="None"><p>Choose an article.</p></Case>
</Match>
```

> Use `pattern="_"` for an intentional catch-all. Name every enum variant when adding a new
> variant should trigger a compiler error. Captures use `snake_case` names; qualify a
> lowercase constant with its Rust path so it is not treated as a capture. The built-in
> reference lists the remaining pattern restrictions.

- [Match and Case pattern contract](/docs/html-and-rust/built-in-components#match)

## Only the selected branch runs {#lifecycle}

Inactive branches create no components or subscriptions. Switching branches disposes their
owned effects, listeners, and async work. Returning creates a fresh branch.

Changes that keep the same case update its captured values while preserving local state.
`<If>` uses the same lifecycle: true to false replaces the active branch.

> If you only want to hide something while keeping its state and work alive, bind the native
> `hidden` attribute instead. `<If>` and `<Match>` also work with server/shared templates
> and coherent `<Async>` regions; a coherent branch replacement publishes with its region.

- [Owned work and cleanup](/docs/ownership)
- [Coherent publication](/docs/coherent-async)

## Nest branches without inventing new components {#nesting}

A branch can contain multiple elements, reusable components, another `<If>` or `<Match>`, or
a native container with `<ForEach>`. Put control-flow tags inside the component’s native
root. `<Match>` accepts only direct `<Case>` children.

Use distinct capture names when nesting: a `<Case>` cannot shadow an enclosing capture, list
binding, or framework name such as state or owner.

```html title=HTML
<If condition="{{ state.show_help.get() }}">
  <h2>Help</h2>
  <p>Branches can have more than one element.</p>
</If>
```

> This release requires ordinary HTML containers. Table, select, SVG, and MathML parsing
> contexts are rejected rather than letting the browser move or reinterpret branch markup.
> `<ForEach>` still needs its own native container and row root; place `<If>` or `<Match>`
> inside that row root.
