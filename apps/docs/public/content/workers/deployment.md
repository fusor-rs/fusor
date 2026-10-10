# Building and hosting workers

The normal Fusor build packages annotated work automatically. Threaded pools additionally
need a special toolchain and a hosting setup that enables browser shared memory.

## Automatic packaging {#packaging}

Use an `<App>` entry point and the usual `fusor_build::compile_app()` build script. The
annotations leave registrations in the compiled code, which lets the CLI find the annotated
tasks and services in your app and its dependencies and package them. Keep using
`fusor dev`, `fusor build`, and `fusor preview`. You don’t need a worker entry file, manual
registration, a JavaScript loader, worker URLs, or an extra Cargo target.

The worker bundle leaves out the generated UI startup code and unrelated JavaScript imports
that depend on the DOM. Code that a worker actually calls must still be usable inside a
worker. Calling an annotated Rust function directly doesn’t move it into a worker; only the
generated `run`, `stream`, and `spawn` calls do.

## Prepare the threaded toolchain {#threads}

Ordinary workers build with stable Rust and need no special headers. Pools need a threaded
build, which Fusor selects automatically when the compiled code has a pool annotation or
uses `Pool`. That build currently relies on `nightly-2025-11-15` with `rust-src`, Rayon
1.11.0, and wasm-bindgen-rayon 1.3.0.

Run `fusor install` for your application to prepare the required tools. When online,
`fusor dev` can install missing tools itself. `fusor build`, including a debug build,
expects them to be installed already and reports the fix if they are missing. Offline builds
need the toolchain and dependencies to be cached beforehand.

```sh title=Terminal · run in your application directory
fusor install
fusor build
fusor preview
```

## Serve pools in a secure, isolated context {#headers}

Pools use shared memory, which browsers allow only in a secure context, such as HTTPS or
localhost, that is also cross-origin isolated. Isolation is enabled by two response headers.
`fusor dev` and `fusor preview` send them for threaded applications. In production,
configure your host to send them on the document and on the worker assets, because page
JavaScript can’t set them.

Deploy the complete generated output, worker assets included, as one unit. The embedder
policy also applies to external scripts, images, and other resources, which must satisfy the
browser’s CORS or resource-policy rules.

```text title=HTTP · production response headers
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

- [MDN: shared memory
  requirements](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer)
- [MDN: cross-origin embedder
  policy](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Cross-Origin-Embedder-Policy)

## Check browser support {#capabilities}

`fusor_worker::capabilities()` returns
`Capabilities { dedicated_workers, shared_memory, hardware_parallelism }`, describing what
the current environment supports. The hardware count is a hint rather than a promise that
those cores are free, and pool initialization limits its requested threads to it. On native
targets both boolean fields are false, and background jobs do not silently run locally.

You can use these fields to tell the user why a feature is unavailable before attempting
initialization. Still handle the initialization result, because the checks don’t guarantee
that loading, memory allocation, or the page’s content security policy will allow startup. A
page without cross-origin isolation produces
`Unsupported { capability: Capability::SharedMemory }`.

```rust title=Rust · inside UI code
let support = fusor_worker::capabilities();
if !support.shared_memory {
    // Explain the pool requirement or offer a smaller ordinary task.
}
// After successful initialization, pool.threads() reports its thread count.
```

- [capabilities() reference](/docs/workers/api#capabilities)

## Networking in workers {#network}

Workers can call Fetch, including through Fusor’s cancellation-aware helper. Relative URLs
passed to Fusor’s helper resolve against the app’s base URL, while raw browser Fetch
resolves them the way workers normally do, relative to the worker. The browser’s usual rules
for CORS, credentials, and content security policy still apply. Workers have no access to
the page’s DOM and shouldn’t assume a `window` global.

When a request doesn’t feed substantial processing, an ordinary async request on the UI
thread is usually enough. A worker helps when the requests feed expensive work or state that
already lives there.

- [See a cancellation-aware request](/docs/workers/tasks#fetch)
- [MDN: APIs and restrictions in
  workers](https://developer.mozilla.org/en-US/docs/Web/API/Web_Workers_API/Using_web_workers)

## Diagnose a failed startup {#troubleshooting}

Ordinary workers and pool coordinators must become ready within ten seconds. Each compute
worker has the same startup deadline. A stalled startup fails with `Load`, terminates the
runtime's workers, and completes its pending jobs with the failure. Unexpected transport
failures while cancelling, disposing, or returning stream credit also terminate the runtime.

`PoolRequired` means an operation needs a pool: add `.on(&pool)` to it, or use an ordinary
task that doesn’t need pool capabilities. For `Unsupported`, check the capabilities, that
the page is served over HTTPS, and that the document has the isolation headers. If the build
fails with a toolchain error, run the installation command the CLI prints.

For `Load`, look at the browser’s network and console output. The worker and Wasm assets
must exist at the generated URLs, and the content security policy must allow those resources
and Wasm execution. `IncompatibleArtifact` means the deployed files come from mismatched
builds, so rebuild and deploy one complete, matching generation of the application. If a
worker crashes, create replacement state deliberately rather than blindly retrying an
operation that changes state.

- [Runtime error reference](/docs/workers/lifecycle#runtime-errors)
