# fusor-std

Typed forms and save actions for fusor.

With `forms`, fields such as `TextField<T>` bind to form controls with `bind` and
validate into typed values. With `actions`, saves run through an explicit policy
and keep newer edits made while a save is in flight. The `resources`, `query`
and `routing` features re-export `fusor-async`, `fusor-query` and
`fusor-router`, so one dependency covers them. No features are on by default.

Add it to an application with `fusor add forms` or `fusor add actions`.

Part of [fusor](https://github.com/fusor-rs/fusor), which builds reactive web applications from HTML and ordinary Rust. Licensed under the [MIT License](LICENSE).
