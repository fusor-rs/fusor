# CLI reference

Every `fusor` command, the options they share, the `Cargo.toml` settings they read, and the
exit codes scripts can rely on.

## Commands at a glance {#commands}

Run commands inside your application directory, or choose an application with
`--manifest-path` or `--package`.

`setup` is another name for `install`, and `serve` is another name for `preview`.

```sh title=Terminal
fusor new my-app      # create and prepare an application
fusor install         # fetch dependencies and matching tools
fusor doctor          # report problems; changes nothing
fusor add router      # add a first-party capability
fusor check           # type-check Rust and HTML
fusor build           # build a static site into dist/
fusor build --site    # build a workspace's applications as one site
fusor dev             # build, watch, serve and refresh
fusor preview         # serve an existing build
fusor expand          # print the Rust generated from HTML
```

## Create an application {#new}

`fusor new PATH` writes an ordinary Cargo project, resolves its lockfile, prepares the tools
and type-checks the starter. If preparation fails, the sources stay and `fusor install`
resumes it.

`--skip-install` writes the sources only. `--javascript` also adds a `package.json` with
esbuild and a starter `web/app.js`. `--framework-path` points the dependencies at a local
fusor checkout.

```sh title=Terminal
fusor new my-app
fusor new my-app --javascript
fusor new my-app --skip-install
```

## Prepare a fresh clone {#install}

`fusor install` fetches Cargo dependencies, adds the `wasm32-unknown-unknown` target if it
is missing, and installs the pinned wasm-bindgen. When the application has a `package.json`,
it restores `node_modules` from `package-lock.json` with `npm ci`.

It may create or update `Cargo.lock`. Pass `--locked` to require the committed lock
unchanged.

`fusor doctor` checks the same things without changing, installing or downloading anything.
It runs even when Rust is not installed.

```sh title=Terminal
fusor install --locked
fusor doctor
```

> wasm-bindgen is cached once per user, by version and host. Set `FUSOR_CACHE_DIR` to use
> another directory.

## Add a capability {#add}

`fusor add` adds the dependencies and browser features for `router`, `async`, `query`,
`forms`, `actions` or `javascript`. It edits `Cargo.toml` through `cargo add`, so your
comments and formatting stay. It never writes application source.

Every added package comes from the same place as `fusor`, whether that is the registry or a
local checkout. If a step fails, the files the command wrote are restored, unless you edited
them in the meantime.

`--dry-run` prints the changes without making them.

```sh title=Terminal
fusor add router
fusor add query --dry-run
```

> `query` also adds `async`, because its loaders receive a `CancellationToken` from
> `fusor-async`.

## Check and build {#check-build}

`fusor check` type-checks the Rust and the HTML without producing a site. Compiler errors
point at the HTML line they came from.

`fusor build` compiles a release build into the configured output directory, `dist/` by
default. `--debug` uses the debug profile. A failed build leaves the previous output in
place.

Both need an existing `Cargo.lock` and never change it.

```sh title=Terminal
fusor check
fusor build
fusor build --debug
```

## Develop with live refresh {#dev}

`fusor dev` prepares any missing tools, builds into `.fusor/dev`, serves the result and
rebuilds when you save. Your `dist/` directory is not touched.

An HTML or CSS change that leaves the generated Rust the same is patched into the open page,
which keeps its state. Any other change rebuilds and reloads the page. A failed build keeps
the last working one served and prints the error in the terminal.

A literal `include_str!` or `include_bytes!` can keep live refresh enabled when it reads a
watched file outside the app's HTML templates and public assets. Editing that file rebuilds
the app. Embedding a template or public asset, custom Rust file includes, and include paths
the CLI cannot resolve keep refresh disabled.

In a workspace that declares a site, `fusor dev --site` does the same for every mounted
application at once, served from one address.

```sh title=Terminal
fusor dev
fusor dev --port 8090 --open
fusor dev --site
```

> Set `dev-refresh = false` when custom build logic reads your HTML or assets.

## Preview a build {#preview}

`fusor preview` serves an existing build with no Cargo, Rust or npm. It uses the base path
and history fallback the build recorded. Pass a directory to serve something other than the
configured output.

```sh title=Terminal
fusor build
fusor preview
fusor preview dist --port 8092
```

> Servers use port 4173 unless you pass `--port`. A busy port fails before any build starts.

## Publish several applications as one site {#site}

A workspace can publish several applications on one domain, each at its own URL path, such
as a homepage at `/` and documentation at `/docs/`. Declare the mounts in the workspace
`Cargo.toml`, then run `fusor build --site`.

`fusor preview` serves the result as one site.

```toml title=Cargo.toml · workspace root
[workspace.metadata.fusor.site]
output = "dist"

[workspace.metadata.fusor.site.mounts]
"/" = "homepage"
"/docs/" = "docs"
```

> `--site` builds every mounted application, so it cannot be combined with `--package`.

- [How a site is built and laid out](/docs/sites)

## Read the generated Rust {#expand}

`fusor expand` prints the Rust generated from one HTML module, without compiling Wasm. Name
the module, or give its path relative to the application. The default is `app`.

```sh title=Terminal
fusor expand
fusor expand --module web/components/counter.html
```

## Options every command accepts {#options}

Inside a workspace member's directory, that member is chosen. When several applications are
in scope, the command lists them and stops.

Progress goes to stderr and results go to stdout, so `fusor preview | head -1` prints only
the URL.

```text title=Text
--manifest-path PATH   Cargo.toml of the application or its workspace
-p, --package NAME     choose an application in a workspace
--features LIST        Cargo features to enable
--offline              no network for Cargo, npm or tool downloads
--locked               require Cargo.lock to stay unchanged
--frozen               --offline and --locked together
--quiet                silence progress; results still print
--verbose              print more detail
--color WHEN           auto, always or never
```

> `CARGO_NET_OFFLINE=true` also turns on `--offline`. With `--color auto`, color is off when
> `NO_COLOR` is set.

## Configure an application {#configuration}

An application declares `[package.metadata.fusor]` in its `Cargo.toml`. Paths are relative
to the package, and unknown keys are rejected.

The values below are the defaults, except `assets`, which new applications set to `public`.

```toml title=Cargo.toml
[package.metadata.fusor]
entry = "web/index.html"        # the HTML entry page
templates = ["web/components"]  # searched for reusable HTML; [] turns discovery off
assets = "public"               # copied into the published site
assets-build = []               # a program and its arguments, run before assets are copied
output = "dist"                 # where fusor build publishes
base-path = "/"                 # the URL path the site is served under
history-fallback = []           # prefixes that receive index.html, for client-side routes
dev-refresh = true              # false when build logic reads HTML or assets
```

> Restart `fusor dev` after changing `base-path`. Island sites also declare a `delivery`
> table.

- [Set up island delivery](/docs/islands/setup)

`assets-build` must be an array: the first item is the executable, and each remaining
item is one argument. For example, to run an asset script from the package directory:

```toml title=Cargo.toml
[package.metadata.fusor]
assets = "public"
assets-build = ["node", "build-assets.mjs"]
```

The command runs without a shell. A string such as `"node build-assets.mjs"` is invalid;
`&&`, pipes (`|`), and environment variable expansion such as `$HOME` are not interpreted.
For more complex commands, put the build steps in a script and invoke it with the array
form. Use `[]` or omit `assets-build` to disable the hook.

## Environment variables {#environment}

`FUSOR_WASM_BINDGEN` wins even when it points at the wrong version; the command then reports
the mismatch instead of using another copy. Builds never download `wasm-opt` or search for
it.

```text title=Text
FUSOR_CACHE_DIR          where downloaded tools are stored
FUSOR_WASM_BINDGEN       use this wasm-bindgen; it must be version 0.2.117
FUSOR_WASM_OPT           run this Binaryen 132 wasm-opt on release builds
FUSOR_KEEP_WASM_NAMES=1  keep Wasm function names in release builds
FUSOR_NODE               the Node executable for bundling and island checks
```

## Exit codes {#exit-codes}

Scripts can tell a mistyped flag from a compile error without parsing messages. Errors print
on stderr, with a suggested fix on a `help:` line when there is one.

```text title=Text
0   success
2   usage: unknown flag, bad value, ambiguous or unknown application
3   project: the manifest, a lockfile or the output is not usable
4   tooling: a required tool is missing, the wrong version, or failed
5   compile: Rust or HTML compilation failed, or island registrations disagree
70  internal: a bug in fusor
```
