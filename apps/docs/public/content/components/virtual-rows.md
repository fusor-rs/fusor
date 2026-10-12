# Long lists with VirtualRows

Show thousands of rows while the page holds only the ones near the viewport. `VirtualRows`
computes which rows to mount from sizes you declare; `<ForEach>` mounts them.

## When to use it {#start}

`<ForEach>` mounts every item. For a photo library, a long table, or a chat history that
makes thousands of elements, and scrolling and memory follow the size of the data rather
than the size of the screen.

`VirtualRows` keeps a layout of every row from sizes you supply up front, either a fixed
height or a function of the item, so the scroll height is exact without measuring. It
exposes the rows that intersect the viewport, plus an overscan of rows beyond each edge,
and `<ForEach>` mounts only those. Rows scrolled out of view unmount; when they scroll
back in they mount afresh. Keep state that must survive scrolling in signals outside the
row.

It works along one axis. For a grid, make each row an item and lay its cells out inside
the row.

> Needs `fusor-components` with the `browser` feature. Row sizes are declared, never
> measured; a row whose content does not match its declared size overlaps its neighbours.

- [Lists with ForEach](/docs/components/for-each)

## Rust: declare items, sizes, and the scroller {#rust}

`VirtualRows::new` takes the collection as a closure, read like a binding, so changing
the signal it reads recomputes the layout. The second closure returns a row's height in
CSS pixels. `Scroller` names what scrolls: the canvas's parent element or the page.
The last argument is the overscan: how many rows beyond each edge stay mounted.

`scroll_to` moves the scroller so a row starts at the top of the viewport. Each `Day` is
one row holding the photos taken that day; its height follows how many lines they fill.

```rust source=tutorial/lessons/virtual-rows/app.rs title=src/app.rs · complete file
```

> Replace `sample_days` with your own loader. The declared sizes must match the CSS: here
> a 40px heading plus 170px per line of four photos.

## HTML: a canvas, absolute rows, and rust:element {#html}

The canvas is the element that holds the rows. Its `height` comes from `total_size`, so
the scroll bar covers every row, and each row is positioned absolutely from its `offset`
and `size`. `rust:element` hands the canvas to `attach`, which measures the viewport now
and again on every scroll and resize.

The `items` expression reads `visible`, and the key reads the item inside each row. Inside
the row, `row.get()` is a `VirtualRow` with `item`, `index`, `offset`, and `size`.

```html source=tutorial/lessons/virtual-rows/index.html title=web/index.html · complete file
```

> `<ForEach>` owns its container's children, so the canvas holds nothing else. Headings,
> empty-state text, and controls go outside it, as the inner photo list sits in its own
> `<div>` beside the heading.

- [rust:element reference](/docs/html-and-rust/attributes#element)
- [ForEach reference](/docs/html-and-rust/built-in-components#each)

## Scroll inside an element instead of the page {#container}

With `Scroller::Container`, the canvas's parent element scrolls. Give that parent a fixed
height and `overflow: auto`, and make the canvas its direct child. `scroll_to` then sets the
parent's scroll position.

```html title=HTML
<div class="scroller">
  <div class="canvas" rust:element="state.rows.attach(element)"
       style="height: {{ state.rows.total_size() }}px">
    <ForEach items="{{ state.rows.visible() }}" key="{{ |row| row.item.id }}" item="row">
      <div class="line" style="top: {{ row.get().offset }}px; height: {{ row.get().size }}px">
        {{ row.get().item.text }}
      </div>
    </ForEach>
  </div>
</div>
```

```css title=CSS
.scroller { height: 100vh; overflow: auto; }
```

> Before `attach` runs, and on the server, the viewport is empty and only the overscan rows
> are produced. `rust:element` cannot sit inside a coherent `rust:async` region, so keep a
> virtualized list outside one.
