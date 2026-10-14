# fusor-macros

Derive macros for fusor components.

`#[derive(FromInputs)]` turns a component's `#[input]` and `#[local]` fields
into its typed constructor inputs; `#[input(default)]` makes an input optional.
`#[derive(Inputs)]` lets component tags build a hand-written inputs struct.
`#[derive(JsInputs)]` exposes the values a component passes to its JavaScript
module.

Use them through `fusor`; no direct macro-crate dependency is needed.
`FromInputs` and `Inputs` are re-exported with the `derive` feature, which also comes with
`dom`. Its constructor returns `Result<Self, Infallible>` and can compile
without browser dependencies. `JsInputs` requires `javascript`.

Part of [fusor](https://github.com/fusor-rs/fusor), which builds reactive web applications from HTML and ordinary Rust. Licensed under the [MIT License](LICENSE).
