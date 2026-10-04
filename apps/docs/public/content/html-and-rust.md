# HTML and Rust

Connect one Rust state type to one HTML page. This complete example makes a greeting and
counter interactive.

## Define all the state in Rust {#modules}

Start with the app from Installation. Replace `src/app.rs` with this complete file. Save it
and the HTML in the next step before checking the result; the old HTML and new state will
not match halfway through the edit.

The `name` field is a `Signal<String>` for the text input. The `count` field is a
`Signal<i32>` for the counter. Each signal stores a changing value that HTML can watch.

The expression `signal("Ada".into())` creates the name signal. Rust distinguishes a borrowed
string slice, such as `"Ada"` (type `&str`), from an owned, growable `String`. Here
`.into()` converts the literal to the `String` the signal needs.

Finally, `template!("web/index.html")` connects this Rust module to the HTML in the next
step.

```rust source=tutorial/lessons/basics/app.rs title=src/app.rs · replace this file
```

- [Look up every template attribute](/docs/html-and-rust/attributes)

## Bind that state to actual elements {#bindings}

Replace `web/index.html` with this complete file. Keep the existing generated manifest,
`build.rs`, `lib.rs`, and `Counter` files; their modules can remain declared even though
this page does not mount a counter component. dev will rebuild.

The initial page says “Hello, Ada” and “Count: 0”. Typing changes the greeting, Increase
changes the count, and Use Grace changes both the greeting and input.

```html source=tutorial/lessons/basics/index.html title=web/index.html · replace this file
```

- [See Rust and HTML working together](/docs/showcase/reactive)
- [Events: browser handlers and custom messages](/docs/events)

## Read and write at the element {#binding-forms}

Use `{{ expression }}` to display escaped text or update an ordinary HTML attribute. A
binding subscribes to the signals it reads, so `{{ state.count.get() }}` updates when the
count changes.

Use `on:click` to run Rust when a click occurs. Use `bind` to synchronize a form control
with a signal in both directions; pass `state.name`, the signal handle, without `.get()`.

Boolean attributes follow browser semantics: `disabled="{{ false }}"` removes the attribute
rather than writing the text “false”.

> Reading a signal with `get()` inside a text or attribute binding subscribes that binding.
> A plain Rust field is ordinary state; changing it alone does not notify the DOM.

## Choose a Rust type for each form control {#form-state}

Give each control a signal of the type it should edit. Here `seats` is a `Signal<u32>` for a
number input, `volume` a `Signal<f64>` for a range, and `toppings` a `Signal<Vec<String>>`
that collects the checked values. The `bind` reference lists every control and the types it
accepts.

```rust source=tutorial/lessons/forms/app.rs title=src/app.rs · a form's state
```

- [bind reference](/docs/html-and-rust/attributes#bindings)

## Bind each form control {#form-controls}

Every control takes the signal handle, such as `bind="state.seats"`, and each `<output>`
shows the value its control edits. Typing half a number leaves `seats` unchanged, and the
input keeps what you typed. Radios bound to `state.size` act as one group.

```html source=tutorial/lessons/forms/index.html title=web/index.html
```

## Read the closure inside update {#closures}

The expression `state.count.update(|n| *n += 1)` adds one to the stored count. The part
`|n| *n += 1` is a closure: an anonymous function, similar to a JavaScript arrow function.
Rust writes its parameters between vertical bars.

`update` gives the closure temporary mutable access to the number through `n`. Because `n`
is a reference, `*n` accesses the number itself; `*n += 1` changes it in place.

You can also use `set` to replace the whole value. The two lines below are alternatives:
either one adds one. Running both adds two.

```rust title=Rust · two equivalent ways to add one
state.count.update(|n| *n += 1);
state.count.set(state.count.get() + 1);
```

> A closure that keeps running after the surrounding function returns — an event handler,
> for example — is usually written `move || …`, which makes the closure take ownership of
> the values it captures. Signals and derived state shows why a handle is cloned before such
> a closure.

- [Clone a handle before a move closure](/docs/reactivity#handles)

## Know which names are in scope {#scope}

Some HTML expressions receive local variables from the framework. You do not declare or
import these variables in the template.

`state` refers to the current component’s Rust value. For example, `state.count` accesses
its `count` field.

`owner` is an `OwnerHandle`: a handle that ties requests and cleanup to a mounted view’s
lifetime. The framework supplies it inside the `<App>` state expression. Child components
receive their own handle through `FromInputs::from_inputs`. It is not a global variable.

This framework lifetime is separate from Rust’s language-level ownership and borrowing
rules. The reference below lists the other local names and where each is available.

```text title=Binding scope reference
state  — current component value, in its template bindings
owner  — OwnerHandle in the App state expression
event  — web_sys::Event in an on:* handler
item   — Memo<T> inside ForEach child HTML
index  — Memo<usize> inside ForEach child HTML (zero-based)
chosen name — Rc<T> inside <Await value="{{ read }}" let="chosen_name">
```

> `OwnerHandle` is the public Rust type; `owner` is the variable holding a value of that
> type. Importing `OwnerHandle` makes the type name available but does not create an `owner`
> variable.

- [Where item and index come from](/docs/components/for-each#scope)
- [See where ready comes from](/docs/coherent-async#await)
- [Constructor arguments and framework-supplied values](/docs/ownership/mounting#supplied)
- [What is owner, and where does it come from?](/docs/ownership/mounting#owner-variable)

## Keep HTML attribute quoting valid {#quotes}

When a Rust expression contains string literals, use single quotes around the HTML attribute
and double quotes inside the Rust. With a double-quoted HTML attribute, escape the inner
quotes as `&quot;`.

Wrapping statements in `{ }` groups Rust code; it does not protect string quotes from the
HTML parser.

```html title=HTML · equivalent quoting styles
<button on:click='state.name.set("Grace".into())'>Use Grace</button>
<button on:click="state.name.set(&quot;Ada&quot;.into())">Use Ada</button>
```

## Let the framework start your App {#startup}

The built-in `<App>` tag evaluates its `state` expression at startup. Here `App::new()`
returns the value exposed as `state` throughout the child HTML. The framework retains that
value and its bindings, so you do not write a Wasm startup function.

If construction needs an owner, custom arguments, or error handling, follow The App boundary
guide.

> A fallible constructor can use `state="{{ create_dashboard(owner)? }}"`. Here `owner` is
> supplied by `<App>`, and `?` propagates an initialization error. A return type such as
> `Result<Dashboard, JsValue>` means success returns `Dashboard`, while failure returns a
> JavaScript value represented by `JsValue`.

- [App boundary: state, owner, lifecycle, and customization](/docs/html-and-rust/app)
