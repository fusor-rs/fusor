# Project structure

Walk through the app that `fusor new` generated. Each file below already exists after
Installation.

## Find the generated files {#files}

The entry HTML defines the page. HTML under `web/components/` defines reusable markup and is
discovered automatically. Each ordinary Rust module associates its HTML with
`template!(path)`. `public/` holds served assets; `dist/` is generated output.

```text title=Generated file tree
my-app/
  Cargo.toml
  rust-toolchain.toml
  build.rs
  src/
    lib.rs
    app.rs
    counter.rs
  web/
    index.html
    components/
      counter.html
  public/
    app.css
```

## Read the manifest {#configuration}

The generated manifest pins the framework crates to the CLI’s release version and downloads
them from crates.io.

The `dom` feature enables browser bindings, and `fusor-components` supplies browser
components. The default entry is `web/index.html`; reusable HTML is discovered under
`web/components/`. One file may declare several component templates.

The core crate is published as `fusor-core`, and its library is named `fusor`, so your code
writes `use fusor::...`.

```toml title=Cargo.toml · generated
[package]
name = "my-app"
version = "0.1.0"
edition = "2024"
rust-version = "1.85"

[workspace]

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
fusor-core = { version = "=0.1.4", features = ["dom"] }
fusor-components = { version = "=0.1.4", features = ["browser"] }
wasm-bindgen = "=0.2.117"

[build-dependencies]
fusor-build = { version = "=0.1.4" }

[package.metadata.fusor]
assets = "public"
output = "dist"
base-path = "/"

[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
panic = "abort"
```

> In `[package.metadata.fusor]`, set `templates = ["ui/components", "ui/pages"]` once for
> another directory layout, or \[\] to disable discovery. Paths are package-relative,
> bounded to this package, and cannot overlap output. Discovery rejects symlinks.

## Let Cargo compile the HTML {#build-script}

Cargo runs `build.rs` before compiling the app. `compile_app` compiles the entry and
discovered reusable HTML into generated bindings. `template!(path)` includes those bindings
when Rust expands the module. No separate template command is needed.

Compiler build helpers return `fusor_build::BuildError`. Its variants distinguish
configuration, manifest, filesystem, source and generated-metadata failures.
`BuildError::Source` retains the authored path and optional line and column; build-script
diagnostics print that location.

```rust title=build.rs · generated
fn main() -> Result<(), fusor_build::BuildError> {
    fusor_build::compile_app()
}
```

## Declare the Rust modules {#crate-root}

`mod app` and `mod counter` declare ordinary Rust modules. Rust does not automatically treat
every file as a module: these declarations include `src/app.rs` and `src/counter.rs` in the
application.

Other modules can then import public items through paths such as `crate::counter::Counter`.
Here `crate` means your application’s library, and `::` separates path segments.

Each module connects its HTML with `template!(path)`. The trailing `!` marks a macro, which
expands into Rust code at compile time. Add new Rust modules here in the same way. The
generated root `include!` is only needed for the older inline/external registration APIs.

```rust title=src/lib.rs · generated
mod app;
mod counter;
```

## Start with one shared count {#app-state}

`struct App` declares the data: a `count` field of type `Signal<i32>`. That signal stores a
32-bit integer that HTML bindings can watch.

`impl App` contains the type’s functions and methods. `fn new() -> Self` returns an `App`;
`new` is a naming convention, not a special language constructor. Inside this block, `Self`
means `App`, so `Self { count: signal(0) }` creates the value with an initial count of zero.

The first `use` line imports `Counter` from `src/counter.rs` so the associated HTML can use
its tag. `use fusor::prelude::*` imports common framework names, including `Signal` and
`signal`. Imports make names available; they do not register HTML.

Cloning a signal gives another handle to the same value. It does not create an independent
counter.

```rust source=../../crates/fusor-cli/template/app.rs title=src/app.rs · generated
```

## Connect the page to App {#entry}

The built-in `<App>` tag marks the region the framework starts and owns. Its `state`
expression evaluates `App::new()` at startup; the returned value becomes `state` inside
`<main>`.

The tag `<App>` is reserved compiler syntax. The Rust struct `App` is your own state type
and may have any name. The call `template!("web/index.html")` in `src/app.rs` connects this
module to the entry HTML.

Each `<Counter>` resolves to the imported Rust type, receives its named inputs, and inserts
its `<section>` root without an extra wrapper.

```html source=../../crates/fusor-cli/template/index.html title=web/index.html · generated
```

## Separate shared and local state {#counter-state}

Both counters receive a handle to the same `count` signal. Each instance also initializes
its own `clicks` signal.

The `#[...]` lines are Rust attributes. `#[derive(FromInputs)]` generates the input
contract: `#[input]` marks the parent-supplied field, and `#[local(init = signal(0))]`
creates private state for each instance.

Neither field is `pub`, so both are private to this Rust module. The HTML can still access
them because `template!("web/components/counter.html")` includes its generated bindings in
this same module.

```rust source=../../crates/fusor-cli/template/counter.rs title=src/counter.rs · generated
```

## See what each Counter renders {#counter-template}

The template wrapper declares reusable markup; each mount inserts its section root. Click
the first Increment button: both shared labels and the parent total change, but only the
first local clicks label changes. No global component name lookup is involved: the compiler
checks the Rust type and its generated template implementation.

```html source=../../crates/fusor-cli/template/counter.html title=web/components/counter.html · generated
```

- [Learn the binding syntax](/docs/html-and-rust)
- [Follow the complete reusable component lesson](/docs/components)

## Select an app in this monorepo {#workspace}

The preceding commands run inside your own generated app. In the framework repository,
multiple packages are applications, so select one by package name. The documentation app is
a normal consumer of the same compiler and runtime.

```sh title=Terminal
# From the fusor repository root:
fusor dev --package fusor-docs --port 8080
```

> Open http://127.0.0.1:8080/docs/. This package has a /docs/ base path. If the preview is
> already using port 8080, choose another port and keep /docs/ in the URL.
