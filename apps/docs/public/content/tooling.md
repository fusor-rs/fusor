# Tooling and testing

Use the standalone CLI in your application. Use the repository’s additional test suites when
working on fusor itself.

## Commands to run in your app {#commands}

Run these commands inside your generated `my-app` directory. `fusor check` compiles and
type-checks the Wasm target. `fusor expand` shows the Rust generated from an HTML template.

`fusor dev` builds, watches, and serves your app. `fusor build` creates optimized files in
`dist/`; `fusor preview` previews that output locally.

Builds and checks preserve `Cargo.lock` by default. Run `fusor install` after deliberately
changing dependencies; use `--frozen` for locked, offline execution.

```sh title=Terminal
# In your application directory:
fusor check
fusor expand --module app
fusor dev --port 8090
fusor build --locked
fusor preview --port 8092
```

> The `dev` command keeps running. Use another terminal for `check` and `build`, or stop it
> first. To choose an application in a Cargo workspace, use `--package`; to use a manifest
> elsewhere, pass `--manifest-path`.

- [Every command, option and setting](/docs/cli)

## Read an error at its source {#diagnostics}

The compiler reports authored HTML locations for binding errors. Rust expressions in HTML
inherit the consuming module's lint settings, including denied warnings. Generated scope
bindings may be unused without suppressing diagnostics in your expressions. For a component
tag, check the imported Rust type, `FromInputs` contract, and
`template!(package-relative-path)` association in its ordinary Rust module.

Projects using the older external-script API should check their explicit Cargo HTML
registration, `rust:module` path, and `bindings!` name.

Missing state/input fields and wrong values are ordinary Rust errors; expand helps inspect
generated bindings.

> If no Rust module includes the entry’s `template!` association, `check` and `build` can
> still pass because nothing references those bindings. At runtime, the `<App>` page cannot
> start: the loader reports a missing startup export through `fusor:error`. Add
> `template!("web/index.html")` to the module that owns the entry.

- [Check the template association](/docs/project-structure)
- [Check binding scope names](/docs/html-and-rust#scope)

## Know what a save preserves {#refresh}

In Rust-only applications, compatible HTML and CSS edits can refresh while keeping current
state, focus, and requests. Applications with native component JavaScript currently rebuild
and reload on source edits.

Rust behavior changes and incompatible markup rebuild and reload.

A compile error leaves the last working page available and reports the error. This is a
development workflow; production serves versioned generated output.

## Test your own application behavior {#verify}

Test ordinary Rust logic with `cargo test` in your app.

Use browser tests for the things a native test cannot observe: rendered elements, focus,
navigation, actual requests, and cleanup when a view disappears.

fusor’s optional `fusor-test` crate supplies controlled executors for deterministic async
tests; it is not required to render an app.

```sh title=Terminal · inside your application
cargo test --locked
```

- [Try an overlapping request](/docs/async-data#cancel)
- [Observe cleanup when a child disappears](/docs/ownership#lifetime)

## Run the framework’s acceptance suites {#repository-tests}

These commands come from the `justfile` at the framework repository root. They are
contributor commands, not commands generated in `my-app`. Run `just` on its own to list
every recipe.

`just setup` installs the toolchain, Node dependencies and browsers. `just fixtures` builds
the examples and the site that the suites serve. The companion examples are also compiled
and exercised by `just test docs-examples`.

```sh title=Terminal · framework contributors
# From the fusor repository root:
just setup
just fixtures
just test docs
just test docs-examples
just test foreach
just test coherent
just test islands
```

> `just check` runs the broader Rust checks. The repository also contains standard-library
> forms/actions examples, integration tests, and benchmark workloads; they are separate from
> this beginner path.

## Write documentation in Markdown {#documentation}

Each guide is a **Markdown file** in `apps/docs/public/content/`. Its first
heading supplies the page title. Level-two headings supply the table of
contents; give them explicit IDs to keep links stable when their wording changes.

| File | Purpose |
| --- | --- |
| `public/content/<slug>.md` | Guide text, links, lists, tables and code examples |
| `content/navigation.json` | Reading order, groups and parent pages |
| `content/references.json` | Contextual links from code examples to API sections |

````markdown title=A guide
# My guide

Explain the task and link to [related guidance](/docs/ownership).

## Show a complete example {#example}

```rust source=tutorial/src/watch.rs title=src/watch.rs
```
````

An empty `source=` fence reads a real file, relative to `apps/docs/`, during
the build. For a standalone example, write code directly inside a fenced block.
Use ordinary Markdown blockquotes for notes and `<details>` disclosures for
long signatures. See `apps/docs/README.md` for the complete authoring conventions.

`just site` renders the guides and publishes their Markdown files as static
assets. The **View Markdown source** link on each page opens its authored file.
The shared [docs-base](https://github.com/fusor-rs/docs-base) packages own the
Markdown compiler, article layout, navigation, search and theme controls.
Fusor keeps its guides, branding and interactive examples in this application.
Run `just test docs` and `just test docs-examples` to check the rendered guides
and execute their application examples.

## Read the exact source behind a guide {#source}

The code blocks for the generated app and companion examples are read from their source
files during the docs build.

Source links below open plain text in a new tab; commands and paths marked “repository root”
refer to the same checkout as these docs. The installation guide uses the released CLI and
registry crates. Repository examples can also be run from a local checkout.

- [Complete companion App state](/docs/source/tutorial-app.rs.txt)
- [Complete companion HTML](/docs/source/tutorial-index.html.txt)
- [Companion manifest and registration](/docs/source/tutorial-Cargo.toml.txt)
