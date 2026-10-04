# Several applications on one site

Publish a homepage, documentation and other applications together on one domain, each at its
own URL path, with one build and one output directory.

## When to use a site {#when}

Use a site when several applications share a domain, such as a homepage at `/`,
documentation at `/docs/` and a dashboard at `/app/`. A single application does not need
one: `fusor build` publishes it on its own.

Each application stays its own Cargo package and compiles to its own WebAssembly module. A
visitor downloads only the application they are viewing, and following a link to another
application is an ordinary page load, so state does not carry across.

## Declare the mounts {#configure}

List the site in the workspace `Cargo.toml`. Each key under `mounts` is a URL path, and its
value is the name of a workspace package.

The path must match that application's own `base-path`, because the base path decides every
URL the application generates. If they differ, the build stops and names both values.

```toml title=Cargo.toml · workspace root
# Cargo.toml at the workspace root
[workspace]
members = ["homepage", "docs"]

[workspace.metadata.fusor.site]
output = "dist"

[workspace.metadata.fusor.site.mounts]
"/" = "homepage"
"/docs/" = "docs"
```

> `output` defaults to `dist` and must be a path inside the workspace.

## Match each application's base path {#base-path}

In each application's own `Cargo.toml`, set `base-path` to its mount. The homepage keeps the
default `/`.

```toml title=Cargo.toml · docs application
# docs/Cargo.toml
[package.metadata.fusor]
base-path = "/docs/"
history-fallback = ["/"]
```

## Build the site {#build}

`fusor build --site` builds each mounted application exactly as `fusor build` would, into
`target/fusor/site/<package>/`. Only when every application has built does it copy each one
into the site output at its mount and publish the result.

Publishing replaces the previous site in one step. If any application fails to build, or two
mounts collide, the previous site stays in place.

```sh title=Terminal
$ fusor build --site
▲ fusor 0.1.4 · production build
○ Compiling homepage ...
✓ Compiled homepage in 12.4s
○ Compiling docs ...
✓ Compiled docs in 18.0s
✓ Published dist/ in 30.6s
  /       homepage  → dist/
  /docs/  docs      → dist/docs/
```

## Read the output {#layout}

Each application is copied to the directory named by its URL path, so the output has the
same shape as the URLs. The application at `/` fills the top level, and every other
application gets its own directory.

Each application keeps its own page, assets and generated files. An application's files
cannot occupy another application's mount: a homepage asset folder named `docs/` is an
error, not a merge.

```text title=Text · site output
dist/
  .fusor-site.json        which application serves each path
  index.html              the homepage
  __fusor/…               the homepage's JavaScript and Wasm
  styles.css              the homepage's assets
  docs/
    index.html            the documentation
    __fusor/…             the documentation's JavaScript and Wasm
    docs.css              the documentation's assets
```

## Routes and links {#routing}

Routes inside an application are written relative to its base path. A documentation route
whose path is `/installation` is served at `/docs/installation`; the router adds and removes
the base path for you. `history-fallback` prefixes are relative to the base path too.

Links to another application are ordinary absolute URLs, such as `/docs/`, because they
leave the current application.

## Develop the site {#develop}

`fusor dev --site` runs the development loop for every mounted application together. Each
application builds into its own `.fusor/dev`, has its own watcher, and is served at its
mount from one address, so links between applications work while you edit.

An edit to one application's files rebuilds or refreshes only that application; a change to
a shared crate rebuilds each application that uses it. Use `fusor dev -p <package>` to work
on one application alone.

```sh title=Terminal
fusor dev --site
```

## Preview the site {#preview}

`fusor preview` serves a site output as one site. Each request goes to the application with
the longest matching URL path, which answers with its own history fallback. It prints the
same table of mounts before serving.

```sh title=Terminal
fusor preview dist
```

## Deploy {#deploy}

Upload the contents of the site output as they are. Because the folders match the URLs, any
static host serves them without path rewrites.

> Client routes such as `/docs/installation` have no file of their own. As with a single
> application, configure your host to answer them with that application's `index.html`, for
> each prefix in its `history-fallback`.
