# fusor landing page

A fusor application for the public homepage of the Fusor lab. On a large display
it is one screen: a strip for Hughie's waitlist across the top, what the lab is and
an index of everything it makes on the left, one object on stage on the right, and
fusor's install command along the bottom. A
second page, `/made-with-fusor`, is a gallery of hobby projects built with fusor,
each badged with the lab's tool it is built with.
HTML owns the layout; Rust owns the interactive state; Tailwind styles both. There
is no JavaScript UI framework or remote font dependency.

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

- `web/index.html` and `src/app.rs`: the page and its two routes, the selected
  project, whether the object on stage is turned over, and the install-command
  clipboard interaction. `/made-with-fusor` is listed in `history-fallback` for
  `fusor dev` and `fusor preview`, and rewritten to `/index.html` in the repository's
  `vercel.json` for the deployed site.
- `src/project.rs`: everything the lab makes, as a table. Each project has its
  index entry (name, kind, the day it last shipped), its caption (status, summary,
  links), the frame shape and the page's world while it is on stage, and for tools
  the source files on the back of the frame, plus the lab's tools it is built with
  (the gallery's badges, with each tool's mark). Add a project there and to a group
  in `GROUPS` for the index, or to `MADE_WITH` for the "Made with fusor" gallery.
- `src/greeting.rs` and `web/components/greeting.html`: the live component in the
  browser frame. Its strip shows the Rust `String` it binds, `Debug`-formatted.
- `shown/`: files shown on the back of the stage that this app does not compile.
  `hypercmd/browser.html` is HyperCMD's file browser template, copied unchanged from
  `examples/file-browser/ui/browser.html` at fusor-rs/hypercmd 594eaa7, the example
  in `public/lab/hypercmd-file-browser.png`; copy it again when that example
  changes. `hyperpane/run.sh` is the command that runs any fusor app on iPhone.
- `build/highlight.rs`, `src/code.rs`, and `web/components/code.html`: build-time
  Syntect highlighting rendered as escaped text tokens. No runtime highlighter or
  raw HTML injection. Colors have at least 4.5:1 contrast against the charcoal code
  background.
- `web/app.css`: the Tailwind entry. It defines the color tokens (swapped for dark
  mode and registered with `@property` so worlds can glide), each project's world,
  the spring easing curves, the bench layout and index, the `control` and `field`
  utilities, the segmented-control thumb, the stage frame's shapes and turn, the
  fusing mark, Hughie, and the arrival transition for what comes on stage.
- `public/lab`: images from the lab's projects, and HyperCMD's mark for badges, from
  its `assets/brand/hypercmd.svg`. HyperCMD's file browser (cropped to
  its window), SQL Bomb, spotify-tui and supabase-tui (each cropped to the terminal
  frame's 1.45 proportion) come from each project's repository, and Hughie's faces
  from hughie.si. `hyperpane-iphone.png` is this page captured in the iPhone 17
  simulator through `hyperpane run --ios --dist`; capture it again when the top of
  the page changes.
- `build.rs`: highlighting, standard HTML compilation, installer assets, and exact
  plain-text source copies. Generated copies in `public/` are ignored by Git.

The page is an app rather than a document. The index lists the lab's tools (fusor,
HyperCMD, hyperpane) and the things it makes with them (Hughie, SQL Bomb). Hobby
projects made for fun (spotify-tui, supabase-tui) are on the "Made with fusor" page,
linked from beneath the index and from the header. Choosing one puts it on stage: one frame becomes a
browser running a live fusor component, a terminal, an iPhone showing this page
through hyperpane, or Hughie himself. The whole page takes on that project's world:
warm white for fusor and hyperpane, Hughie's own night and peach light for Hughie,
and the lab's charcoal and vermilion for the terminal projects. A tool's "How it's made" turns the frame over to the exact files behind its
front, so code appears as the lab's proof rather than as a tutorial; a thing links
to where it lives instead. fusor's install command stays on screen whatever is on
stage. On phones the object comes first, with the index as a dial beneath it.

The design uses system fonts, the fusor F emblem and wordmark (painted in the
world's ink), an oxide accent on warm white, and brand charcoal for source code.
[Branding](BRANDING.md) records the artwork and palette.

Motion has one arrival and otherwise answers a choice. On load, the mark's two
shapes arrive from opposite sides and lock together. Choosing a project glides the
page into its world, reshapes the frame on a spring while the new content sharpens
into it, and moves the index's accent dot; "How it's made" turns the frame over in
3D. Hughie breathes, glances up when he arrives on stage or when the page loads in the
strip, and opens his eyes when the pointer comes near. The strip carries Hughie's
world (`data-world` and `data-project`), so it keeps his night on every page and
blends into the page when he is on stage. With reduced motion, nothing moves: worlds, frame and thumb
change at once, the mark and Hughie stay still, and new content only fades in.

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

Each browser run starts its own temporary preview server. Tests check the index
and that each of its five projects takes the stage with its world and frame shape,
the greeting and its Rust state, that each tool turns over to source files identical
to the files on disk and to their published copies with readable syntax contrast,
that things do not turn and land face up, the "Made with fusor" page by link and by
its own address, the install commands, documentation
links, no horizontal overflow from 320 to 1440 pixels, the object above the index on
phones, and keyboard selection. Chromium also checks the clipboard. Screenshots are
saved by browser in the ignored `test-results` directory. Docs and benchmark builds
are not required.
