# fusor-macros

Derive macros for fusor components.

`#[derive(FromInputs)]` turns a component's `#[input]` and `#[local]` fields
into its typed constructor inputs. `#[derive(JsInputs)]` does the same for the
values a component passes to its JavaScript module.

Use them through `fusor`; no direct macro-crate dependency is needed.
`FromInputs` is re-exported with the `derive` feature, which also comes with
`dom`. Its constructor returns `Result<Self, Infallible>` and can compile
without browser dependencies. `JsInputs` requires `javascript`.

Part of [fusor](https://github.com/fusor-rs/fusor), which builds reactive web applications from HTML and ordinary Rust. Licensed under the [MIT License](LICENSE).
