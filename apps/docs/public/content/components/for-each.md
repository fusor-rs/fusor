# Lists with ForEach

Repeat HTML directly, keep each row’s identity, and read its current position. You do not
need a Rust struct for every row.

## Start with a generated app {#start}

Follow Installation to generate and run an app. Replace `src/app.rs` and `web/index.html`
with the two complete files below. Keep the generated `src/lib.rs`, `build.rs`, and
`Cargo.toml`. Newly generated apps include the `fusor-components` dependency; an existing
workspace app should add `fusor-components` using the same version or local path as its
other fusor crates.

```toml title=Cargo.toml
[dependencies]
fusor-components = { version = "=0.1.4", features = ["browser"] }
```

> Apps created with `fusor new` already include this dependency. `<ForEach>` is a reserved
> built-in HTML tag; it needs no use import or template registration. Your own component
> tags still use normal Rust imports.

- [Generate and run an app](/docs/installation)

## 1. Own the collection in Rust {#state}

`Signal<Vec<Item>>` holds a changing list. `Vec` is Rust’s growable array.

`Item` derives `Clone` so a row can read its value, and `PartialEq` so the runtime can
detect changed values. The id stays with an item when it moves. remove updates the source
collection.

```rust source=tutorial/lessons/foreach/app.rs title=src/app.rs · complete file
```

## 2. Write the row HTML where it belongs {#html}

The ul is the real DOM container. `<ForEach>` repeats its li child once per item and
disappears from the delivered markup.

The key closure `|item| item.id` returns the identity of a borrowed item. Inside li,
`.get()` reads the reactive row data and its position.

```html source=tutorial/lessons/foreach/index.html title=web/index.html · complete file
```

## 3. See what a key preserves {#try-it}

Run `fusor dev`. Type a different note into each row, then press Reverse. The same rows
move, their notes stay with their ids, and their numbers change. Remove deletes only the
chosen row and disposes its owned work. An empty collection renders no rows.

```sh title=Terminal
fusor dev
```

> The note is an ordinary browser input in this example. Use a reusable row component if you
> also need to hold that note in Rust state.

- [Try the reading-list showcase](/docs/showcase/keyed)
- [A reusable row with local Rust state](/docs/explicit-composition#lists)

## Where item, index, and state come from {#scope}

`<ForEach>` supplies `item: Memo<Item>` and `index: Memo<usize>` inside its child HTML. A
`Memo` is a read-only reactive handle: `.get()` reads it and `.clone()` shares the handle.

`index` starts at zero. The name `state` still refers to the enclosing `App`, so a handler
can call `state.remove(...)`. These names exist in HTML expressions, not as globals in the
Rust module.

```html title=HTML
<ul>
  <ForEach items="{{ state.items.get() }}" key="{{ |item| item.id }}">
    <li>{{ index.get() + 1 }}. {{ item.get().title }}</li>
  </ForEach>
</ul>
```

> The `item` parameter in `|item| item.id` is a separate Rust closure argument of type
> `&Item`; you choose its name. The child HTML’s `item` is supplied by `<ForEach>`. Change
> the source collection to edit row data; `item` and `index` have no `.set()`.

## Choose a stable key {#identity}

Use an id from your data. Keys must be unique within a list and implement Ord + `Clone` +
'static. Duplicate keys are errors.

A surviving key retains its DOM, component state, and lifetime; its item and index update
when the collection changes. A new key creates a row; a removed key disposes it.

> Do not use the current index as an id for reorderable items. The index describes a
> location, not an identity. items must produce `Vec<T>` with T: `Clone` + `PartialEq` +
> 'static.

## Name bindings for nested lists {#nested}

Optional `item` and `index` attributes name the local reactive handles. Here `state.groups`
is your collection of groups, each with id, title, and an items vector.

The inner loop can read both its own `item` and the outer group. Each nested list owns its
own container.

```html title=HTML
<div>
  <ForEach items="{{ state.groups.get() }}" key="{{ |group| group.id }}"
           item="group" index="group_index">
    <section>
      <h2>{{ group.get().title }}</h2>
      <ul>
        <ForEach items="{{ group.get().items }}" key="{{ |item| item.id }}">
          <li>{{ group_index.get() + 1 }}.{{ index.get() + 1 }} {{ item.get().title }}</li>
        </ForEach>
      </ul>
    </section>
  </ForEach>
</div>
```

> The default inner `item`/index names shadow the outer defaults. Use aliases when you need
> both. Names must be distinct Rust identifiers and cannot shadow `state`, `owner`, `event`,
> or `ready`.

## Keep the HTML structure explicit {#boundaries}

`<ForEach>` must be the only meaningful child of its native container, with exactly one
native row root or reusable component root. Put headings, empty-state text, or other static
siblings outside that container.

For table rows, use an explicit tbody container. Conditional components belong inside a
native row root. SVG, MathML, and select controls are not supported in this version.

> Inline rows work with coherent regions and server/shared templates. Pass resolved async
> data through the `items` expression. `rust:key` separately controls the identity of a
> component tag.
