# Resource API

A browser resource manages one independently displayed request. You supply its lifetime,
changing key, and loader; it manages state and obsolete results.

## browser::resource(owner, key, load) {#resource}

This is a public function in `fusor_async::browser`, not a special compiler feature. It
returns `Resource<K, T, E>` and uses the browser executor to run your Rust future.

It takes exactly three arguments: a borrowed `OwnerHandle`, a synchronous key function, and
a loader function.

In this excerpt, `owner` is a parameter of `Reader::new`. `Reader::from_inputs` receives the
child’s handle from the framework and passes it into that constructor; `&owner` borrows it
for the resource.

```rust title=Rust · constructor excerpt
use fusor_async::{browser, fetch};

// Inside Reader::new(owner: OwnerHandle, selected_id: Signal<u32>):
let data = browser::resource(
    &owner,                              // whose lifetime?
    move || Some(selected_id.get()),     // which request?
    |id, cancel| async move {            // how to load it?
        fetch::get_text(&format!("/data/{id}.txt"), &cancel).await
    },
);
```

> The result here is `Resource<u32, String, FetchError>`: request key, successful value,
> error. `&owner` borrows the handle; move lets the stored closure keep `selected_id`. The
> async feature is in a separate crate with `features = ["browser"]`.

- [Dependency setup](/docs/async-data#dependency)
- [Complete Reader Rust file](/docs/async-data#resource)
- [What is owner, and where does it come from?](/docs/ownership/mounting#owner-variable)

## The key function tracks changing inputs {#key}

The second argument runs synchronously and tracks the signals it reads. Return `Some(key)`
to load; return `None` to disable and clear previous data.

A changed key supersedes the old attempt. Equal keys keep the current request; `refresh()`
explicitly starts another attempt.

Put all changing inputs into the key, such as an id and a locale.

```rust title=Rust · alternative key closure
move || Some((selected_id.get(), locale.get()))
```

> K must implement `Clone` + `PartialEq` and be owned long enough to be stored ('static).
> `Signal` reads performed only inside the async loader are not reactive dependencies.

## The loader receives key and CancellationToken {#loader}

The third argument is your function. The framework passes the selected K and a
`CancellationToken` for this attempt; the function returns a `Future` yielding
`Result<T, E>`.

`Ok(value)` succeeds and `Err(error)` fails. Use your own HTTP client, parser, cache,
authentication, or other async work.

Captured data and the future must be 'static; owned values and cloned handles usually
satisfy this.

> Work starts when the owner is active. A later key or disposal revokes the old attempt’s
> permission to publish. Stale results cannot overwrite the current result, even if the
> underlying transport cannot be stopped.

## Read state with Resource::with {#state}

`resource.with(|state| ...)` gives a borrowed `ResourceState<K, T, E>`. When called from an
HTML binding, it subscribes that binding to changes.

`resource.get()` returns an owned state snapshot.

`ResourceState::data()` returns available successful data, including the previous result
during Loading or Error; always use `data.key` to label that result.
The returned `ResourceData<K, T>` retains the request key and a shared `Rc<T>` payload.

```rust title=Rust · read the resource from a binding method
let text = data.with(|state| {
    state.data().map_or_else(String::new, |data| {
        format!("Issue {}: {}", data.key, data.value)
    })
});
```

> Idle: no selected key. Loading { key, previous }: an attempt is pending. Ready(data):
> success. Error { key, error, previous }: failed attempt. Disposed: terminal. `data.value`
> is `Rc<T>`, a shared pointer to the successful payload; `data.key` identifies the request
> that produced it.

## refresh(), is\_loading(), and dispose() {#refresh}

`resource.refresh()` retries the current key explicitly; it does not create a new component.
`ResourceState::is_loading()` reports an attempt in progress. `resource.dispose()` cancels
this resource permanently; normal component disposal already does that. There is no built-in
automatic retry or shared cache.

- [Loading and retry controls in HTML](/docs/async-data#display)

## get\_text and custom requests {#request}

`fetch::get_text(&url, &cancel).await` performs a browser GET, rejects non-2xx status codes,
and returns the response body as `String` or a `FetchError`. Its variants are `Cancelled`,
`Status` and `Js`. `FetchError` implements `Display`, so it also works as the error of
`browser::read`.

For other methods, headers or bodies, build the request with `web-sys` or your own client
and pass `cancel.abort_signal()` as its signal. The signal lives as long as the attempt, so
replacing the key or removing the component aborts the fetch, including the body read.

```rust title=Rust · inside a loader
let options = web_sys::RequestInit::new();
options.set_method("POST");
options.set_signal(Some(&cancel.abort_signal()?));
options.set_body(&JsValue::from_str(&body));
let response = JsFuture::from(window.fetch_with_str_and_init(&url, &options)).await?;
```

> `get_text` is an optional convenience, not a required data layer. It does not deserialize
> JSON or add your app’s authentication policy. Browser Fetch caching rules still apply.
>
> Enable the `web-sys` features your own request code calls, such as `RequestInit`.

## Adapt a cancellable client {#cancel}

`cancel.is_cancelled()` checks whether the attempt was cancelled.
`cancel.on_cancel(callback)` registers transport cleanup and returns a `CancelRegistration`
guard; keep that guard alive for the request.

Dropping it unregisters the callback. Prefer `abort_signal()` for Fetch-based clients; use
`on_cancel` for timers, sockets, and SDKs with their own cancel method.

Cancellation protects local work and publication; it cannot undo a server-side mutation.

- [Observe cancellation in the browser](/docs/async-data#cancel)

## browser::read is the coherent counterpart {#read}

`browser::read(&owner, key, load)` declares an `AsyncValue<K, T, E>` consumed by `<Await>`.
The key returns K directly and E must implement Display.

Standalone `<Await>` updates independently; an enclosing `<Async>` coordinates its reads.

Declaring a read does not start it; it starts when its view reaches it. `Resource` remains
the separate state-machine API for custom loading UI.

- [See browser::read in the complete Price and Stock
  file](/docs/coherent-async#read-declaration)
- [Coherent status, retry, and limitations](/docs/coherent-async/semantics)
