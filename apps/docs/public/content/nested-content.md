# Nested content

Let a `Panel` choose the surrounding layout while its caller supplies the HTML inside it.
Continue the working counter app with two new files.

## Continue your counter app {#start}

Complete Reusable HTML first. Keep `src/counter.rs` and `web/components/counter.html`
exactly as shown there.

Add `panel.rs` and `panel.html` below, then replace `lib.rs`, `app.rs`, and `index.html`
with the complete files on this page. `Cargo.toml` and `build.rs` stay as generated.

```text title=Files for this lesson
src/
  lib.rs                 # update
  app.rs                 # update
  counter.rs             # keep
  panel.rs               # add
web/
  index.html             # update
  components/
    counter.html         # keep
    panel.html           # add
```

- [Complete the counter example](/docs/components)

## 1. Define the Panel component {#panel-state}

`Panel` needs no fields for children. This unit struct (a struct with no fields) identifies
the component; `FromInputs` lets the framework construct it from `<Panel>`. The `template!`
call associates it with the complete HTML file shown next.

```rust source=tutorial/lessons/content/panel.rs title=src/panel.rs · complete file
```

## 2. Place the caller’s HTML {#panel-template}

`Panel` renders a section and a heading. `<Children></Children>` is the exact position where
HTML written between `<Panel>` and `</Panel>` appears. It adds no element around that HTML.

`<Children>` is built in: there is no import, Rust field, or constructor parameter to
declare.

```html source=tutorial/lessons/content/panel.html title=web/components/panel.html · complete file
```

## 3. Declare the new module {#modules}

Add `mod panel` while retaining the counter and app modules.

```rust source=tutorial/lessons/content/lib.rs title=src/lib.rs · complete file
```

## 4. Import Panel in the parent {#parent}

`App` still owns the shared count. Import both `Panel` and `Counter` here because both names
are used in `App`’s associated HTML.

```rust source=tutorial/lessons/content/app.rs title=src/app.rs · complete file
```

## 5. Write HTML inside Panel {#explicit-content}

Everything between `<Panel>` and `</Panel>` becomes its children. Here we choose a div to
group two counters and a total; that div is ordinary HTML, not a requirement. You can supply
text and several sibling elements directly.

These expressions still read `App`’s state and imports. `Panel` decides placement, without
taking over their meaning.

```html source=tutorial/lessons/content/index.html title=web/index.html · complete file
```

## 6. Check the rendered panel {#try-it}

In `my-app`, run dev or keep your existing dev terminal open. Visit http://127.0.0.1:8090/.
You should see “Counters inside a panel”, then the “Shared counters” panel heading, two
Increment buttons, and a total.

Click either button: both shared labels and the total update, while only that counter’s
local clicks change. The supplied HTML remains reactive even though `Panel` placed it.

```sh title=Terminal
fusor dev --port 8090
```

## Pass children through another component {#forwarding}

A wrapper can pass its incoming children to Panel. Here `<Children>` means the HTML supplied
to this wrapper by its caller. `Panel` then places that same content in its own `<Children>`
location.

```text title=Wrapper’s template
<template rust:component="Wrapper">
  <aside>
    <Panel><Children></Children></Panel>
  </aside>
</template>
```

> `Wrapper` must import `Panel` in its associated Rust module. `<Children>` requires no
> import.

## What Children does {#rules}

An empty `<Panel></Panel>` supplies nothing. If `Panel` has no `<Children>`, supplied HTML
is unused: its components and owned work never start.

Use one `<Children>` placement per component, including forwarding.

Removing `Panel` cleans up its mounted children and listeners. Updating a shared signal
keeps their local state; changing `Panel`’s `rust:key` creates fresh children.

> `<Children>` works in browser, coherent, server, and shared templates. A reusable
> component can have multiple native HTML roots; put `<Children>` inside one of them.
> `<Children>` itself has no attributes, names,
> or fallback body. Do not repeat the same incoming `<Children>` inside `<ForEach>`; put a
> component with its own children in each row instead.

- [How ownership and cleanup work](/docs/ownership)
- [Component identity](/docs/explicit-composition)
