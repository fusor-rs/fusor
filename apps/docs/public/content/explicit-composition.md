# Component identity

Show, hide, and replace components while understanding which local state survives.

## Open the working companion {#run}

The remaining examples use the checked-in companion, which deliberately keeps the supported
external-script/explicit-registration API.

To run the companion, clone https://github.com/fusor-rs/fusor and run this command from the
repository root. Open http://127.0.0.1:8091/. Paths below are relative to
`apps/docs/tutorial/`.

```sh title=Terminal
# From the fusor repository root, after Installation:
fusor dev --manifest-path apps/docs/tutorial/Cargo.toml --port 8091
```

- [Complete companion App state](/docs/source/tutorial-app.rs.txt)
- [Complete companion HTML](/docs/source/tutorial-index.html.txt)
- [Companion manifest and registration](/docs/source/tutorial-Cargo.toml.txt)

## Connect a tag to its state and HTML {#declare}

The component guide introduced named inputs. This companion uses the same tag API with
explicit HTML file registration. Its `Details` component takes a snapshot id and owns an
editable note. You can control whether it exists and when a fresh instance replaces it. Run
the companion separately from your generated `my-app`.

```text title=Diagram · connections, not Rust syntax
<Details id="{{ state.selected_id.get() }}"></Details>
  → DetailsInputs { id }, passed to FromInputs
  → Details::new(inputs.id), defined in src/details.rs
  → template rust:component="Details" in web/details.html
  → <section class="details">…</section> at the tag’s position
```

> The imported Rust type selects the template; no filename is guessed. A native wrapper is
> optional: `<div class="layout"><Details id="{{ 1 }}"></Details></div>`.

## Understand the companion’s file connections {#authoring-styles}

The counter lesson connects script-free HTML with `template!(path)` in Rust. This companion
uses the older explicit-registration style.

A components entry in `Cargo.toml` gives an HTML file a registration key. Its `text/rust`
script is a build-time link: src points to the Rust file relative to the HTML file, and
`rust:module` gives the Rust module path.

`bindings!(details)` includes bindings for the details registration key in that module. The
`lib.rs` include below adds the generated registration checks.

These links are already present in the companion; the browser does not execute the Rust
script.

```rust title=src/lib.rs · complete companion file
mod app;
mod details;
mod pricing;
mod reader;
mod rows;
mod watch;

include!(env!("FUSOR_MODULE"));
```

> HTML file registration is independent of composition. Both `template!` and explicit script
> registration use the same component tags.

## Define the child’s state {#details-state}

`Details` receives an ordinary `u32` — an unsigned 32-bit integer with no signal wrapper, so
it is a snapshot — and creates its own editable note.

This component needs no lifecycle handle, so its constructor takes only the id. Later guides
pass an owner when a component creates owned work.

`pub` makes the type and constructor usable by the parent module; the template can read the
private fields because its generated bindings are included in this module.

```rust source=tutorial/src/details.rs title=src/details.rs · complete file
```

## Write the elements it will insert {#details-template}

This is the HTML for Details. Selecting issue 1 produces a heading “Issue 1” and a Local
note input. The file’s script resolves to `src/details.rs` relative to `web/details.html`.
Each mounted instance has its own section and note signal.

```html source=tutorial/web/details.html title=web/details.html · complete file
```

## Register files when using the older API {#registration}

The companion registers external-script HTML explicitly, declares its normal Rust modules,
and keeps `include!(env!("FUSOR_MODULE"))` in `lib.rs` for registration checks.

New script-free templates under `web/components/` use `template!(path)` instead and need no
per-file Cargo entry. The Rust import makes `Details` available to the parent in either
mode.

```toml title=Cargo.toml · actual companion component table
[package.metadata.fusor.components]
details = "web/details.html"
rows = "web/rows.html"
watch = "web/watch.html"
reader = "web/reader.html"
pricing = "web/pricing.html"
```

> These are all five entries from the companion. The full manifest and module declarations
> are linked below.

- [Companion manifest](/docs/source/tutorial-Cargo.toml.txt)
- [Companion module declarations](/docs/source/tutorial-lib.rs.txt)

## Mount, hide, and replace an instance {#mount}

`App` has expanded: `Signal<bool>` initialized to true and selected\_id: `Signal<u32>`
initialized to 1. This markup goes inside `<App>`’s root.

`rust:if` controls whether `Details` exists; `rust:key` controls which instance is retained.
With the same key, `Details` keeps its note. A new key constructs a new `Details`, and false
removes it and disposes its owned work.

```html title=web/index.html · inside App
<button on:click="state.expanded.update(|v| *v = !*v)">Toggle details</button>
<button on:click="state.selected_id.set(2)">Select issue 2</button>
<Details id="{{ state.selected_id.get() }}" rust:if="state.expanded.get()"
     rust:key="state.selected_id.get()"></Details>
```

> Try it: type a note, choose issue 2, and observe the new heading and empty note. Hiding
> then showing also creates a new instance. Hiding with the HTML `hidden` attribute or CSS
> alone would leave the child mounted.

## Decide whether an input should stay reactive {#inputs}

`Details` stores an id snapshot. Its key intentionally replaces the whole instance when the
id changes.

If a child should keep local state while following a changing parent value, accept a
`Signal<T>` and pass a cloned handle instead. The generated app’s `Counter` does exactly
that. A constructor is not rerun for every reactive DOM update; pass a signal for values a
retained child needs to observe.

- [See shared and local Counter state](/docs/project-structure#counter-state)
- [Retain a reader while its request key changes](/docs/async-data#mount)

## Use a component when a row needs local state {#lists}

`<ForEach>` usually repeats inline HTML directly. The companion instead reuses a `Row`
component because each row owns a local note signal.

The list provides `item: Memo<Item>`, a read-only reactive view of that row’s current value.
Pass the handle to `Row` through its item input.

```html title=web/index.html · inside App
<ul>
  <ForEach items="{{ state.items.get() }}" key="{{ |item| item.id }}">
    <Row item="{{ item.clone() }}"></Row>
  </ForEach>
</ul>
```

> The key closure receives `&Item`. The child HTML receives `Memo<Item>`; `.clone()` shares
> that reactive handle with Row.

- [Start with inline HTML and no row struct](/docs/components/for-each)
- [Try the inline reading list](/docs/showcase/keyed)

## Define Item and Row {#row-state}

`Row` keeps the item `Memo` supplied through its input, plus an independent local note.
`FromInputs` connects the HTML item input to `Row::new`. A surviving key gets current item
data without rebuilding `Row` or resetting its note. Keeping only `item.get()` would retain
a snapshot instead.

```rust source=tutorial/src/rows.rs title=src/rows.rs · complete file
```

## Choose the root for each row {#row-template}

The li below is what the `Row` component renders. Register rows = "`web/rows.html`", declare
`mod rows` in `lib.rs`, and import `Item` and `Row` in the parent. The companion already
contains these entries.

Template root selection is explicit: a `<ul>` parent does not automatically turn arbitrary
component markup into an `<li>`.

```html source=tutorial/web/rows.html title=web/rows.html · complete file
```

## Observe what a stable key preserves {#list-updates}

Initially the companion renders two li elements labeled First issue and Second issue. Type
different notes into them.

Reverse rows: the same row instances move, keeping their notes. Rename issue 1: its title
changes while its note remains. Remove issue 1: that row and its owned work disappear.

Keys must be unique and stable for an identity; use the data id instead of the current list
index.

```text title=Resulting structure · framework markers omitted
<ul>
  <li><strong>First issue</strong><label>Row note <input></label></li>
  <li><strong>Second issue</strong><label>Row note <input></label></li>
</ul>
```

- [What removal cleans up](/docs/ownership)
