# AGENTS.md

Rules for coding agents in this repository. Read this first, then the document
it points to for the area you are changing. These are requirements, not
suggestions. If a rule and the task truly conflict, explain the conflict rather
than quietly bending either. `CONTRIBUTING.md` is the source of truth for setup,
checks and workflow.

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
   wasm-bindgen and optional npm bundling, then publishes the local build output.

Note: the crate `fusor-core` has library name `fusor` (the crates.io name was
taken), so code says `use fusor::prelude::*`.

Template syntax at a glance: `{{ expr }}`, `on:event`, `class:name`, `bind`,
`prop:name`, `rust:if`, `rust:key`, `rust:render`, `hydrate*`, plus the built-in
tags `App`, `If`/`Else`, `Match`/`Case`, `ForEach`, `Children`, `Async`, `Await`,
`Router`/`Route`. The full reference is in `apps/docs/public/content/html-and-rust/attributes.md`
and `apps/docs/public/content/html-and-rust/built-in-components.md`.

## Repository map

- `crates/fusor-core`: signals, ownership/cleanup and the optional DOM runtime.
  `src/template.rs` is the versioned compiler/runtime template contract.
- `crates/fusor-build`: the HTML compiler, typed IR, shared semantic lowering,
  DOM and external backend emission, and source maps.
- `crates/fusor-macros`: `FromInputs` and `JsInputs` derives.
- `crates/fusor-components`: built-in tags (`App`, `ForEach`, `If`, `Router`, …).
- `crates/fusor-cli`: the `fusor` CLI (`new`, `dev`, `build`, `check`, `preview`,
  `add`, `doctor`, …). Hidden `repo check` backs `just check`. Read its
  `ARCHITECTURE.md` before editing.
- `crates/fusor-async`, `crates/fusor-query`, `crates/fusor-router`,
  `crates/fusor-std`: optional resources, queries, routing and forms/actions.
  `fusor-std` re-exports packages behind features.
- `crates/fusor-worker`, `crates/fusor-worker-macros`: background tasks,
  stateful workers, pools, streams and their generated clients.
- `crates/fusor-islands`, `crates/fusor-server`: islands and server-rendered HTML.
- `crates/fusor-npm`: internal npm bundling for the CLI.
- `crates/fusor-test`: deterministic requests, clock, executor and lifetime probes.
- `crates/fusor-release`: contributor tooling for release archives.
- `apps/landing`, `apps/docs`, `apps/benchmarks`: the fusor.build site at `/`,
  `/docs/`, `/benchmarks/`; configured in `[workspace.metadata.fusor.site]`.
- `apps/docs/tutorial`: a separate Cargo project the guides compile against.
- `examples/`: runnable apps; most are workspace members. `examples/npm` is
  excluded from the workspace.
- `benchmarks/`: seven-framework workloads, harness and published records.
- `tests/fixtures/`: standalone consumer projects, including the executable
  external renderer in `tests/fixtures/external-backend`.
- `tests/tooling/*.mjs`: Node browser/tooling suites, run with `just test <name>`.
- `tests/browser/`: Playwright tests for the playground.
- `scripts/`: Node repository tooling.

`dist/`, `target/`, `test-results/`, `apps/docs/public/source/` and several
files under `apps/benchmarks/public/` are generated; don't edit them by hand.

Constraints that hold everywhere:

- Rust edition 2024, MSRV 1.85; CI builds on 1.85 too, so don't use newer std
  or language features. `unsafe_code` is forbidden workspace-wide.
- Keep the reactive core and external renderer contracts usable without DOM
  features. Keep optional packages and features isolated; a workspace build
  can hide missing dependencies through feature unification.
- Repository tooling is Rust, or Node under `scripts/`. No third language.
- New tasks go in the `justfile`, not npm scripts. `package.json` only declares
  dependencies.
- `wasm-bindgen` is pinned exactly and must match `BINDGEN_VERSION` in
  `crates/fusor-cli/src/layout.rs`. Change both together.
- All crates share one version in `[workspace.package]`.

## The standard

Write the smallest correct change a careful senior reviewer would merge.
Assume your first draft contains unnecessary code, duplication and abstraction;
remove them before you finish.

- **Less code, never compressed code.** Between equally readable, correct
  solutions, take the one with less logic. Deleting code is progress. Every
  added line must be needed by the task; report the net line change. Squeezing
  the same logic into denser lines is worse code. Readability wins.
- **Write for the maintainer.** Passing checks is the minimum. The code must
  remain easy to read, extend and debug by someone who never saw the task.
- **Read before writing.** Before adding a function, type, constant or
  dependency, search for one that already does it with `rg` and use or extend it.
- **Verify, don't assume.** Check APIs against their source or documentation
  for the resolved version, including `~/.cargo/registry/src/`. Never invent a
  method or dependency. Before calling code unused, search for its uses.
- **Run it.** Compilation alone does not prove behavior. Execute the behavior
  you changed, or a test that fails without the change, and say what you ran.
- **Fix causes, not symptoms.** Reproduce bugs first. Don't add a check, retry,
  sleep or fallback to hide a failure you do not understand.

## Scope and integrity

- **Do exactly the task.** Change the smallest region that achieves it. Don't
  rewrite, refactor, rename, reformat or modernize unrelated code. Existing
  violations elsewhere are not an invitation to clean up the repository.
- **Don't break what's next to it.** Before editing shared code or configuration,
  find its other users with `rg` and preserve behavior outside the task. Run the
  full checks as well as the tests near your change.
- **Finish or say so.** Never present partial, stubbed or untested work as done.
  State exactly what is missing or could not be verified.
- **Don't expand the task.** Report unrelated problems in one line, unfixed.
  Don't end with offers of adjacent work.
- **Never game a check.** Don't special-case test inputs, hardcode expected
  outputs, or delete, skip, ignore or weaken tests, assertions or fixtures to
  make failures disappear. If the task deliberately changes a contract, update
  its tests to verify the new behavior. Otherwise report a suspect test and why
  it appears wrong.
- **Never fake the result.** Build the mechanism requested: no canned data in
  place of computation, mocks in production paths or output staged as success.
- **Never destroy work outside the task.** No deleting unrelated files or data,
  and no `rm -rf`, `git reset --hard`, `git clean`, `git checkout --` on
  uncommitted work or force pushes without explicit permission.
- **Match the repository.** Follow its structure, naming and idioms. Don't
  introduce a second style or architecture for the same kind of thing.

## Hard rules

### Duplication and abstraction

- No near-duplicate blocks. Repeated implementations of one concept become a
  function, loop or table. Similar shapes implementing different concepts may
  stay separate.
- One concept, one implementation. Extend the existing parser, validator,
  formatter or error type instead of adding a parallel path.
- One source of truth per constant. Define each limit, name, path or version
  once and reference it in logic and messages. Tests may use independently
  derived literals to verify the contract.
- No speculative generality. Don't add traits, generics, builders, flags or
  parameters without a current use. Public extension points such as renderer
  traits need a concrete consumer and the design discussion required by
  `CONTRIBUTING.md`; one in-tree implementation alone does not make them unused.
- No trivial wrappers. A function must remove duplication or name a real domain,
  ownership or API boundary. Don't add forwarding layers just to rename calls.
- No vague `utils`, `helpers`, `common` or `misc` modules, or `Manager`, `Helper`,
  `Util`, `Data`, `Info`, `Impl` or `Wrapper` type names. Name the domain concept.

### Readability

- One statement per line; use the repository's `rustfmt` defaults for Rust.
  Don't compress several steps into a one-liner.
- Names use full domain words: `pending_job`, not `pj`, `tmp`, `res` or `data2`.
  Single letters are for counters, coordinates and conventional type parameters.
- No unexplained numbers or strings in logic. Name domain constants.
- Long text, templates and CSS belong in their own files or readable multiline
  literals. Application components remain ordinary `.html` and Rust files.
- Propagate errors with `?`; don't repeat check-and-unwrap boilerplate.
- Check repeated guards once at the loop or function head.
- When branches build nearly the same value, build it once and vary the part
  that differs. Repeated routes, columns or cases belong in a table and loop.
- Decompose by concept. A function uses one level of abstraction; an
  orchestrating function is a short sequence of named steps. A file holds one
  coherent concept.

### Functions and types

- Functions stay under 60 lines, with at most 5 parameters and 3 levels of
  nested blocks inside a function. Flatten with early returns and `let … else`
  before extracting functions; split by concept, not line count. Values that
  travel together become a struct.
- No boolean parameters that switch behavior (`render(node, true)`). Use named
  operations or an enum; boolean values such as a checkbox's state are data.
- Make invalid states unrepresentable: enums instead of string or integer tags,
  newtypes for identifiers and units, no `Option<Option<T>>` or paired `Option`s
  that must be set together.
- Default visibility is private, then `pub(crate)`. Use `pub` only for intended
  public APIs and required generated-code contracts.
- No intermediate variables that merely rename an expression. A binding must
  clarify meaning, evaluation order, borrowing or ownership. Clone a handle for
  a closure only when its lifetime requires independent ownership.

### Errors

- Never swallow a failure. No `.ok()`, `unwrap_or_default()`, `unwrap_or(…)`,
  `let _ = …` or empty arm on a `Result` unless ignoring that specific failure
  is an expected outcome, explained in a short invariant comment.
- No `unwrap()` outside tests. `expect("…")` is only for guaranteed invariants;
  its message states the invariant.
- Library code returns typed errors. `Box<dyn Error>` and `anyhow` belong in
  binaries. Diagnostics explain what failed and how to fix it; the CLI keeps
  the fix in `remedy` as required by its architecture.
- Don't defensively re-check states the types exclude. Validate external input
  at the boundary, then express the invariant in the type.

### Comments and documentation

Default to no implementation comments. Names and types carry meaning. A comment
is useful only when removing it would let a competent reader make a wrong
change: an invariant, ordering or ownership requirement; an external constraint
with a link when available; or the reason an obvious alternative is unsuitable.

- No narration, section banners, commented-out code or comments explaining
  each line of a test.
- Rustdoc documents public contracts: usage, units, errors, panics, ownership,
  cleanup and feature requirements that signatures cannot express. Keep useful
  module docs and runnable examples. Don't add docs that only repeat a name or
  signature, and don't strip contract documentation to meet a comment budget.
- No process comments such as "now uses", "updated", "fixed", "previously",
  "new:" or "as requested". History belongs in the commit message.
- No `TODO`, `FIXME`, `todo!()` or `unimplemented!()` in a finished change.
- Fix or delete comments made inaccurate by your edit.

Docs describe current behavior plainly. No unsolicited work logs, progress
files, AI summaries, changelog tense ("now supports") or vague non-claims.

### Dependencies and performance

- Prefer the standard library. A new dependency must provide functionality the
  task needs, have a confirmed API and an exact version, and support the MSRV.
  Reuse workspace dependencies instead of declaring competing versions.
- Update lockfiles only as needed. After changing crate dependencies, run
  `just lock` for the separate Cargo projects and include those lockfiles with
  the change. Don't bump unrelated dependencies.
- Don't add a clone, `to_vec`, `collect` or `String` allocation just to placate
  the borrow checker or iterate again. Borrow or restructure; preserve necessary
  ownership for reactive callbacks and generated closures.
- No `async`, `Arc`, `Mutex` or threads without a concurrency requirement.
  Keep single-threaded reactivity separate from worker concurrency.
- No optimization without measurement, and no repeated rebuilding, parsing or
  sorting of unchanged data in a loop.

### Tests: only what guards behavior

- Write the fewest tests that catch real regressions. Extend existing cases
  before adding tests; use tables for variations. A bug fix needs a focused
  reproducer. Compiler changes also need the valid and invalid cases below.
- Test observable contracts, not private helper structure or incidental call
  order. Scheduling and cleanup order are observable framework behavior.
- Don't test trivial getters, derived implementations, dependencies or facts
  already guaranteed by the type system. Documentation-only edits do not need
  invented behavior tests.
- Expected values are derived independently, never by repeating the code under
  test or calling it to compute the expectation.
- Don't mock your own code. Fake true external boundaries such as network or
  time only when real or in-memory behavior is impractical. Use `fusor-test`
  controls for framework requests, clocks and lifetimes where applicable.
- Assert exact behavior, not merely `is_ok()`, "not empty" or "doesn't panic".
  If an assertion is in a callback, also prove the callback ran.
- Verify regression tests fail without the implementation change. Use an
  isolated copy or a reversible patch; never discard unrelated uncommitted work.
  Report when that verification could not be performed.
- No test-order dependence or fixed sleeps to hide races. Use deterministic
  controls where possible. Browser, worker and dev-server tests should wait for
  observable state or output with a deadline.
- Names state behavior (`rejects_duplicate_keys`), not implementation
  (`test_parse_2`).

### Suppressions

- Don't silence handwritten code with `#[allow(…)]`. Fix it; a justified
  exception uses a narrow `#[expect(lint, reason = "…")]`.
- Never weaken a lint, threshold or check to make a change pass.
- Never edit generated code by hand or hide generator defects at a template
  call site. Fix the owning emitter in `fusor-build`, `fusor-macros` or
  `fusor-worker-macros`. If a lint cannot apply to generated scaffolding, emit a
  narrowly scoped allowance with its reason. Don't suppress diagnostics on
  authored expressions; compile a real consumer to verify the emitted code.

## Framework contracts

### Compiler and backends

- New HTML behavior goes through the parser and typed IR, then `quote!`. Never
  build Rust source from strings.
- Preserve authored spans and line positions so rustc errors point at the HTML.
- Add a valid and an invalid case. When generating new Rust, prove it compiles
  in a real application and execute the changed behavior. A generated-source
  assertion alone is not enough. Use the relevant `tests/fixtures/` consumer and
  `tests/tooling/` suite, not a replacement test-only compiler path.
- Fusor owns semantic lowering, captures, input construction and factories.
  Keep DOM and external backends on the shared lowering path. Backend-specific
  code owns rendering operations, not duplicate parsing or reactive semantics.
- Changes to the external backend callback or generated-code contract bump
  `VERSION` in `crates/fusor-build/src/backend.rs` and exercise
  `tests/fixtures/external-backend`. Preserve feature isolation from the DOM.

### Runtime and DOM

- Test observable behavior: scheduling order, when cleanup runs, DOM node
  identity across updates. For event or lifetime changes, include a callback
  that removes its own element.
- Preserve focus, cursor position and accessibility behavior.
- Changing the shared template format means bumping `VERSION` in
  `crates/fusor-core/src/template.rs`.

### CLI

Follow `crates/fusor-cli/ARCHITECTURE.md`, including its documented exceptions:
commands in `commands/<verb>.rs` with dispatch in `commands/mod.rs`; errors are
`error::Error` with the fix in `remedy`; all output through `Reporter` (no bare
`println!`); generated paths and pinned versions live in `layout.rs`; external
data is deserialized into types. Adding an error `Kind` means documenting its
exit code on the CLI docs page.

### Docs and benchmarks

- User-visible changes update the matching page in
  `apps/docs/public/content/`. See `apps/docs/README.md` for Markdown authoring,
  navigation and source-file inclusion.
- Follow `benchmarks/README.md` and `benchmarks/METHODOLOGY.md`. Never change a
  workload to help one framework's numbers. To run or publish benchmarks, use
  the `benchmarks` skill in `.claude/skills/benchmarks/`.

## Lint configuration

The root `Cargo.toml` defines workspace lints; crates opt in with
`[lints] workspace = true`. It currently forbids `unsafe_code`. `just check`
runs Clippy with `-D warnings`. Separate Cargo projects under `tests/fixtures/`,
`examples/npm` and `apps/docs/tutorial` do not inherit root workspace lints.

The rules above are review requirements, including function-size, parameter and
nesting limits. They are not all automated: this repository has no
`clippy.toml` or workspace Clippy deny list. Do not describe an unconfigured
lint as enforced. Changes to enforcement must account for the separate projects.

## Checks and commands

Run this before considering a change done:

```sh
just check
```

It runs formatting, workspace tests, `fusor-std` feature isolation, Clippy,
out-of-workspace consumer builds, executable external-renderer contracts and
rustdoc with warnings denied. It needs no Node or browser. CI separately checks
Rust 1.85; `just check` uses the active toolchain.

Every task is a `just` recipe; run `just` to list them. Recipes use `cargo
fusor`, an alias in `.cargo/config.toml` that runs the CLI from this checkout.

- `just setup-browser` installs framework browser prerequisites; `just setup`
  also prepares the full examples and docs, including the nightly toolchain
  used by threaded worker pools.
- `just test <suite>` runs a suite from `tests/tooling/`. Pick suites by area
  using the table in `CONTRIBUTING.md` ("Running the tests").
- `just test-browser` runs Playwright for the playground. Rebuild first with
  `cargo fusor build -p fusor-playground`.
- `just fixtures` rebuilds what `coherent`, `islands`, `docs`, `benchmarks` and
  `just test-landing` serve. Without it those suites test stale output.
- `just test-tools` runs Node tooling tests.
- `just ci-browser` is the required framework browser gate; `just ci-examples`
  runs the separate full site and third-party demonstrations.
- `just dev` / `just dev-app <package>` serves development on
  http://127.0.0.1:4173; `just site` / `just preview` builds and serves `dist/`.

Suites start their own servers: make sure port 4173 is free and run one suite at
a time. `just test authoring` needs the `rust-analyzer` component. Report failed
or skipped checks plainly; don't fix unrelated failures or weaken the gate.

## Final review

Review the actual diff before reporting completion:

```sh
git diff --stat
git diff --numstat
git diff --check
git diff | rg -n '^\+.*(unwrap\(\)|\.ok\(\)|unwrap_or_default\(\)|let _ =)'
git diff | rg -n '^\+.*(#\[allow|todo!|unimplemented!|dbg!|TODO|FIXME)'
git diff | rg -ni '^\+\s*//.*\b(now|updated?|fixed|previously|new:|step [0-9]|as requested)\b'
git diff | rg -n '^\+\s*(//|/\*|#\[test\])'
git diff | rg -n '^\+.{101,}'
```

These searches flag lines to review, not proof of a violation. Justify or remove
each hit against the rules above, including added comments, tests and lines over
100 characters. Re-read every changed hunk and confirm:

1. Nothing duplicates an existing concept inside or outside the diff.
2. Every new function, type, parameter and dependency has a current use.
3. Changed behavior was executed or covered by a test that fails without the
   change, and tests guard distinct regressions.
4. The diff contains nothing outside the task.
5. Changed functions read top to bottom without dead code, empty branches,
   leftover experiments or stale names.
6. The implementation delivers the requested mechanism, not a staged result.

## Reporting

Say what changed, what you ran and what it showed, the net line change, and
anything you could not verify. Report failures and skipped checks plainly;
never claim a check passed if you did not run it.

## Commits and pull requests

- Commit messages are a short imperative sentence in sentence case, no prefix
  (e.g. "Show page source for the selected landing example").
- No `Co-Authored-By`, "Generated with" or other AI attribution in commits or
  pull requests.
- Don't commit, push, tag, deploy or release unless asked.
- One change per pull request. Describe the problem, the behavior change and
  the commands you ran (see `.github/pull_request_template.md`). Include a
  before/after example for syntax or API changes; for a bug fix, name the test
  that fails without it.

## Deploying and releasing

The site deploys to Vercel only through the **Deploy site** workflow or `just
deploy`; pushes don't deploy, and Vercel can't build it (no Rust toolchain).
Releases are cut by publishing a GitHub release tagged `v<version>`. Don't
deploy or release unless asked. Details are in `CONTRIBUTING.md`.
