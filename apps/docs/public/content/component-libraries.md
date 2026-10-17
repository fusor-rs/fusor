# Component libraries

Split a large application into several crates. A component library is a Rust crate with its
own HTML templates and Rust modules. An application that depends on it uses its components
exactly like its own: as tags in HTML, or as `Content` chosen at run time.

## Create the library crate {#create}

A library has the same files as an application, minus the page: no `web/index.html`, no
`public/` assets and no `dist/` output. `library = true` tells fusor that the crate's HTML
joins the page of whichever application depends on it. Templates under `web/components/`
are discovered as they are in an application.

```toml title=panel/Cargo.toml
[package]
name = "panel"
version = "0.1.0"
edition = "2024"
rust-version = "1.85"

[dependencies]
fusor-core = { version = "=0.1.7", features = ["dom"] }

[build-dependencies]
fusor-build = { version = "=0.1.7" }

[package.metadata.fusor]
library = true
```

The build script calls `compile_library` instead of `compile_app`.

```rust title=panel/build.rs
fn main() -> Result<(), fusor_build::BuildError> {
    fusor_build::compile_library()
}
```

A library has no page of its own, so the keys that configure one are rejected:
`entry`, `assets`, `assets-build`, `tailwind`, `output`, `base-path`, `history-fallback`,
`dev`, `dev-refresh` and `delivery`. The application sets those. `templates` and
`components` work as in an application.

## Write its components {#components}

Components are written as in an application: an HTML template, and a Rust module that
connects it with `template!(path)`. Mark the type `pub` so other crates can use it, and
export it from the crate root.

```html title=panel/web/components/panel.html
<template rust:component="Panel">
  <section class="rounded-xl p-4" class:ring-2="state.active.get()">
    <button on:click="state.active.update(|active| *active = !*active)">Toggle</button>
  </section>
</template>
```

```rust title=panel/src/panel.rs
use fusor::prelude::*;

#[derive(FromInputs)]
pub struct Panel {
    #[local(init = signal(false))]
    active: Signal<bool>,
}

impl Panel {
    pub fn new(_owner: OwnerHandle) -> Self {
        Self { active: signal(false) }
    }
}

fusor::template!("web/components/panel.html");
```

```rust title=panel/src/lib.rs
mod panel;

pub use panel::Panel;
```

## Use it from an application {#use}

Add the library to the application's dependencies. Within one Cargo workspace, a path
dependency is simplest:

```toml title=app/Cargo.toml
[dependencies]
panel = { path = "../panel" }
```

Import the type, then use its tag, or wrap it in `Content` for a
[slot](/docs/nested-content) whose component is chosen at run time:

```rust title=app/src/app.rs
use fusor::prelude::*;
use panel::Panel;

struct App {
    sidebar: Content,
}

impl App {
    fn new() -> Self {
        Self { sidebar: Content::new(|owner| Panel::new(owner)) }
    }
}

fusor::template!("web/index.html");
```

```html title=app/web/index.html
<App state="{{ App::new() }}">
  <main>
    <Panel></Panel>
    <aside rust:slot="state.sidebar.clone()"></aside>
  </main>
</App>
```

You don't list the library anywhere else. `fusor build`, `fusor dev` and `fusor preview` find
every component library the application depends on.

## What fusor adds to the page {#build}

When it builds the application, fusor:

- adds each library's templates to the application's page. Every library numbers its
  components separately from the application and from other libraries, so templates never
  collide.
- adds each library's package to the files [Tailwind CSS](/docs/tailwind) scans, and passes
  it the library's `class:name` bindings. Classes written in a library work like the
  application's own.
- watches libraries that are path dependencies during `fusor dev`. Editing a library's HTML
  or Rust rebuilds the application and reloads the page. Editing the application's HTML
  still refreshes the page in place.

## Put routes in a library {#routing}

A library's template can contain a `Router`, as long as it is placed inside a route of the
application's router that ends in `/*`. It then matches the rest of the URL, like any
[nested router](/docs/routing#nested). Add the same `fusor-router` dependency the
application uses to the library's `[dependencies]`. Here the application gives the library
everything under `/panel/`:

```html title=app/web/index.html
<Router>
  <Route path="/"><Panel></Panel></Route>
  <Route path="/panel/*"><PanelPages></PanelPages></Route>
</Router>
```

```html title=panel/web/components/pages.html
<template rust:component="PanelPages">
  <section>
    <Router>
      <Route path="settings"><p>Panel settings</p></Route>
      <Route fallback><p>Panel home</p></Route>
    </Router>
  </section>
</template>
```

`/panel/settings` shows the settings route. The library does not know the application's
`base-path`, so its router cannot own browser history: outside the application's router it
fails with "a component library's Router must be inside a Route of the application's Router".

## Know the limits {#limits}

- `App` belongs to the application's entry page only.
- Component JavaScript (`<script type="module">` in a template) belongs in the application.
- Library templates are added to browser applications. Islands delivery does not add them.
