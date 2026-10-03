# Signals and derived state

A signal is a shared handle to a changing value. HTML bindings subscribe when they read it,
then update when it changes.

## Get, set, and update a value {#signals}

`get()` returns a cloned value and tracks a dependency when called inside a reactive
computation. `set()` replaces the value. `update()` lends the value mutably to a closure.
Read inside the HTML binding so it subscribes to future changes; reading once into a plain
field is only a snapshot.

```rust title=Rust · API demonstration
let count = fusor::signal(0_i32);
count.set(2);
count.update(|value| *value += 1);
assert_eq!(count.get(), 3);
```

> If you come from React or Vue: a signal update refreshes the bindings and computations
> that read that signal. Plain Rust fields do not notify the page. An existing component
> instance keeps its state and does not run its constructor again for those updates; a
> replacement or later remount creates a fresh instance.

- [Run the counter in HTML](/docs/html-and-rust)
- [Read the closure syntax in handlers](/docs/html-and-rust#closures)

## Clone the handle when sharing state {#handles}

`Signal::clone` shares the original signal. Calling `signal(...)` creates a new signal. This
is why the generated app’s counters share `count` while each has its own `clicks`.

A closure written `move || …` takes ownership of captured values so it can outlive the
function that created it. Clone a signal before the closure when you also need to retain the
original handle: the closure keeps one handle, the caller keeps the other, and both access
the same value.

```rust title=Rust · API demonstration
let count = fusor::signal(0_i32);
let same_count = count.clone();
same_count.set(7);
assert_eq!(count.get(), 7);
```

## Compute from other state {#derived}

A `Derived` computes a value from source signals when read. Store it in a field to name a
reusable calculation, then read it in HTML like a signal.

In the constructor below, the block containing `let count = count.clone();` first clones the
handle. The following `move || count.get() * 2` closure keeps that clone. The original
`count` remains available for the `Counter` being constructed.

A `Memo` also caches its result and can avoid notifying readers when that result compares
equal. Keep both kinds of calculation free of application side effects.

```rust title=Rust · alternative Counter state, independent example
use fusor::{Derived, Signal, derived, signal};

struct Counter {
    count: Signal<i32>,
    doubled: Derived<i32>,
}
impl Counter {
    fn new() -> Self {
        let count = signal(2);
        let doubled = derived({
            let count = count.clone();
            move || count.get() * 2
        });
        Self { count, doubled }
    }
}
```

> This is an independent Rust state example, not a replacement for the `Counter` in Reusable
> HTML: it has no `FromInputs` implementation. A matching HTML template can read
> `{{ state.doubled.get() }}` to display 4, then 6 after count increases. The live example
> linked below shows derived values in a complete page.

- [Try the reactive workspace](/docs/showcase/reactive)

## Publish related synchronous writes together {#batch}

To try batch in the greeting app from HTML and Rust, add this Reset button inside `<App>`’s
main element in `web/index.html`. Change the name and count, then click Reset: the name
returns to Ada and the count to 0. batch groups the writes so reactive subscribers run after
both. It is synchronous and does not wait for HTTP requests.

```html title=web/index.html · add inside App’s main element
<button on:click='fusor::batch(|| {
    state.name.set("Ada".into());
    state.count.set(0);
})'>Reset</button>
```

- [Coordinate asynchronous display updates](/docs/coherent-async)
