# fusor documentation

This is a complete fusor application. HTML templates own layout; external
Rust files own application state, search, theme, and typed routes. The managed
`App` handles startup and cleanup.

## Writing a guide

Write page bodies in `public/content/<slug>.md`; the introduction is
`public/content/index.md`. Nested slugs use directories, for example
`public/content/workers/tasks.md`. Start with one `# Page title`, followed by
introductory prose and `## Section headings {#stable-id}`. Keep existing IDs
when renaming headings so published links continue to work. Subheadings may
have explicit IDs too; otherwise the renderer generates unique IDs from their
text. The page title, table of contents and search text come from the Markdown.

Use ordinary Markdown for paragraphs, emphasis, links, images, lists, quotes,
fenced code, tables, strikethrough and task lists. Use inline code for types,
paths, attributes and expressions. A code fence's first word is its language;
an optional `title=` at the end supplies the visible caption:

````markdown
## Read a signal {#read}

Call `count.get()` to read the current value.

```rust title=src/app.rs
let count = signal(0);
```

Read the [ownership guide](/docs/ownership) for cleanup.
````

An empty fence can include a real source file, relative to `apps/docs/`:

````markdown
```rust source=tutorial/src/watch.rs title=src/watch.rs
```
````

The build tracks included files and fails if they are missing or the fence also
contains code. The scaffold walkthrough uses the CLI templates; the other
complete examples come from the compiling `tutorial/` application and lessons.
The executable browser suites copy these same authored code blocks into apps.

Use blockquotes for notes and ordinary headings and lists for API entries.
For collapsible signatures, use `<details>` and `<summary>Signature</summary>`
on their own lines, a blank line before Markdown content, and `</details>`
after another blank line. Other raw HTML is displayed as text. Link and image
URLs accept relative paths, fragments, HTTP(S), and mailto links.

`content/navigation.json` contains only slugs, groups, parent relationships,
reference-page flags and reading order. Add a record there when adding a guide.
Set `parent` to another page's slug in the same group, and `reference: true` for
detailed lookup pages. The first record is the introduction with an empty slug.
Keep children immediately after their parent; order also controls previous/next
links. The build rejects invalid or
duplicate slugs, missing parents, cycles, mismatched groups and duplicate IDs.
`content/references.json` maps code tokens to documentation anchors: guide code
blocks get an “In this example” disclosure with the matching references.
Reference pages omit these automatic links. Targets are checked during build.
Tokens ending in `:` match directive families; other tokens use identifier
boundaries. Add a reference entry when documenting a public directive or API.

## Building and checking

`build.rs` parses Markdown with pinned `pulldown-cmark` and highlights code
with Syntect and `two-face`. The private Markdown component mounts the generated
HTML through the existing DOM component API. No Markdown parser or highlighter
ships to the browser, and native Cargo checks need no Node. Light and dark code
colors meet 4.5:1 contrast against the docs code backgrounds; keep those
background values in sync when changing the theme.

The ordinary asset pipeline also copies the authored `.md` files into
`dist/docs/content/`. Every guide has a “View Markdown source” link. Source-file
inclusion is resolved for the rendered page; the downloadable Markdown retains
its include directives. `content/resources.json` lists plain-text source copies
written to the ignored `public/source/` directory. Do not edit those generated
copies.

From the repository root, run `npm ci --prefix apps/docs --ignore-scripts`,
`just site`, then `just preview`. Open http://127.0.0.1:8080/docs/.
For development use `just dev-app fusor-docs`; the base path is `/docs/`.
Run `just check`, `just test docs` and `just test docs-examples` after changes.
The browser suites serve the assembled site, so run `just site` first.

The information architecture and typography take inspiration from
[Deno's documentation](https://docs.deno.com/runtime/). The design and content
are authored for fusor. No remote fonts or UI services are required.
Navigation, search, breadcrumbs, theme persistence, mobile navigation and the
introduction's signal-driven counter remain part of the Rust application.

The [showcase](http://127.0.0.1:8080/docs/showcase) has twelve interactive examples,
each with a deep link, reset control, guide link, and source viewer. Gallery
metadata lives in `content/showcase.json`. Each slug names matching files in
`src/demos/` and `web/demos/`; those exact files become the highlighted source and
plain-text downloads during the build. JavaScript demos also name a matching
`web/demos/{slug}.js` and set `javascript: true` in their metadata. To add an
example, declare its module in `src/demos/mod.rs`, associate its discovered HTML
with `template!`, and add a content factory in `src/showcase.rs`, metadata, and
a relevant guide link. The route outlet
owns the demo's lifetime; changing pages or resetting it disposes owned work.

Three examples run background workers and set `workers: true`, which gives them
the wide layout. `search` keeps a million generated books in a stateful worker,
`game` is a Four in a row engine that reports its search as progress and stops on
Move now, and `fractal` paints tiles on a pool's compute threads through a result
stream. Each keeps its worker code and page code in one Rust file. The pool makes
the docs a threaded application: `just site` needs the nightly toolchain from
`cargo fusor install -p fusor-docs`, the build takes a few minutes longer, and
every `/docs/` page is served cross-origin isolated (`vercel.json` in production,
`fusor preview` and the docs test server locally). Without isolation the fractal
falls back to one ordinary worker, which `just test docs` also checks.

The async examples fetch `public/demo-data/` text fixtures over HTTP. Their
deliberate delays and one-time simulated error are labeled in the UI. They don't
require a backend or third-party API. The async/coherent comparison uses the same
loader on both sides: independent `Resource` results alongside an `AsyncBoundary`
with `AsyncValue` reads. Switching products demonstrates a fast price read, a slow stock read, and
cancellation when a selection changes. The shared reset control starts over.
`just test docs` checks all showcase demos,
source fidelity, syntax colors/contrast, row identity, timer cleanup, async
publication, deep links, and responsive navigation in the selected browsers.

Run `just test docs-examples` to build the documented lessons as independent
Cargo apps and exercise bindings, route selection, keyed identity, cleanup,
loading/error/retry, request disposal, and coherent publication in a browser.
Set `PLAYWRIGHT_BROWSERS=chromium,firefox,webkit` for all three engines.

The worker overview has child guides for tasks, stateful workers, pools, shared
data, streams, lifetimes/errors, and deployment. Their complete Rust examples
come from `tutorial/lessons/workers/`. The independent worker consumer includes
those exact files: `just test worker` exercises ordinary examples and
`just test worker-pool` exercises the compute and shared-data examples with
real Wasm threads. Pool examples are feature-gated in that consumer so ordinary
examples retain their ordinary build. These files are not compiled by
`docs-examples` or included in the tutorial app's startup.

The `workers/api` reference has one Markdown section per public type or function,
with a linked type index and glossary first. Its examples are short excerpts based on the
compiled lessons in `tutorial/lessons/workers/`, but they are not compiled
themselves; its declarations omit bodies and private fields. Both are lookup
material, not runnable files. Keep them aligned
with `fusor-worker` and the generated service clients, and link example tokens
to their type sections in `content/references.json`.

## Native library showcases

Chart.js and Three.js use native component modules and `JsInputs`. The app’s
`package.json` pins its libraries and esbuild; install with npm explicitly before
a CLI browser build. Cargo’s native workspace checks still need no Node. Dynamic
imports keep both libraries off ordinary Rust-only documentation routes.

The Orbital Garden uses 24 deterministic curved ribbon meshes, standard physical
materials, a generated room environment, one renderer/output color transform,
and simple analytic motion. It uses no post-processing stack or custom shaders.
Rust owns bloom, palette, pause, and selected ribbon; native events return picks.
Pointer orbit, keyboard selection, responsive framing, reduced motion, and
visibility pausing are supported. Cleanup releases the RAF, subscriptions,
listeners, observer, geometries, materials, textures, environment, and renderer.

`tests/tooling/docs-showcase.mjs` verifies interactions, JavaScript source fidelity,
library deferral, resource cleanup, and mobile layout. Chromium screenshots in
`target/screenshots/` provide fixed reduced-motion views for visual review; frame
rates and GPU times are not correctness gates. WebGL 2 is required for the garden.

`just test docs-libraries` builds an independent temporary consumer from the
exact garden source. Two instances verify independent Rust state and events,
keyed renderer retention, survivor cleanup, and cancellation during library load.
