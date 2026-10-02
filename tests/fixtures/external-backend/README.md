# External backend contract fixture

This separate Cargo workspace tests supported public APIs as an independent
consumer. It contains `memory-compiler`, a `fusor_build::backend::Backend` build
helper; `memory-renderer`, a small in-memory renderer; `fixture-components`, which
owns private component state and `ui/panel.html`; and an application consuming
that library without reading its HTML or generating foreign trait implementations.
The `browser` feature compiles the same HTML with `compile_app()`.

Run `just check` for the compiled native contract, or `just test external-backend`
for the native assertions, dependency isolation, Wasm compilation with and
without browser features, and real rustc diagnostics translated through the
versioned output manifest and line-range source maps.

The assertions in [`src/main.rs`](src/main.rs), the renderer tests and the
[tooling suite](../../tooling/external-backend.mjs) cover static/reactive structure,
typed construction, events and controls, structural factories, keyed identity,
cleanup, nested routing with decoded parameters and retained parent views,
failed navigation rollback, dependency isolation, shared browser compilation and
authored diagnostics. Routing uses the shared `fusor_router::view` runtime;
the renderer only attaches, publishes and removes its in-memory nodes.

Compiled `Async`/`Await` also exercise nested aliases, supplied children, keyed
row projections through cached memos, deferred candidate effects, pending scene
retention, handler gating, failed preparation, request cancellation and retry.
The fixture uses core's boundary protocol and `fusor_test` controlled requests;
its frame only stages in-memory text, listeners and child placement.

This is not a reusable renderer. Its compiler rejects unsupported features;
it provides no layout, stylesheets, focus/cursor implementation, browser bubbling
or event payloads, application startup, select/radio binding, dynamic attributes
or properties, platform history, hydration, JavaScript or server output. Coherent
regions reject editable controls, nested Async boundaries and router outlets.
Initial component preparation errors propagate to the caller;
later reactive component errors and duplicate keys abort the assertion run. See the
[compiler](../../../crates/fusor-build/BACKENDS.md) and
[runtime](../../../crates/fusor-core/BACKENDS.md) contracts for lifecycle rules.
