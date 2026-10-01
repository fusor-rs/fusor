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
cleanup, dependency isolation, shared browser compilation and authored diagnostics.

This is not a reusable renderer. Its compiler rejects unsupported features;
it provides no layout, stylesheets, focus/cursor implementation, browser bubbling
or event payloads, application startup, select/radio binding, dynamic attributes
or properties, routing, hydration, JavaScript, async/coherent publication or server
output. Preparation failures and duplicate keys abort the assertion run rather
than providing application error recovery or transactional publication. See the
[compiler](../../../crates/fusor-build/BACKENDS.md) and
[runtime](../../../crates/fusor-core/BACKENDS.md) contracts for lifecycle rules.
