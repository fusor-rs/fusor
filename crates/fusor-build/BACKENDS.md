# Independently maintained compiler backends

`fusor_build::backend` is the supported, versioned in-process extension surface.
It compiles fusor HTML into ordinary Rust using a backend supplied by a build
helper. It does not load browser delivery metadata, register an external crate
name, or infer a renderer from `target_arch`.

`compile_app()` and plain `template!` select the internal `DomBackend`. It and
the external adapter implement the private `CompilerBackend`, sharing semantic
lowering and closure/factory generation. `DomBackend` does not implement the
public `Backend` trait; it owns DOM mounting, binding bundles, hydration, coherent
rendering, routing, JavaScript setup and browser/server component emission.
Document delivery, asset discovery and web packaging stay in the build pipeline;
there is no application-facing backend selector.

The DOM compiler implementation lives in `src/bindings/codegen/dom/`: `mod.rs`
connects the backend contract, `component.rs` handles mounting and delivery
constants, `bindings.rs` installs bindings, `coherent.rs` emits coherent frames,
and `template.rs` prepares descriptors and binding bundles. Shared semantic
lowering stays in `codegen.rs`.

The executable contract example is
[`tests/fixtures/external-backend`](../../tests/fixtures/external-backend).
It is a separate Cargo workspace containing a build helper, small in-memory
renderer, component-owning library and consuming application. It is test
infrastructure, not a terminal renderer or a proposed production runtime.

## Build and inclusion

An external helper validates its own configuration and supplies explicit
package-relative HTML paths:

```rust,ignore
use fusor_build::backend::{build, generate};
let inputs = build::BuildInputs::from_cargo(["ui/panel.html"])?;
let manifest = build::compile_cargo(&inputs, "memory", |_, html| {
    generate(html, &MemoryBackend)
})?;
```

The component type's owning Rust module includes the result:

```rust,ignore
fusor::template!(backend = "memory", "ui/panel.html");
```

The original `fusor::template!("ui/panel.html")` and `compile_app()` retain their
browser behavior. A component library can include both implementations, selected
by explicit Cargo features. The compiler does not decide that Wasm means browser.
Each library compiles the templates it owns; applications depend on that library's
compiled implementations. This satisfies Rust's orphan rules and avoids reaching
into another package's sources. Publish the HTML/build inputs with the library.

`BuildInputs` checks canonical package boundaries, duplicate files, relative
paths and source identity. `compile_cargo` only writes below Cargo's `OUT_DIR`,
rejects output symlinks, and lowers all sources before starting output writes.
It watches each HTML file. A helper must also watch its configuration and any
source-discovery directories, stylesheet or other asset dependencies. The API
does not discover or deliver external stylesheet assets for a renderer.

The output contract (`build::OUTPUT_VERSION = 1`) is:

```text
OUT_DIR/fusor_backends/<namespace>/<package-relative-html>.rs
OUT_DIR/fusor_backends/<namespace>/<package-relative-html>.map
OUT_DIR/fusor_backends/<namespace>/manifest.json
```

Namespaces contain only ASCII letters, digits, `_` and `-`; independently called
helpers must choose different names. Browser output remains under
`OUT_DIR/fusor_templates/`. The manifest describes canonical source, generated
Rust and map paths; `OutputManifest::read` rejects an incompatible version.
The existing browser artifact and HTML marker formats are unchanged.

Maps use `SourceMap` / `fusor-source-map-v1`: ordered generated **line ranges** to
one-based authored HTML locations. They are not token-perfect maps. A diagnostic
adapter matches Cargo JSON spans to `OutputSource::rust`, calls
`SourceMap::lookup(span.line_start)`, and displays `OutputSource::source` plus the
returned line/column. The external fixture actually compiles an invalid binding
and performs this translation; keeping token spans alone would not suffice.

## Compiler responsibilities and backend callbacks

Implement `Backend` with `version()` returning the literal contract version it
was written against (currently **1**, equal to `backend::VERSION`).
An incompatible version fails before emission. Its callbacks provide:

- `runtime`: explicit paths for the external scope, error, children factory and
  construction-error conversion function.
- `supports` and `validate_binding`: capability decisions with the operation's
  anchor, static template and authored origin. The latter supports restrictions
  such as button-only click handlers. Rejections happen before any Rust emission.
- `validate`: checks every static node and attribute, including unbound elements.
- `mount`: emits a fallible scope-creation expression for complete static structure.
- `operation`: emits target-specific statements using supplied closures/factories.
- `component`: emits the renderer's mounting implementation or application entry.

All Rust is `proc_macro2::TokenStream`, normally built with `quote!`. Backend
emitters must preserve supplied token streams instead of stringifying and
reparsing expressions. Fusor performs its canonical parsing, lexical rewriting,
nested `state` aliases, input struct inference, branch selection and payload
projection, row projection, capture cloning and recursive factory generation.
No downstream parser, capture visitor or copy of the private compiler IR is
required. Rust types remain checked by rustc in the consuming application.

A `Template` contains nodes in preorder with parent indices. Element attributes
and text are HTML-decoded; static comments remain available. Typed anchor kinds
separate element, text and child-region IDs. IDs are sparse and only meaningful
within a mounted template instance; never use them as global runtime identity.
Compiler delivery wrappers/marker comments are translated into this structure,
so a backend does not parse fusor's browser marker protocol. Each static node and
attribute has its authored origin. This is explicit authored nesting, not a
browser HTML tree-builder's implicit element insertion; a renderer must reject
markup it cannot interpret. CSS interpretation belongs to the backend.

The v1 operation set is text, attributes/properties/boolean values/classes,
events, existing binding control kinds, branches, keyed lists, component tags
and `Children`. A backend accepts only the subset it implements. Routing,
hydration, JavaScript, coherent HTML `Async`/`Await`, `rust:render`, opaque slots
and named projected content fail explicitly in v1. Templates use ordinary Rust
modules; inline and external script linkage is rejected by this entry point.
These limitations do not change browser compilation of those features.

Each source must declare a component or an `App`. All static markup and nonblank
text must belong to those trees; a document shell, doctype, stylesheet/link or
static sibling outside them is rejected at its authored location. External
helpers own document configuration and asset discovery. `App` itself requires
the backend's explicit `Capability::App` support; a backend that only emits
reusable component implementations can reject application startup cleanly.

## Generated runtime contract

This interface does not add a universal runtime `Renderer` trait. The backend
owns its mounting trait, scope and node types, independently of `dom::Component`.
`ComponentCode::body` expects locals `parent: Option<&OwnerHandle>` and `make`, a
one-shot constructor taking `OwnerHandle` and returning `Result<State, Error>`.
The backend component hook wraps the body in its own implementation/entry point.

`mount` creates a prepared scope. That scope supplies `owner()` and
`retain_state(T) -> Rc<T>`; retaining state must tie its lifetime to scope
cleanup. `Children` supplies `Default`, `Clone`, `take()` and
`new(Fn(&OwnerHandle) -> Result<Scope, Error>)`. `take()` transfers the incoming
children factory from the renderer's mounting context. Context installation must
restore its predecessor, including on panic; the fixture uses `Children::with`.

Each operation receives ready-to-use closures. Branches read `(case_index, T)`
and prepare from `(usize, Signal<T>, &OwnerHandle)`. Lists read `Vec<T>`, key
`&T -> K`, and prepare from `(Signal<T>, &OwnerHandle)`. These payloads include
fusor's canonical lexical and row projections. The renderer retains keyed source
signals/scopes, validates duplicate keys and updates retained sources. It must
not reconstruct branch payload types or capture environments. Component
operations receive a typed constructor plus conditional identity and incoming
children. The backend's conversion function maps each `FromInputs::Error` to its
own error. Browser conversion remains `dom::IntoMountError`.

Invoke preparation and input-construction factories under `fusor::untrack`.
Constructor reads must not subscribe the surrounding structural effect; effects
created inside a factory still track their own reads and run immediately.

Ordinary constructors run immediately, as they do in browser mounting. Their
ordinary effects can execute before publication and can have run before a later
mount failure. Owner activation is separate: validate and publish the scope
before committing it. Scope disposal must stop retained bindings and children.
Fusor does not silently defer constructor effects or promise rollback of arbitrary
application side effects. For staged async construction and coherent publication,
see [the supported core integration contracts](../fusor-core/BACKENDS.md).

Generated binding installation preserves existing order: text/checkbox bindings
precede authored listeners, while select bindings install after other bindings.
A renderer dispatches one reactive batch per listener and releases mutable scene
borrows before calling application code. Event payload types remain owned by the
renderer; `on:click` handlers that ignore `event` need no shared event type.
Use `TextValue`/`Checkbox` rather than introducing another parsing policy. Editor
drafts, cursor/focus, propagation and unsupported control behavior remain explicit
renderer responsibilities.

The external compiler protocol is version 1, independently of the unchanged
browser template format (version 3), coherence integration (version 1) and
`fusor_components` row/capture integration (version 1). Pin compatible fusor
compiler/runtime releases; fixture emitters should assert the contracts they use.
