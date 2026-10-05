# fusor landing page

A fusor application for the public homepage, with selectable working examples
and their exact highlighted HTML and Rust source files. HTML owns the layout; Rust owns the
interactive state. There is no JavaScript UI framework or remote font dependency.

## Build and preview

The landing page is served at `/`, beside the docs at `/docs/` and the benchmark
results at `/benchmarks/`. All three are built together as one site, declared in
the workspace `Cargo.toml` under `[workspace.metadata.fusor.site]`:

```sh
just site
just preview
```

Open <http://127.0.0.1:8080/>. `just site` runs `fusor build --site`, which
builds each application and publishes them together into `dist/`. `just preview`
runs `fusor preview dist`, which sends each request to the application whose
base path matches it, with that application's own history fallback. Deploy the
contents of `dist/` as they are.

For development, `just dev` serves all three with live reload at
<http://127.0.0.1:4173/>. To work on the page alone, run `just dev-app fusor-landing`.

## Installer assets

The build copies the repository-root `install.sh` and `install.ps1` into
`public/`. The assembled site includes them as `dist/install.sh` and
`dist/install.ps1`, served directly at `/install.sh` and `/install.ps1`.
Edit the root scripts; the generated copies are ignored by Git and refreshed
when their sources change or the copies are missing.

After deploying the site, users can install with:

```sh
curl -fsSL https://fusor.build/install.sh | sh
```

On Windows, in PowerShell:

```powershell
irm https://fusor.build/install.ps1 | iex
```

Both installers discover the latest GitHub release when run, download the
appropriate archive, and verify its checksum. Publishing a new release requires
no site update. Changes to the installer scripts themselves require a site
rebuild and deployment.

## Application structure

- `web/index.html` and `src/app.rs`: page, example and source-file controls, and
  the install-command clipboard interaction.
- `src/examples.rs`: the example registry, descriptions, and related guide links.
- `src/{counter,search,keyed_list,async_data}.rs` and matching templates under
  `web/components`: four independently owned live components. Choosing a different
  example disposes the previous one and starts the next with fresh state. Changing
  the displayed source file leaves the running example intact.
- `host/{counter,search,keyed_list,async_data}/web/index.html` and matching
  `src/app.rs`: for each example, a short page that mounts and imports only that
  component inside `<App>`, shown under the "Page" file group in the example
  editor. These files are displayed and downloadable but not compiled into this
  app, which has its own entry page. Template discovery only scans `web/components`,
  so the host pages are never picked up by this build.
- `build/highlight.rs`, `src/code.rs`, and `web/components/code.html`: build-time
  Syntect highlighting rendered as escaped text tokens. No runtime highlighter or
  raw HTML injection. Colors have at least 4.5:1 contrast against the code background.
- `public/landing.css`: responsive styles, focus states, and reduced-motion support.
- `public/data/issues`: clearly labeled local fixtures for the async example.
- `build.rs`: highlighting, standard HTML compilation, installer assets, and exact
  plain-text source copies. Generated copies in `public/` are ignored by Git.

Counter is the first example because it shows the whole model in two short files:
the HTML calls `increment` and `reset` methods on a Rust struct, and one binding
displays the count. A key below the source explains the four pieces of template
syntax visitors will see. The example files are complete reusable components, not
whole apps, so the editor's tab bar has two labeled groups: "Component" (the
selected example's HTML and Rust) and "Page" (that example's host `index.html` and
`app.rs`). One sentence above the key names the selected component's tag and says
the page places it inside `<App>`, which starts the app; a one-line note below says
what `fusor new` adds. Choosing another example while a page file is open keeps that
page file open, so it switches to the new example's page. From a component file, it
opens the new component's HTML. At phone
widths each group's label sits above its two files. Live search filters real guide links using a bound input.
Keyed lists demonstrate that notes survive reordering of the actual DOM nodes.
Async data reads a title and status independently after 250 ms and 1,000 ms delays;
`<Async>` creates and owns its coherent boundary, retaining the previous result
until both reads succeed. No boundary field or initialization is needed. The
framework marks the result busy while work is pending; CSS dims it. Status and
retry controls using an optional explicit handle are covered in the linked guide.

The design uses system fonts, the selected fusor F emblem, an oxide accent, and a
compact source-and-result view. Header navigation and the logo stay visible at
phone widths. [Branding](BRANDING.md) records the selected artwork and palette.
The hero pairs the introduction with platform-specific installation and a copy
button. A closing call to action shows the commands to create and run an app.
The “More from Fusor” section links to HyperCMD and SQL Bomb, explaining
how they use fusor. Their horizontal logos in `public/brand/` come from each
project’s `assets/brand/` directory.

## Verification

The browser checks use the repository's Playwright installation. Build first, then:

```sh
node apps/landing/tests/browser.mjs
BROWSER=webkit node apps/landing/tests/browser.mjs

cargo fmt --manifest-path apps/landing/Cargo.toml --check
CARGO_TARGET_DIR="$PWD/apps/landing/target" \
cargo clippy --manifest-path apps/landing/Cargo.toml \
  --target wasm32-unknown-unknown --locked -- -D warnings
```

Each browser run starts its own temporary preview server. Tests check that Counter
opens first with its HTML shown and that the source key renders template syntax
literally. They exercise search
and empty states, keyed input/DOM retention, async coherent publication,
supersession, failed refresh retention and recovery, switching away during a request, counter updates,
keyboard interaction, and all examples at six widths from 320 to 1440 pixels.
They compare all eight component files and all eight host page files, and their
downloadable copies, with the sources on disk. They check that only one file tab
is active, that each host page mounts and imports only its own component, that an
open page file follows a newly selected example, that viewing the page files keeps
the running example's state, syntax-color contrast, and that documentation links
resolve. Chromium also checks the clipboard. Screenshots are saved by browser in
the ignored `test-results` directory. Docs and benchmark builds are not required.
