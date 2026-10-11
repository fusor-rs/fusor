# Context

Context lets an outer component share a value with any component inside it, at any depth,
without passing it through the components in between. Typical uses are the signed-in user,
a theme, a language or an API client.

## The problem it solves {#problem}

Say `App` knows who is signed in, and a `UserBadge` deep inside the page shows the name:

```text title=Text · where the value lives and where it is needed
App            knows the user: "Ada"
└─ Sidebar     doesn't care about the user
   └─ UserBadge   shows "Signed in as Ada"
```

Without context, the user travels as an input through every layer, so `Sidebar` must accept
and forward a value it never uses:

```html title=HTML · without context
<Sidebar user="{{ state.user.clone() }}"></Sidebar>

<template rust:component="Sidebar">
  <aside>
    <UserBadge user="{{ state.user.clone() }}"></UserBadge>
  </aside>
</template>
```

With context, `App` provides the user once and `UserBadge` asks for it. `Sidebar` has no
inputs and no knowledge of the user:

```html title=HTML · with context
<Sidebar></Sidebar>

<template rust:component="Sidebar">
  <aside>
    <UserBadge></UserBadge>
  </aside>
</template>
```

## A complete example {#example}

This is that page as a working app. Create one with `fusor new my-app`, then replace
`src/app.rs` and `web/index.html` with the two files below and keep the other files.

```rust source=tutorial/lessons/context/app.rs title=src/app.rs
```

```html source=tutorial/lessons/context/index.html title=web/index.html
```

Run `fusor dev` in my-app. The sidebar shows “Signed in as Ada”. Click Switch to Grace, and
the badge changes to Grace: `App` changed the signal it provided, and `UserBadge` reads that
same signal.

- [Try shared context in a live app](/docs/showcase/context)

## How it works {#how}

The example uses three steps, which every use of context follows:

1. **Name the value with a key type.** `CurrentUser` implements `ContextKey`, whose `Value`
   says what is shared: here a `Signal<String>`.
2. **Provide it in an outer component.** `App::new` calls
   `owner.provide::<CurrentUser>(user.clone())`.
3. **Read it in any component inside.** `UserBadge::new` calls
   `owner.context::<CurrentUser>()`, which returns `None` if nothing provides it.

`owner` is the component's owner handle, which fusor passes to constructors. The rules:

| Rule | What it means |
| --- | --- |
| The key is a type | The compiler checks every lookup; there is no string name to mistype |
| The nearest provider wins | A lookup searches this component, then each component containing it |
| A component provides a key once | Providing the same key twice on one component returns an error |
| Context isn't reactive by itself | Provide a `Signal` for a value that changes, so readers update |
| A missing provider is `None` | Turn it into an error with a message that says what is missing |
| Readers share the value | The value is stored behind an `Rc`, so every reader sees the same signal |

> `Rc` is Rust's reference-counted pointer: cloning it hands out another reference to one
> value. That is why `UserBadge` holds `Rc<Signal<String>>`.

## When to use context {#when}

Prefer an input when one child needs the value: the connection stays visible in the HTML.
Use context when many components at different depths need the same value, or when passing it
through every layer would add inputs that only forward it.

- [Pass values as component inputs](/docs/components)

## Read context in a component {#component}

A component reads context through its owner handle, so its constructor needs one. The root
`<App>` receives `owner` in its `state` expression, as in `App::new(owner)?` above.

A template component such as `UserBadge` gets its owner through `FromInputs`. The derive,
used for `Sidebar`, only fills `#[input]` and `#[local]` fields, so a component that reads
context implements `FromInputs` by hand and calls its own constructor. The
`UserBadgeInputs` block at the end of `src/app.rs` is that implementation; copy it and change
the type names.

> Both constructors return `Result<Self, JsValue>`. A missing provider becomes a startup
> error with your message instead of a panic.

- [Where owner comes from](/docs/ownership#mount)

## Next steps {#next}

- [Share the signed-in user with every page](/docs/quick-guides/user-sessions)
- [How owners clean up](/docs/ownership)
