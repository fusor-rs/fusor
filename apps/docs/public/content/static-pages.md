# Static pages

Publish a site where every page is plain HTML, written by Rust before anyone visits. fusor adds
nothing that runs in the browser: no WebAssembly and no scripts, only documents and a stylesheet.
A `<script>` you write in a template is published as written.

## When to use static pages {#start}

Use static delivery for a site that is mostly reading, such as a blog, a changelog or a set
of notes. Each URL gets its own HTML document, so its title, description and content are in
the page itself.

That matters wherever the page's HTML is all that gets read. Search engines and link previews
in apps like X, Slack and iMessage do not run WebAssembly, so a page that builds itself in the
browser looks the same to them at every URL.

> Choose a normal fusor application when the pages need to respond to people, and
> [islands](/docs/islands) when mostly static pages have a few interactive parts.

## How a static site is built {#build}

A static site is a Rust program. `fusor build` compiles it for your own computer, not for the
browser, and runs it once. The program writes every file of the site into a directory that
`fusor build` gives it. fusor then publishes those files, links the compiled stylesheet into
each page and, in `fusor dev`, adds live reload.

```text title=Text
BUILD
Your program writes index.html, notes/first/index.html, robots.txt …
fusor links the stylesheet into each page and publishes everything

VISIT
Static hosting sends a page's index.html → the browser displays it
```

## Turn it on {#setup}

Declare static delivery in `Cargo.toml`. The package needs a binary, `src/main.rs`, and the
server rendering crates:

```toml source=../../examples/static-pages/Cargo.toml title=Cargo.toml · complete file
```

Static delivery has no units, the separately built browser parts of
[island delivery](/docs/islands/setup). Declaring one is an error.

## Write the page in HTML {#template}

The entry file is the whole document. `rust:render="server"` renders it in your program
instead of the browser. `{{ }}` bindings, `<ForEach>` and `<If>` work as in any other
template.

```html source=../../examples/static-pages/web/index.html title=web/index.html · complete file
```

A `<title>` can hold a binding when the binding is its whole text, as
`<title>{{ state.title }}</title>` does here. That gives each page its own title.

## Write every page from Rust {#program}

`fusor build` runs the program as `--fusor-render-pages DIRECTORY`, from the package directory,
so it can read its content by relative path. The program renders one document per page and
writes it into that directory.

```rust source=../../examples/static-pages/src/main.rs title=src/main.rs · complete file
```

- **`index.html` is required.** It is the site's first page.
- **A page at `/notes/first/` is written to `notes/first/index.html`.** Static hosts, `fusor dev`
  and `fusor preview` serve it at that URL, and redirect `/notes/first` to it.
- **Every `.html` file must be a complete document with a `<head>`.** fusor adds the
  stylesheet link there.
- **Other files are published unchanged,** such as `robots.txt` or a feed.
- **`FUSOR_BASE_PATH` is the URL path the site is served under,** from `base-path`. Use it to
  write links that work when the site is mounted at a path such as `/blog/`.

> A file the program writes under `__fusor/`, or at the same path as a file in
> `[package.metadata.fusor.assets]`, stops the build and names the file.

## Style it with Tailwind {#tailwind}

Set `tailwind` as for any application. fusor compiles the stylesheet from your templates and
Rust code and links it into every page.

- [Tailwind CSS](/docs/tailwind)

## Build and preview {#run}

From the fusor repository, build the example and serve its output; open
http://127.0.0.1:8095/.

```sh title=Terminal
fusor build --package fusor-static-pages-example --locked
fusor preview examples/static-pages/dist --port 8095
```

In `fusor dev`, each page also loads the live-reload module, and every edit to the program,
its templates or its content builds the whole site again.

## Limits {#limits}

- **No interactive parts.** Islands need their own [island delivery](/docs/islands/setup),
  which publishes one page.
- **Every build writes every page.** There is no incremental rebuild of a single page yet.
