# Contributing to fusor

Thanks for taking the time to contribute. fusor is experimental: there is no
stable release yet, and the HTML syntax and Rust APIs can still change. That
makes this a good time to influence the design, and it also means we care more
about getting behavior right than about keeping every existing API.

## Reporting a bug

Search the [open issues](https://github.com/fusor-rs/fusor/issues)
first. If nobody has reported it, open a new issue with:

- the output of `fusor --version`, your OS, and the browser if it matters;
- the smallest HTML and Rust that reproduce the problem, ideally starting from
  `fusor new`;
- what you expected, and what happened instead, including the full compiler or
  browser error.

Please report one bug per issue.

Security problems should not go in a public issue. Report them privately
through [GitHub security advisories](https://github.com/fusor-rs/fusor/security/advisories/new).

## Proposing a change

Bug fixes and small improvements can go straight to a pull request.

For anything that changes the HTML syntax, a public Rust API, or adds a crate,
open an issue first so we can agree on the design before you write the code.
Include a short example of the application code you want to write and what it
would mean for existing applications. Internal refactors and new private helpers
don't need this step.

## Setting up

You need:

- Rust stable with `rustfmt` and `clippy`. The minimum supported version is 1.85.
- [just](https://github.com/casey/just), which runs every repository task. Install
  it with `cargo install just`, `brew install just`, or your package manager.
- Node 22 or newer, for the browser and tooling suites.

```sh
git clone https://github.com/fusor-rs/fusor.git
cd fusor
just setup
just dev
```

`just setup` installs the Wasm target and the pinned wasm-bindgen, the npm
packages, the Playwright browsers, and the nightly toolchain that threaded
worker pools build with (the docs' fractal demo uses one). `just dev` serves the whole site, the
landing page, docs and benchmarks, at http://127.0.0.1:4173, and rebuilds
whichever application you edit. To work on one application alone, such as the
framework playground, use `just dev-app fusor-playground`.

## The justfile

The `justfile` at the repository root is the one place tasks are defined. Run
`just` on its own to list them. The recipes call `cargo fusor`, an alias in
`.cargo/config.toml` that runs the CLI from your checkout, so every recipe uses
your local changes.

| Recipe | What it does |
| --- | --- |
| `just setup-browser` | Installs the Wasm tools, fixture dependencies and browsers for framework CI. |
| `just setup` | Also installs dependencies for the full examples and docs site. |
| `just dev` | The whole site with live reload, at http://127.0.0.1:4173. |
| `just dev-app <name>` | One application with live reload, such as `fusor-playground`. |
| `just check` | Formatting, workspace tests, Clippy, consumer builds and rustdoc. No browser. |
| `just test <suite>` | One suite from `tests/tooling/`, such as `just test dev`. |
| `just test-browser` | The Playwright suite for the playground. |
| `just test-tools` | Unit tests for the Node tooling. |
| `just fixtures` | Builds the examples and the site that some suites serve. |
| `just site` | Builds the landing page, docs and benchmarks into one site in `dist/`. |
| `just preview` | Serves that site at http://127.0.0.1:8080. |
| `just ci-browser` | Required framework browser and tooling contracts, using focused fixtures and public-API consumers. |
| `just ci-examples` | Full docs, landing page, benchmark site and third-party library walkthroughs. On Linux, use `xvfb-run -a just ci-examples`. |
| `just ci-docker` | Runs the required browser job in Linux with its framework-only setup. |
| `just bench …` | The benchmark tools; see [`benchmarks/README.md`](benchmarks/README.md). |

When you add a task, add it as a recipe rather than as an npm script or a shell
snippet in a README. `package.json` only declares dependencies.

## Running the tests

Before opening a pull request, run:

```sh
just check
```

It runs formatting, the workspace tests, Clippy with warnings denied, each
`fusor-std` feature on its own, two applications built outside the workspace,
the executable external-renderer consumer, and rustdoc. It needs no Node and no
browser.

CLI socket tests require permission to listen on loopback, including in
sandboxes. A denied bind fails the test; supported CI hosts must run this coverage.

Then run the browser suites for the area you changed:

| If you changed | Run |
| --- | --- |
| Compiler errors or diagnostics | `just test consumer`, `just test authoring` |
| Compiler backend, construction or output contracts | `just test external-backend`, `just test component-tags`, plus the affected structural suites below |
| Components, lists, children | `just test-browser`, `just test foreach`, `just test children`, `just test component-tags` |
| Conditions, async, coherent views | `just test control-flow`, `just test async-components`, `just test coherent` |
| Routing | `just test router`, `just test navigation` |
| Islands | `just test islands`, `just test delivery`, `just test component-hydration` |
| JavaScript modules and npm | `just test javascript`, `just test javascript-build`, `just test javascript-dev`, `just test integrations` |
| Workers, tasks, shared pools, or streams | `just test worker`, `just test worker-pool`, `just test worker-dev` |
| The CLI or the dev server | `just test dev`, `just test standalone` |
| Tailwind CSS compilation or stylesheet refresh | `just test tailwind` |
| The docs, landing page or benchmark site | `just test docs`, `just test docs-examples`, `just test-landing`, `just test benchmarks` |
| Benchmark tooling | `just test-tools`, `just bench verify` |

Some suites test output that was built beforehand, so rebuild it after changing
the code, or you will be testing the old version:

- `just test-browser` serves the playground: run `cargo fusor build -p fusor-playground`.
- `just test coherent`, `just test islands`, the site suites (`docs`,
  `benchmarks`, `test-landing`) serve prebuilt examples and the site: run
  `just fixtures`.

`just test authoring` also needs `rustup component add rust-analyzer`.

A few things that save time:

- The suites start their own servers. Stop anything already on port 4173, and
  run one suite at a time.
- `just test-browser` uses Chromium by default. Set
  `PLAYWRIGHT_BROWSERS=chromium,firefox,webkit` to run all three engines, and
  `PLAYWRIGHT_CHANNEL=chrome` to use an installed Chrome.
- Required CI runs `just check` on Linux, macOS and Windows, the Rust 1.85 build,
  and `just ci-browser` with Chromium, Firefox and WebKit. It checks framework
  behavior: Wasm loading, reactivity, DOM identity, events, ownership and cleanup,
  routing, forms, hydration, islands, compilation and CLI behavior.
- Full third-party demos and site walkthroughs run separately with
  `just ci-examples`, or through the **examples** workflow's **Run workflow**
  button in GitHub Actions. Run these when changing the examples or site.
  Graphics-heavy demos do not gate framework changes; small JavaScript
  integration fixtures still check the framework's ownership and lifecycle
  contracts in required CI.

## Guidelines

**Compiler.** New HTML behavior goes through the parser and the typed
representation in `crates/fusor-build/src/bindings/`, then gets lowered to Rust
with `quote!`. Don't build Rust source out of strings. Keep the authored spans
and line positions intact, so rustc's errors point at the right HTML. Add both a
valid and an invalid case, and when you generate new Rust, check it compiles in
a real application: a string assertion doesn't prove that.

**Runtime and DOM.** Test what a user can observe: the scheduling order, when
cleanup runs, and whether the same DOM nodes survive an update. Include the case
where a callback removes its own element. Changes to rendering should keep focus,
cursor position and accessibility behavior. If you change the shared template
format, bump its schema version.

**The CLI.** Read
[`crates/fusor-cli/ARCHITECTURE.md`](crates/fusor-cli/ARCHITECTURE.md) first.
It's short, and it lists the rules the crate is written to.

**Tooling.** Repository tooling is written in Rust, or in Node under `scripts/`.
Please don't add a third language.

**Benchmarks.** Follow [`benchmarks/README.md`](benchmarks/README.md) and read
the methodology before changing a measurement. Never change a workload to help
one framework's numbers.

**Dependencies.** Commit lockfile changes along with the change that needs them,
and don't update unrelated dependencies in the same pull request. The test
fixtures, `examples/npm` and `apps/docs/tutorial` are separate Cargo projects
that depend on the workspace crates by path; after changing a crate's
dependencies, run `just lock` and commit their updated lockfiles.

## Sending a pull request

Keep each pull request to one change. Small ones get reviewed faster, and it's
fine to split a large change into several.

In the description, say what problem you're solving and how the behavior
changes, and list the commands you ran to test it. For a bug fix, point to the
test that fails without your change. For a syntax or API change, include a short
before-and-after example.

Before you push:

1. `just check` passes.
2. The browser suites for the area you changed pass.
3. If the change is visible to application developers, the matching page in
   `apps/docs/public/content/` is updated.

## Releasing

Every crate shares one version: `version` under `[workspace.package]` in the
root `Cargo.toml`. A release publishes all of them at that version.

1. Bump the version, run `just check` so `Cargo.lock` follows, and merge that to
   `main` with CI passing.
2. On GitHub, publish a release whose tag is the version with a `v`, such as
   `v0.2.0`, on that commit.

The `Release` workflow then does three things:

- builds the CLI for Linux (x86-64 and ARM, statically linked), macOS (Apple
  Silicon and Intel) and Windows, checks each binary runs from its archive,
  smoke-tests it against a new application, and runs `install.sh` or
  `install.ps1` against the archive;
- once every platform passes, attaches the archives and their `.sha256`
  checksums to the release, which is where the installers download from;
- checks that the tag matches the version, runs `just check` and a dry run, and
  runs `cargo publish --workspace`, which uploads every crate in dependency
  order.

To try the binaries without releasing, run the `Release` workflow by hand from
the Actions tab. It builds and tests every platform and keeps the archives as
workflow artifacts, but publishes nothing.

One-time setup: create a crates.io API token with the `publish-new` and
`publish-update` scopes, and save it as the secret `CARGO_REGISTRY_TOKEN` in a
GitHub environment named `crates-io`. Add required reviewers to that
environment if each release should wait for an approval.

crates.io lets an account publish only five brand-new crates in a burst, then
one every ten minutes. The workflow publishes in rounds: it skips crates already
on crates.io at that version and waits whenever crates.io says to slow down, so
the first release, fourteen new crates, takes about an hour and a half. Later
releases only update existing crates and finish in one round. If a run stops
anyway, rerun the job; it picks up where it left off.

## Deploying the site

fusor.build is the landing page, docs and benchmarks from `just site`, hosted on
Vercel. Vercel can't build it, since its build machines have no Rust toolchain,
so the site is built elsewhere and uploaded as finished files. Pushes don't
deploy.

To deploy, run the **Deploy site** workflow from the Actions tab and choose
production or preview. From your own machine, `just deploy` does the same once
you have run `vercel login` and `vercel link --scope pirela --project fusor`;
`just deploy preview` makes a preview deployment.

`vercel.json` sends the cross-origin isolation headers on every `/docs/` page,
because the docs' pool demo needs shared memory. Anything the docs embed must
come from the same origin or allow cross-origin embedding.

One-time setup: create a Vercel access token for the pirela team and save it as
the secret `VERCEL_TOKEN` in a GitHub environment named `vercel`.

## Using AI tools

You're welcome to use AI tools, but you're responsible for everything you
submit. Read and understand every line, make sure the tests you add would fail
without your change, and be ready to explain your reasoning in review. Please
don't open pull requests or issues generated without that review.

fusor's own development uses AI tools heavily too, mainly Claude Code, and the
maintainer works under the same rules.

## License

By contributing, you agree that your contributions are licensed under the
project's [MIT License](LICENSE).
