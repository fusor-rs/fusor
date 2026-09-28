# fusor-cli architecture

## Rules

1. **Commands live in `commands/<verb>.rs`.** `cli.rs` holds argument types and
   no logic. There is one dispatch, in `commands/mod.rs`, and whether a command
   needs a resolved application is visible in its arm.
2. **Errors are `error::Error`**, with a `Kind` that selects the exit code and
   an optional `remedy`. Put the fix in the `remedy` field, never as another
   clause in the message. One imperative sentence.
3. **Output goes through `Reporter`.** No bare `println!` or `eprintln!` in
   command or pipeline code. Progress and diagnostics go to stderr; anything a
   user might pipe, such as a URL or a built path, goes to stdout. Commands
   print a `▲ fusor` banner, then `○` when work starts, `✓` when it finishes
   and `✗` when it fails; Cargo's own progress appears only under `--verbose`.
   The exceptions: `pipeline/diagnostics.rs` and `pipeline/cargo.rs` forward
   the compiler's own output verbatim, and `error.rs` renders a fatal error,
   which must work before a `Reporter` exists.
4. **Generated paths and pinned third-party versions are constants in
   `layout.rs`.** No `"__fusor"` spelled out in a format string.
5. **External data is deserialized into types at the boundary.** Cargo
   metadata, output manifests, witnesses and the npm bundle graph have
   structs. No `value["key"]["key"]` navigation in logic. The exception is
   Cargo's `--message-format=json` stream in `pipeline/cargo.rs` and
   `pipeline/diagnostics.rs`: only a few fields are read, and rustc's spans
   are walked recursively.
6. **Functions stay under about sixty lines.** One that grows past that is
   doing two things. An immediately-invoked `(|| -> Result { … })()` is a
   signal to extract a function with a cleanup caller, not a pattern to copy.
7. **Contributor-only commands are hidden.** `repo check` carries
   `#[command(hide = true)]`.
8. **Repository tooling is written in Rust or in Node under `scripts/`.** Not a
   third language. See `CONTRIBUTING.md`.

## Module map

```
src/
  main.rs          entry point; maps an Error to an exit code
  lib.rs           run(), module wiring
  cli.rs           clap types only
  context.rs       Context: settings after the flags interact
  error.rs         Error { message, remedy, kind } and the exit-code contract
  reporter.rs      the one output channel
  layout.rs        every generated name and pinned version
  process.rs       cargo(), rustup(), checked()
  transaction.rs   Staging and OwnedFile: how a partial write is undone
  commands/        one module per verb, plus the hidden repo check
  workspace/       Project, typed cargo metadata, application selection
  toolchain/       Rust, the Wasm target, wasm-bindgen, npm
  pipeline/        Publication, sites, Cargo, wasm-bindgen, diagnostics, output
  dev/             server, HTTP policy, watcher, fast refresh
```

## Before changing these

**Staged publication** (`pipeline/publish.rs`). A failed build must leave the
last successful site serving. Work happens in a staging directory beside the
output, publication is two renames, and a failure between them restores the
previous site. `transaction::Staging` removes the staging directory on any
early return.

**Immutable generations** (`pipeline/mod.rs`). Everything a build generates goes
under `__fusor/<generation>/`. A page loaded before a rebuild keeps resolving
its own JavaScript and Wasm, because the previous generation is retained for one
build. This is why generation directories can be cached forever and the rest
cannot.

**Application selection** (`workspace/select.rs`). `doctor` and `preview` must
work on a host with build output and no Rust, so candidates come either from
`cargo metadata` or from reading manifests. Both feed `choose`, which owns the
`--package` filter, the tie-break and the ambiguity message. The
`[package.metadata.fusor]` test exists once per backend and the two must agree.

**Fast refresh** (`dev/refresh.rs`). An edit can skip Cargo only when the Rust
the browser is already running is unchanged, compared as token trees *and*
locations. Anything unclear (a source that does not tokenize, an unresolved
file include, a JavaScript module) falls back to a normal build. `fusor-build`
recognizes its generated includes and direct literal data includes. The CLI
may clear a data include only when its file remains watched across publications
and lies outside authored HTML and public assets; editing that file still
requires a normal build.

## Exit codes

Documented for scripts on the CLI reference page in `apps/docs`.
`tests/exit_codes.rs` pins the usage and project codes. Adding a `Kind` means
documenting its code on that page.
