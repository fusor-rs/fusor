# AGENTS.md

Context for coding agents working in this repository. Read this first, then the
document it points to for the area you are changing. `CONTRIBUTING.md` is the
source of truth for setup, checks and workflow; this file summarizes it and adds
what isn't obvious from the code.

## What this is

fusor is an experimental (v0.1) Rust web framework. Components are written as
plain `.html` templates with Rust expressions in bindings, backed by ordinary
Rust modules. A build script compiles the HTML into Rust, the app compiles to
WebAssembly, and bindings update the DOM directly from fine-grained signals.
There is no virtual DOM and no markup inside Rust macros.

APIs and HTML syntax may still change. Prefer getting behavior right over
preserving an existing API, but changes to the HTML syntax, a public Rust API,
or adding a crate need a design issue first (see `CONTRIBUTING.md`).

The GitHub repository is `fusor-rs/fusor`; the local checkout directory name
(`rust-front`) is historical.

## How an application compiles

1. The app's `build.rs` calls `fusor_build::compile_app()`.
2. `fusor-build` parses the HTML entry (`[package.metadata.fusor] entry`, usually
   `web/index.html`) and the component templates it reaches, into a typed IR in
   `crates/fusor-build/src/bindings/`, then lowers it to Rust with `quote!`.
3. A module includes the generated code with `fusor::template!("web/…/x.html")`.
   `rust:component="Counter"` binds a template to a Rust type, exposed as
   `state` inside the template. `<script type="text/rust" src="../src/app.rs"
   rust:module="crate::app">` links a page to an external module.
4. `fusor-cli` (`cargo fusor` / `fusor`) runs Cargo for `wasm32-unknown-unknown`,
   wasm-bindgen, optional npm bundling, and publishes the site.

Note: the crate `fusor-core` has library name `fusor` (the crates.io name was
taken), so code says `use fusor::prelude::*`.

Template syntax at a glance: `{{ expr }}`, `on:event`, `class:name`,
`bind`, `prop:name`, `rust:if`,
`rust:key`, `rust:render`, `hydrate*`, plus the built-in tags `App`, `If`/`Else`,
`Match`/`Case`, `ForEach`, `Children`, `Async`, `Await`, `Router`/`Route`. The
full reference is `apps/docs/content/pages.json` (slugs
`html-and-rust/attributes` and `html-and-rust/built-in-components`).

## Repository map

| Path | What it is |
| --- | --- |
| `crates/fusor-core` | Signals, ownership/cleanup, DOM runtime. `template.rs` is the versioned compiler↔runtime contract. |
| `crates/fusor-build` | Build-script compiler: HTML → Rust. Parser, IR, codegen, source maps. |
| `crates/fusor-macros` | `FromInputs` and `JsInputs` derives. |
| `crates/fusor-components` | Built-in template tags (`App`, `ForEach`, `If`, `Router`, …). |
| `crates/fusor-cli` | The `fusor` CLI (`new`, `dev`, `build`, `check`, `preview`, `add`, `doctor`, …; hidden `repo check` backs `just check`). Read its `ARCHITECTURE.md` before editing. |
| `crates/fusor-async`, `fusor-query`, `fusor-router`, `fusor-std` | Optional packages: async resources, shared queries, typed routing, forms/actions (`fusor-std` re-exports the others behind features). |
| `crates/fusor-islands`, `fusor-server` | Server-rendered islands and SSR HTML. |
| `crates/fusor-npm` | Internal npm bundling for the CLI. |
| `crates/fusor-test` | Deterministic test helpers (requests, clock, lifetimes). |
| `crates/fusor-release` | Contributor tooling for release archives. |
| `apps/landing`, `apps/docs`, `apps/benchmarks` | The fusor.build site, mounted at `/`, `/docs/`, `/benchmarks/` (see `[workspace.metadata.fusor.site]` in `Cargo.toml`). |
| `apps/docs/tutorial` | Separate Cargo project the guides compile against. |
| `examples/` | Runnable feature examples; most are workspace members. `examples/npm` is excluded from the workspace. |
| `benchmarks/` | Seven-framework benchmark workloads, harness and published records. |
| `tests/tooling/*.mjs` | Node browser/tooling suites, run with `just test <name>`. |
| `tests/fixtures/` | Standalone consumer apps built outside the workspace. |
| `tests/browser/` | Playwright suite for the playground. |
| `scripts/` | Node repository tooling. |

`dist/`, `target/`, `test-results/`, `apps/docs/public/source/` and several
files under `apps/benchmarks/public/` are generated; don't edit them by hand.

## Commands

Every task is a `just` recipe; run `just` to list them. Recipes use `cargo
fusor`, an alias in `.cargo/config.toml` that runs the CLI from this checkout.

- `just check` — fmt, workspace tests, Clippy with `-D warnings`, `fusor-std`
  feature isolation, out-of-workspace consumers, rustdoc. No browser needed.
  Run it before considering a change done.
- `just test <suite>` — one suite from `tests/tooling/`. Pick suites by area
  using the table in `CONTRIBUTING.md` ("Running the tests").
- `just test-browser` — Playwright for the playground. Rebuild first:
  `cargo fusor build -p fusor-playground`.
- `just fixtures` — rebuild what `coherent`, `islands`, `docs`, `benchmarks`
  and `test-landing` serve. Without it those suites test stale output.
- `just dev` / `just dev-app <package>` — live-reload dev server on
  http://127.0.0.1:4173.
- `just site` / `just preview` — build and serve the combined site from `dist/`.
- `just ci-browser` — the required browser gate CI runs.

Suites start their own servers: free port 4173 first and run one suite at a
time. `just test authoring` needs the `rust-analyzer` component.

## Rules

**General**
- Rust edition 2024, MSRV 1.85; CI builds on 1.85 too, so don't use newer std
  or language features. `unsafe_code` is forbidden workspace-wide.
- Repository tooling is Rust, or Node under `scripts/`. No third language.
- New tasks go in the `justfile`, not npm scripts. `package.json` only declares
  dependencies.
- Commit lockfile changes with the change that needs them; don't bump unrelated
  dependencies.
- `wasm-bindgen` is pinned exactly and must match `BINDGEN_VERSION` in
  `crates/fusor-cli/src/layout.rs`. Change both together.
- All crates share one version in `[workspace.package]`.

**Compiler (`fusor-build`)**
- New HTML behavior goes through the parser and typed IR, then `quote!`. Never
  build Rust source from strings.
- Preserve authored spans and line positions so rustc errors point at the HTML.
- Add a valid and an invalid case. When generating new Rust, prove it compiles
  in a real application; a string assertion is not enough.

**Runtime and DOM (`fusor-core`)**
- Test observable behavior: scheduling order, when cleanup runs, DOM node
  identity across updates. Include a callback that removes its own element.
- Preserve focus, cursor position and accessibility behavior.
- Changing the shared template format means bumping `VERSION` in
  `crates/fusor-core/src/template.rs`.

**CLI (`fusor-cli`)** — the rules in `crates/fusor-cli/ARCHITECTURE.md`, notably:
commands in `commands/<verb>.rs` with dispatch in `commands/mod.rs`; errors are
`error::Error` with the fix in `remedy`; all output through `Reporter` (no bare
`println!`); generated paths and pinned versions live in `layout.rs`; external
data is deserialized into types; functions stay under ~60 lines. Adding an error
`Kind` means documenting its exit code on the CLI docs page.

**Docs**
- User-visible changes update the matching page in
  `apps/docs/content/pages.json`. Its prose format is deliberately small; see
  `apps/docs/README.md` before editing.

**Benchmarks**
- Follow `benchmarks/README.md` and `benchmarks/METHODOLOGY.md`. Never change a
  workload to help one framework's numbers. To run or publish benchmarks, use
  the `benchmarks` skill in `.claude/skills/benchmarks/`.

## Commits and pull requests

- Commit messages are a short imperative sentence in sentence case, no prefix
  (e.g. "Show page source for the selected landing example").
- One change per pull request. Describe the problem, the behavior change and
  the commands you ran (see `.github/pull_request_template.md`). For a bug fix,
  name the test that fails without it.

## Deploying and releasing

The site deploys to Vercel only through the **Deploy site** workflow or `just
deploy`; pushes don't deploy, and Vercel can't build it (no Rust toolchain).
Releases are cut by publishing a GitHub release tagged `v<version>`. Don't
deploy or release unless asked. Details are in `CONTRIBUTING.md`.
