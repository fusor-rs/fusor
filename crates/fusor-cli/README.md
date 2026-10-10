# Fusor CLI

One native CLI for creating, checking, building and developing applications
written as HTML with ordinary Rust inside it.

The framework crates and the binary installers are not published yet. Install
from a checkout and point new projects at it:

```sh
cargo install --path crates/fusor-cli --locked
fusor new my-app --framework-path /path/to/rust-front
cd my-app
fusor dev
```

`new` writes an ordinary Cargo project, resolves its lockfile, prepares the
tools and type-checks the starter before telling you it is ready. Rust and your
platform's native build prerequisites need to be installed already.

## The four commands

```sh
fusor dev       # build, watch and serve on localhost with live reload
fusor check     # type-check Rust and HTML
fusor build     # build a deployable static site, normally into dist/
fusor preview   # serve an existing build, with no Cargo and no Rust
```

And three you will reach for early:

```sh
fusor install     # prepare a fresh clone: dependencies and matching tools
fusor add router  # declare a first-party capability
fusor doctor      # report problems without changing anything
```

Applications build on Rust 1.85 or later; new projects pin the tested 1.95.0
toolchain and the Wasm target. Browser builds with JavaScript modules, workers,
or islands need Node.js 22 or newer to bundle modules or inspect registrations.
Native Cargo checks and artifact-only preview do not need Node.

## Reference

Every command, option, setting and exit code is on the CLI reference page of the
guides in [apps/docs](../../apps/docs), which also cover authoring.

Contributors: [ARCHITECTURE.md](ARCHITECTURE.md) has the module map and the
rules this crate is written to.
