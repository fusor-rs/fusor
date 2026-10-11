# User sessions

Show who is signed in, let people sign in and out, and show some pages only to signed-in
people. Your server owns the session; the page asks the server who is signed in and shares
the answer with every component through context.

## How it fits together {#overview}

The page never handles a password or a token. The server keeps the session in a cookie that
page scripts can't read, and the page only asks “who is signed in?”.

```text title=Text · Sign-in flow
Sign-in form → POST /api/sign-in → server sets a session cookie → page reloads
Page loads   → GET /api/me       → 200 with a name, or 401      → Session in context
```

- [What context is](/docs/context)

## What your server provides {#server}

Any backend works, in any language, as long as it answers these three requests:

| Request | What the server does |
| --- | --- |
| `GET /api/me` | Answers 200 with the signed-in person's name as text, or 401 when nobody is signed in |
| `POST /api/sign-in` | Checks the form fields, sets the session cookie, and redirects to `/` with 303 |
| `POST /api/sign-out` | Clears the session cookie and redirects to `/` with 303 |

Set the cookie with `HttpOnly` so page scripts can't read it, `SameSite=Lax` so other sites
can't send it with their own forms, `Secure` in production, and `Path=/`. A real sign-in
checks a password or delegates to an identity provider; this guide only shows the shape.

> The page and the API must share an origin so the browser sends the cookie. `fusor dev`
> serves only your app and doesn't forward `/api` requests. Serve both from one origin, for
> example with your API serving `dist/`, or a reverse proxy in front of both.

## Add the async crate {#dependency}

The session is read with an HTTP request, which needs `fusor-async`. In your app directory:

```sh title=Terminal
fusor add async
```

- [Async data loading](/docs/async-data)

## Ask the server who is signed in {#session}

Replace `src/app.rs` in the app from `fusor new my-app` with this file:

```rust source=tutorial/lessons/session/app.rs title=src/app.rs
```

What each part does:

- **`Session`** wraps one request to `/api/me`. It starts when the page loads, and
  `me.refresh()` repeats it.
- **`status()`** turns the request's state into the four cases the page shows. A 401 means
  signed out; any other failure means the session couldn't be checked.
- **`CurrentSession`** is the context key. `App` provides the session once, at the root.
- **`AccountMenu`** reads the session through context, so `App` doesn't pass it anything.
  The `FromInputs` block at the end gives it its owner handle.

## Show each state {#page}

Replace `web/index.html` with this file:

```html source=tutorial/lessons/session/index.html title=web/index.html
```

The sign-in and sign-out buttons are ordinary HTML forms. The browser posts them to your
server, follows the redirect, and reloads the page, which then asks `/api/me` again.

## Keep pages for signed-in people {#protect}

Content inside the `SessionStatus::SignedIn` case appears only after `/api/me` confirms who
is signed in. Put signed-in-only pages there. In an app with routing, place the `Router`
inside that case, or call `navigate` to send signed-out people to a sign-in page.

> Hiding a page is not security. Your server must check the session cookie on every API
> request that reads or changes private data.

- [Routing and navigation](/docs/routing)

## Security checklist {#security}

- Keep the session in an `HttpOnly` cookie. Don't store tokens in `localStorage`, where any
  script on the page can read them.
- Set `SameSite=Lax` (or `Strict`) and, in production, `Secure`.
- Check the session on the server for every private request.
- Sign out with a `POST`, as above, not a link, so another site can't sign people out.
