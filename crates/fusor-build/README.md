# fusor-build

The build-script compiler for fusor applications.

It reads an application's HTML templates, validates them, and turns their
bindings and `<script type="text/rust">` blocks into ordinary Rust that rustc
type-checks. Compiler errors point back at the HTML line they came from.

Applications call it from `build.rs`; `fusor new` sets this up for you:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    fusor_build::compile_app()
}
```

Part of [fusor](https://github.com/fusor-rs/fusor), which builds reactive web applications from HTML and ordinary Rust. Licensed under the [MIT License](LICENSE).
