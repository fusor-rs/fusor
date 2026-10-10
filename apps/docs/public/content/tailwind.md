# Tailwind CSS

Style your pages with Tailwind's utility classes, such as `rounded-lg`, `px-4` or
`md:text-xl`. fusor runs Tailwind for you on every build and links the result into your page.
You do not need Node or npm.

## Turn it on {#setup}

Create a stylesheet that imports Tailwind:

```css title=web/app.css
@import "tailwindcss";
```

Point your application at it in `Cargo.toml`:

```toml title=Cargo.toml
[package.metadata.fusor]
tailwind = "web/app.css"
```

Then run `fusor install`, or start `fusor dev`, once. Either one downloads Tailwind's
standalone program (80–110 MB, depending on your system) into fusor's tool cache. `fusor build`
never downloads anything, so prepare a CI machine with `fusor install` first.

fusor adds the `<link>` for the compiled stylesheet to your page. Don't write one yourself.

- [What fusor install prepares](/docs/cli#install)

## Use classes in HTML and Rust {#classes}

Tailwind reads the files in your package and generates CSS for every class name it finds.
There are three places a class can come from:

| Where the class is written | Example |
| --- | --- |
| A `class` attribute in HTML | `<p class="text-sm text-slate-500">` |
| A `class:name` binding, switched on and off by Rust | `class:bg-indigo-600="state.active.get()"` |
| A string in Rust code | `"text-emerald-700"` in `src/app.rs` |

A `class:name` binding adds the class while its expression is `true` and removes it while
it is `false`. Tailwind can't see these names in your files the way it sees the others, so the
fusor compiler passes them to Tailwind directly. Variants such as `md:` and `hover:`, and
arbitrary values such as `bg-[#4f46e5]`, work in binding names too.

```html title=web/index.html
<button class="rounded px-4 py-2"
        class:bg-indigo-600="state.active.get()"
        class:md:px-8="state.active.get()"
        on:click="state.active.update(|active| *active = !*active)">Toggle</button>
<p class="{{ state.tone() }}">Status</p>
```

```rust title=src/app.rs
fn tone(&self) -> &'static str {
    if self.active.get() { "text-emerald-700" } else { "text-slate-500" }
}
```

> Write each class name in full. Tailwind can't find a name your code assembles while it
> runs, such as `format!("text-{color}-700")`.

- [Attributes, including class:name](/docs/html-and-rust/attributes)

## Customize Tailwind {#customize}

Everything after the import is ordinary Tailwind CSS. Add theme values, your own utilities
or plain CSS rules:

```css title=web/app.css
@import "tailwindcss";

@theme {
  --color-brand: #4f46e5;
}
```

Tailwind skips files that your `.gitignore` excludes, so it never reads build output. To
include templates outside your package, such as a shared component crate, add a `@source`
line with a path relative to the stylesheet:

```css title=web/app.css
@import "tailwindcss";
@source "../../shared-components/web";
```

The official `@tailwindcss/typography` and `@tailwindcss/forms` plugins are built in; enable
one with `@plugin "@tailwindcss/typography";`. Plugins installed from npm are not loaded.

## What a class:name binding can't contain {#limits}

A binding's class name is an HTML attribute name, and HTML has two rules for those:

- **A `/` ends the name.** `class:w-1/2` and `class:bg-black/50` are compile errors.
- **Capital letters become lowercase.** `class:bg-[#4F46E5]` toggles `bg-[#4f46e5]`. Tailwind
  receives the lowercase name, so this still works for colors, but not for values where case
  matters, such as a URL.

For these classes, compute the whole `class` attribute in Rust instead:

```html title=web/index.html
<div class="{{ if state.wide.get() { "w-1/2 bg-black/50" } else { "w-full" } }}"></div>
```

An element with a computed `class` attribute can't also have `class:name` bindings.

## Builds and live refresh {#builds}

- `fusor build` minifies the stylesheet. Its file name contains a hash of its contents, so
  browsers and CDNs can cache it forever.
- In `fusor dev`, editing your HTML or your stylesheet updates the styles in the open page
  without losing its state. Editing Rust rebuilds the application as usual.
- If Tailwind reports an error, for example an unknown class in `@apply`, the build fails and
  shows Tailwind's message.

## Version and download {#version}

fusor uses Tailwind CSS 4.3.3. It downloads the program from Tailwind's GitHub releases and
checks it against a checksum recorded in fusor. Each fusor release pins one Tailwind
version, so every machine builds the same CSS.

Standalone builds exist for macOS, Linux and 64-bit Windows on x86. On another system, set
`FUSOR_TAILWIND` to a `tailwindcss` 4.3.3 program you installed yourself.

- [Environment variables](/docs/cli#environment)
