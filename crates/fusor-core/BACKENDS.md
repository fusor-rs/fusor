# Runtime contracts for external renderers

An independently maintained renderer can use `fusor-core` with
`default-features = false`. Signals, memos, effects, batching, owners, context,
binding traits and coherent-read integration have no browser dependencies.
Select browser support with features and generated output, not by assuming
that every `wasm32` program runs in a browser.

## Supported integration surface

| API | Responsibility |
| --- | --- |
| `Signal`, `Memo`, `effect`, `batch`, `untrack`, cleanup effects | Reactive dependencies and scheduling. Retain subscriptions for the lifetime of the rendered view. |
| `Owner`, `OwnerHandle`, `Registration`, `ContextKey` | Weak lifetime checks, child ownership, activation callbacks, context and cleanup. Retain registrations until they run or should be removed. |
| `render::Children<Scope, Error>` | Captured child factories and nested, unwind-safe incoming-child delivery. DOM and external renderers supply their own placement and mounting. |
| `render::{Scope, construct}` | The generated construction handoff: obtain an owner, select ordinary or explicitly prepared effects, and retain state. Both DOM and external generated components use it. |
| `bind::{TextValue, Checkbox, selected}` | Model-side text conversion, equality, touch and checkbox membership. The renderer owns the editor draft, selection and cursor. |
| `coherence::{AsyncBoundary, Attempt, Publication, ReadLease, BoundaryMount, BoundaryLifetime, prepare_state}` | Candidate read discovery, retry generations, cancellation, validation and publication coordination. |
| `versions::Versions`, `Signal::with_render_value` | Optimistic source validation and synchronous candidate reads through signals and memos. |
| `fusor_components::{Entry, Value, RowValue, Row, ItemRow, Captured}` and `ForEach::{entries, values, key, value_key, row, item_row}` | The compiler's typed row and lexical-capture environments. The renderer implements key validation, placement, identity retention and removal. |

`fusor::render::VERSION` is **1** and covers shared renderer integration.
`fusor::coherence::VERSION` is **2** and covers the coherence and source-version
integration semantics above. `fusor_components::BACKEND_VERSION` is **1** and
covers the listed row/capture helpers. An external integration can assert these
constants at compile time. These contracts are separate from the browser
`fusor::template::VERSION` and the compiler backend facade's version. APIs remain
experimental with fusor's release series; a changed contract increments its
version rather than silently changing generated-code assumptions.

This support does not expose DOM `Scope` internals, the DOM commit queue,
mount-readiness flags, the private keyed movement planner, or a generic renderer
trait. Existing owner primitives suffice for the synchronous external consumer.
The renderer defines its own mounted-component contract; construction uses the
portable `fusor::FromInputs` trait.

## Construction, publication and cleanup

Creating an `Owner` prepares a lifetime. `commit()` activates registered work
once the owner and its ancestors have committed. It does not validate nodes,
publish a scene, or roll back side effects. Dropping an owner disposes it; the
entire descendant tree is invalidated before any cleanup callback runs.
`OwnerHandle::guarded` suppresses calls before activation and after disposal,
and invokes application code without reactive tracking. It does not batch
writes; a listener adapter adds `batch` around each callback.

Ordinary `effect(...)` runs its first callback immediately, including inside a
batch and when a component's owner is still prepared. Ordinary browser
constructors retain that timing. A synchronous backend must preserve it when
claiming the same constructor behavior. A failed mount can therefore have
already executed ordinary constructor side effects. Retain effect handles in
component state or a scope and release them during cleanup.

`coherence::prepare_state(owner, make)` explicitly changes candidate constructor
timing: effects created while that owner is inactive wait for activation and
stop on its disposal, even if an effect handle survives. A discarded candidate
never starts them. Nested preparation restores its enclosing owner, including
on unwind. Supplying an already active owner preserves immediate effects and
does not attach those effects to its cleanup. The helper does not make arbitrary
application side effects transactional.

Coherent rendering adds separate phases:

1. `AsyncBoundary::attach` retains a renderer evaluator. Evaluation can start
   before owner activation, collect dependencies and prepare a candidate scene.
2. `Attempt` records pending reads and their leases. Existing
   `fusor_async::AsyncValue::read` can intentionally start speculative reads
   before commit. The boundary cancels obsolete leases; the read adapter also
   guards request generations so late results cannot publish.
3. If every read is ready, core checks captured source versions, invokes
   `Publication::validate`, then checks versions again. Validation errors leave
   the previous scene in place. Dropping rejected publication objects releases
   their staged patches. Structural candidates may stay alive across pending
   passes and retries in one input epoch. Register weak slot cleanup with
   `Attempt::on_invalidate` to release them on an input change or boundary disposal,
   including slots that are no longer visited.
4. `Publication::apply` mutates the validated scene synchronously without
   application callbacks, formatting, key comparison or signal writes. An
   unexpected failure is `Faulted`; core cannot undo a renderer's partial scene
   mutation. This phase should have no remaining ordinary failure cases.
5. `Publication::finish` activates adopted owners and retires old scopes in the
   renderer's specified order. Application callbacks may run here. Core batches
   apply/finish and checks versions afterward so activation cannot overwrite a
   newer pending state with `Ready`.

The renderer still owns staged nodes, atomic scene mutation, focus and
interaction while pending, and rollback before publication. Supporting resource
state does not imply support for HTML `<Async>`/`<Await>`; reject those tags until
the renderer supplies and tests the full publication behavior.

`Signal::with_render_value` temporarily overrides tracked and untracked reads
without notifying subscribers. While an override is active, `Memo` reads
evaluate their pure computations against candidate inputs, including nested
memos and `ForEach::row`/`item_row` projections. They do not change committed
caches, equality results or subscriptions. The consuming evaluation tracks
candidate dependencies directly, validating overridden signals against the
supplied collection versions. Ordinary reads retain memo caching and equality
suppression. Candidate values are temporary: repeated candidate reads may
recompute, and no candidate cache is carried into publication.

Nested overrides and panics restore the previous read context. Source-version
validation still refreshes committed memos using committed signal values.
Coherence version 2 adds these memo semantics; version 1 only supported direct
signal overrides. Do not mutate signals inside an override.

## Event and control parity

The browser implementation and the component-tags consumer's event traces
establish these ordering rules:

- `Scope::on` and generated event handlers batch **one listener at a time**.
  Effects may run between listeners for the same event. DOM propagation is not
  one reactive batch. Raw `dom::Listener::new` is unbatched and has no owner gate.
- Native attributes are lowered in name order, so `bind` installs before an
  authored `on:input`, `on:change` or `on:blur` on the same text/checkbox/radio
  element, regardless of authored attribute order.
- The compiler moves single/multiple select bindings after other bindings so
  option values exist first. An authored select `on:change` therefore runs
  before the select's binding listener and can observe the previous bound
  value. Do not promise that every authored change handler sees the new value.
- Text controls call `TextValue::edit` on input and call `touch` on blur.
  Checkbox/radio binding uses change; single-select binding uses change and
  touch on blur. Multiple-select and boolean/vector-checkbox bindings do not
  add a text touch callback. Browser composition has its own draft guard.
- Programmatic signal changes do not synthesize user events. Release registry
  and scene borrows before application callbacks. A callback may remove its own
  element; subsequent delivery must honor expired owners and node generations.

The executable traces assert `text:edited|input:edited|pulse:2` for text input and
`change:a|pulse:4|select:b` for select change. They prove the listener boundary,
including two writes in each authored handler producing one effect delivery. The
external fixture's narrower declared event profile must reject unsupported
event/control pairs rather than approximate browser bubbling or browser event
payloads.

A text adapter keeps visible text independent of its parsed model. On user edits
it stores the draft and calls `TextValue::edit`. An invalid numeric draft leaves
a `Signal<T>` unchanged, so do not immediately overwrite that draft with
`text()`. A subsequent model update checks `shows(draft)` before replacing it;
equivalent text such as `012` for integer `12` stays intact. `TextField<T>`
already retains invalid drafts and validation state. Use existing forms/actions
APIs and optional async/query crates with their supplied local spawner, loader
and clock interfaces. `fusor-test` supplies deterministic requests, executors,
clock and lifetime probes; no additional reactive or parsing implementation is
needed.

## Evidence and limits

Core's native public-API tests in `tests/rendering.rs` cover ordinary versus
prepared effects, failure and unwind restoration, callback self-removal,
renderer validation versus owner activation, publication
faults, speculative-read cancellation and stale wakeups, source versions and
candidate direct reads. `tests/bind.rs` covers invalid/equivalent drafts and
checkbox parsing/membership. Existing component tests cover row projection
updates, memo equality suppression and lifetime retention.
Per-listener batching is exercised by the external fixture renderer and the
browser component-tags traces.

These tests establish runtime seams, not a production renderer. Terminal event
normalization, focus, editing, layout, painting, OS integration, executors and
Wasm hosting remain external work. The in-memory fixture also executes compiled
coherent HTML, cached speculative row projections, attempt cleanup and routing;
it supplies no terminal interaction or rendering policy.
