# Documentation tutorial

This is the compiling companion to the Owners and cleanup, Rust mounting,
Async data loading, and Coherent async views guides. The docs build reads the source files directly into
the relevant code blocks.

From the repository root, after installing the fusor CLI and running setup:

```sh
fusor dev --manifest-path apps/docs/tutorial/Cargo.toml --port 8091
```

Open http://127.0.0.1:8091/ . The `public/data` files are served over HTTP so the
resource example needs no backend or credentials. A missing issue deliberately
returns 404. Try typing local notes, reversing rows, changing selection, hiding
panels, and reloading a request. The guides explain the expected result of each.
Price and stock read separate static files. With network throttling, switching
products shows the previous complete view until both new results are available.

This package declares its own Cargo workspace to verify the public application
setup. It uses local path dependencies because fusor is not yet published.

The directories under `lessons/` contain the exact files used by the guides.
They are not modules of this companion app. For the application lessons,
`just test docs-examples` applies them to freshly generated applications, builds
the results, and verifies the observable behavior in a browser.

The worker lessons in `lessons/workers/` are included directly by the independent
consumer in `tests/fixtures/worker`. `just test worker` exercises tasks, services,
progress, cancellation, Fetch, and streams; `just test worker-pool` also exercises
the documented compute and shared-data examples. Keeping the pool lessons in
that consumer avoids making the tutorial app require a threaded build.

The Reusable HTML lesson uses the generated counter files directly. Nested content
adds the files under `lessons/content` to that app. The test copies the published
blocks from the Markdown guides in `../public/content/`, including inline module
declarations, and checks both shared updates and independent local state. These lessons use `template!`
and typed component tags. The companion also uses explicit mounts for ownership,
resource, and coherent-view examples. Its list uses `<ForEach>` with a reusable
`Row` component whose item input is `Memo<Item>`. The `lessons/foreach` guide
repeats inline HTML directly, with no row struct.

The example test also copies the async Reader into a fresh generated application
using the guide’s adaptation checklist and inserts the documented batch Reset
button into the greeting lesson.

`lessons/manual-inputs/counter.rs` is the complete handwritten alternative to
`#[derive(FromInputs)]`. The tests run both versions with the same HTML and
observable shared/local behavior.

`lessons/mounting` shows the optional low-level integration path: prepare a
compiled Counter from Rust, insert and retain it under an existing host, then
release the retained Scope. Its Wasm start function deliberately replaces managed
`<App>` startup. The test verifies insertion, reactive updates, and removal
while preserving the existing host element.

`lessons/app` demonstrates the built-in App boundary with an inferred Dashboard
state returned by an ordinary free function. Its native main element remains
the root; the App tag emits no wrapper.
