# Build for the web. Write Rust.

fusor lets you build interactive web pages with HTML and ordinary Rust. Start with a working
app, then learn how state, templates, and browser behavior connect.

## What runs in the browser? {#the-idea}

You write `.html` files and `.rs` modules. The build compiles Rust to WebAssembly (Wasm),
which runs in the browser, and generates the connections between Rust values and HTML
elements.

Clicking the button below changes a signal: a value the page watches. Text that reads that
signal updates automatically. Your Rust runs as compiled Wasm, not as a script the browser
interprets.

```html title=HTML · inside a component
<button on:click="state.count.update(|n| *n += 1)">
  Clicked {{ state.count.get() }} times
</button>
```

> This excerpt assumes a count: `Signal<i32>` field. HTML and Rust shows the complete Rust
> type, HTML file, and setup.

- [See the complete matched example](/docs/html-and-rust)

## What you build {#what-you-get}

The default app runs in the browser. Rust owns interactive state, and generated bindings
update the page’s DOM. Optional routing changes pages without reloading the document. Use
browser HTTP requests to call your own backend.

Later, you can generate HTML ahead of time and start interactive parts on demand using
islands. You do not need that delivery setup for your first app.

> This documentation website is itself a fusor application. Its routing, search, theme
> controls, and live counter run in Rust compiled to Wasm.

## Start with the generated app {#choose-a-start}

Begin with Installation. It installs the CLI, creates a Cargo app, and tells you what to
expect in the browser. Project structure then explains the files the CLI generated. You need
HTML and a terminal, not prior Rust experience: the guides introduce each Rust piece —
structs, impl blocks, signals, closures — the first time an example uses it.

- [Install and run your first app](/docs/installation)
- [Understand the generated files](/docs/project-structure)

## Follow one working path {#learning-path}

Start with HTML and Rust, then Signals and derived state, to make a greeting and counter
interactive. Reusable HTML explains how to connect the Rust and HTML files of a child
component.

Next, learn Conditions and pattern matching to choose what renders, Lists with ForEach to
repeat rows, and Nested content to place caller-supplied HTML inside a panel.

Component identity introduces the tutorial companion at `apps/docs/tutorial`: a second,
ready-made app in the repository. Owners and cleanup, Async data loading, and Coherent async
views use it to demonstrate complete behavior without requiring you to assemble each
example. Routing and islands are optional next steps.

- [Build reusable HTML](/docs/components)
- [Load and display data](/docs/async-data)
- [Conditions and pattern matching](/docs/control-flow)
