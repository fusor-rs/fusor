# fusor-router

Typed routes, URLs and owned route views for fusor.

Routes can be an ordinary Rust enum implementing `Route`, or URL patterns used
by HTML `Router`/`Route` tags. Parsing and formatting are plain functions, and
every URL is relative to the application's base path. Enable `browser` for
outlets and navigation through browser history. Add it to an application with
`fusor add router`.

Part of [fusor](https://github.com/fusor-rs/fusor), which builds reactive applications from HTML and ordinary Rust. Licensed under the [MIT License](LICENSE).
