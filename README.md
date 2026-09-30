<p>
  <img src="apps/landing/public/brand/fusor-horizontal.svg" alt="fusor" width="200">
</p>

# Build reactive web apps with Rust

Write your UI in HTML files. Keep state and frontend logic in Rust. No markup inside Rust macros.

fusor compiles your Rust to WebAssembly for the browser. When a signal changes, the bindings that read it run again.

**In development · v0.1** — This is experimental software. APIs will change; don't rely on it for production yet. See [status and known limitations](#status-and-known-limitations).

[Get started](#get-started) · [How it works](#how-it-works) · [Examples](#examples) · [Status](#status-and-known-limitations) · [Contributing](CONTRIBUTING.md)

## A component in two files

The view is an HTML file. Bindings and event handlers are Rust expressions:

```html
<!-- web/components/counter.html -->
<template rust:component="Counter">
  <div class="counter">
    <output aria-live="polite">{{ state.count.get() }}</output>
    <div class="counter-actions">
      <button on:click="state.increment()">Increment</button>
      <button on:click="state.reset()">Reset</button>
    </div>
  </div>
</template>
```

The Rust module owns the state and behavior, and connects them to the HTML with `template!`:

```rust
// src/counter.rs
use fusor::prelude::*;

#[derive(FromInputs)]
pub struct Counter {
    #[local(init = signal(0))]
    count: Signal<i32>,
}

impl Counter {
    fn increment(&self) {
        self.count.update(|n| *n += 1);
    }

    fn reset(&self) {
        self.count.set(0);
    }
}

fusor::template!("web/components/counter.html");
```

`rust:component="Counter"` makes the struct's fields and methods available as `state`. Each button calls one of its methods, which changes the `count` signal. The output reads that signal, so its text updates when the count changes. The compiler checks the expressions in the HTML along with the rest of the module.

These two files are a complete reusable component, not a whole app. A page places it with a `<Counter></Counter>` tag inside `<App>`, which starts the app and owns everything in it. The landing page's example editor shows, next to each component's files, a short page that mounts it; for this counter, that is [`index.html`](apps/landing/host/counter/web/index.html) and its [Rust module](apps/landing/host/counter/src/app.rs). `fusor new` creates `Cargo.toml`, `build.rs`, and `src/lib.rs`; declare additional component modules in `src/lib.rs`. The documentation site has a full walkthrough of templates, modules, and application startup.

## Get started

The v0.1.0 CLI and crates are released. To build apps you need Rust 1.85 or newer, installed with [rustup](https://rustup.rs). Then install the CLI:

```sh
# macOS or Linux
curl -fsSL https://fusor.build/install.sh | sh
```

```powershell
# Windows PowerShell
irm https://fusor.build/install.ps1 | iex
```

The installer downloads the latest release, verifies its checksum, and puts `fusor` in `.fusor/bin` under your home directory. On macOS and Linux it prints the line to add to your `PATH` and leaves your shell configuration alone. On Windows it adds the directory to your user `PATH`; open a new terminal before running `fusor`. The scripts are [install.sh](install.sh) and [install.ps1](install.ps1) in this repository if you want to read them first.

Create and run an app:

```sh
fusor new my-app
cd my-app
fusor dev
```

Open the URL printed by `fusor dev` (normally `http://127.0.0.1:4173`). The starter has a working counter component. Edit its HTML or Rust and the dev server rebuilds the app. Run `fusor check` for compiler errors or `fusor build` for a static build in `dist/`. The [installation guide](https://fusor.build/docs/installation) covers requirements and troubleshooting in more detail.

The [documentation app](apps/docs/) contains the installation walkthrough and authoring guides. To run it from a repository checkout:

```sh
cargo fusor dev -p fusor-docs
```

## How it works

fusor keeps the boundaries visible:

- **HTML stays HTML.** Components use native markup with Rust expressions for text, attributes, conditions, lists, and events.
- **Rust stays Rust.** State, methods, and imports live in ordinary modules. Cargo and rustc check the generated bindings alongside your code.
- **Updates follow signal reads.** A binding tracks the signals it reads and updates its own DOM target when they change. There is no virtual DOM.
- **Ownership handles cleanup.** Removing a component disposes its listeners, subscriptions, and owned children.

The default application is a client side WebAssembly app. Optional packages add routing, async resources, shared queries, background workers, native JavaScript integrations, and server rendered islands. See the [guides](apps/docs/) for their setup and current limits.

## Examples

- [Landing app](apps/landing/) — a complete fusor site with live counter, search, lists, and async examples.
- [Playground](examples/playground/) — Rust written directly inside an HTML page with reactive bindings.
- [Reader](examples/navigation/) — typed navigation and async data across owned views.
- [Editor](examples/editor/README.md) — typed fields, explicit saves, and state shared across views.
- [Integrations](examples/integrations/) — CodeMirror and Chart.js connected to Rust state.
- [Workers](examples/workers/) — background Rust tasks connected to async resources.

The [documentation showcase](apps/docs/) has more runnable examples. Performance measurements and their methodology live in [benchmarks](benchmarks/README.md).

## Status and known limitations

fusor is v0.1 and experimental. It targets the browser only: there is no desktop or mobile renderer.

**Breaking changes.** Until 1.0, each minor release (0.2, 0.3, …) may change the HTML syntax and the Rust APIs. Patch releases (0.1.x) fix bugs without breaking changes. Every breaking release will come with migration notes.

**Bundle size.** A hello-world app ships about 54 KiB gzipped, counting its HTML, JavaScript and Wasm. In the same [benchmark run](benchmarks/README.md), the Leptos equivalent is 25.5 KiB and Solid's is 4 KiB. Size grows with the app, and reducing it is ongoing work.

**Editor support.** rust-analyzer works in your `.rs` modules, with completion, hover and go to definition on the fields and methods templates use. Compiler errors in template expressions point at the line in the `.html` file. There is no completion or hover inside `.html` files yet.

**Not supported yet:**

- SVG, MathML, `<select>` and table parsing contexts inside `<ForEach>`, `<If>` and `<Match>`. Tables work with an explicit `<tbody>` around `<ForEach>`.
- Match guards and `ref`/`mut` captures in `<Case>` patterns.
- `bind` on file inputs. Read their files in an `on:change` handler.
- Nested `<Async>` boundaries, editable controls and router outlets inside coherent async views.
- Nested islands and streamed server rendering. Server rendering is synchronous, and your application resolves its data first.
- A built-in backend or server functions. fusor renders HTML; you bring your own HTTP server and data layer.
- npm workspaces and linked packages. Apps that use component JavaScript rebuild and reload on every source edit instead of refreshing in place.

The [guides](apps/docs/) list the limits of each feature in more detail.

## How is this different from Dioxus?

[Dioxus](https://github.com/DioxusLabs/dioxus) components are Rust functions that return `rsx!` markup, and Dioxus reconciles their output through a [`VirtualDom`](https://docs.rs/dioxus-core/latest/dioxus_core/struct.VirtualDom.html). fusor writes markup in separate `.html` templates connected to Rust modules with `template!`. It has no virtual DOM: bindings track the signals they read and update their associated DOM targets.

## How is this different from Leptos?

[Leptos](https://github.com/leptos-rs/leptos) and fusor share a reactive model: fine-grained signal tracking and no virtual DOM. The difference is authoring. Leptos views are written in Rust, with the [`view!`](https://docs.rs/leptos/latest/leptos/macro.view.html) macro or builder functions. fusor uses separate HTML templates, with Rust expressions in bindings and attributes, connected to Rust modules with `template!`.

## How is this different from Sycamore?

[Sycamore](https://github.com/sycamore-rs/sycamore) is the closest in design: fine-grained reactivity, no virtual DOM, and Rust compiled to WebAssembly. It is also older and more established. The difference is again authoring. Sycamore components are Rust functions that return markup written with the [`view!`](https://docs.rs/sycamore/latest/sycamore/macro.view.html) macro. In fusor the markup is an `.html` file, and a Rust type provides its state and methods.

## Contributing

fusor is an open source project in early development. The [contributing guide](CONTRIBUTING.md) covers the repository setup, checks, and development workflow. The framework and tooling live in `crates/`; runnable applications live in `apps/` and `examples/`.

fusor is built with the help of AI tools, mainly Claude Code, under the same rules the project asks of contributors: see [Using AI tools](CONTRIBUTING.md#using-ai-tools).

## License

Licensed under the [MIT License](LICENSE).
