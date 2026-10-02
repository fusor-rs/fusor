# fusor-router

Typed routes, URLs and owned route views for fusor.

Routes can be an ordinary Rust enum implementing `Route`, or URL patterns used
by HTML `Router`/`Route` tags. Parsing and formatting are plain functions, and
every URL is relative to the application's base path. Enable `browser` for
outlets and navigation through browser history. Add it to an application with
`fusor add router`.

External renderers use `view` without enabling `browser`. Its versioned
`RouteScope` contract extends `fusor::render::Scope` with renderer-specific
attachment, fallible preparation and activation. `RouteView` factories return
prepared children of the supplied owner; `ViewRouter::mount` returns a token
that the containing scope retains. Dropping the final token, or disposing its
owner, disposes the outlet even if navigation handles survive.

The shared engine selects patterns, decodes parameters, retains unchanged route
identities, updates nested outlets and stages replacements before publishing a
location. Query and fragment changes retain the view. Changed parameters or a
different fallback path replace it. The browser adapter uses this same engine.

A platform obtains `Navigation` from the containing owner or mounted router.
For direct navigation, call `navigate(AppUrl)`. For a history transaction, call
`prepare_navigation`, publish the platform's history only after preparation
succeeds, then `commit` synchronously. Dropping the stage rolls back candidate
views; constructor side effects that already ran are not undone. Only one stage
may exist per tree. Preparation may attach inactive nodes beside the current
view, so finish or abandon the stage before yielding to a presentation loop.

Fusor owns route identity and lifecycle behavior. The adapter owns URL input,
history, focus/scroll policy and concrete nodes. The portable contract does not
provide browser link interception or a terminal history stack. Executable native
contract tests are in [`tests/views.rs`](tests/views.rs); the external backend
fixture also compiles HTML routes against this API.

Part of [fusor](https://github.com/fusor-rs/fusor), which builds reactive applications from HTML and ordinary Rust. Licensed under the [MIT License](LICENSE).
