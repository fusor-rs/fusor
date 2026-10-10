# Island setup

Build the working catalog, then follow the files that connect its initial HTML to separately
loaded Rust/Wasm. Read Islands first for the rendering and reactivity model.

## Build the working catalog first {#run}

The complete example lives under `examples/islands` in the framework checkout. It already
has the manifests, renderer, descriptors, templates, and browser registrations.

Install Node for this build workflow, then use the CLI from Installation. The build runs the
native renderer and saves the HTML; the serve command only serves the generated files.

Build the catalog and serve its output; open http://127.0.0.1:8094/. These commands run in
the framework repository, not in your generated `my-app`.

```sh title=Terminal
# From the fusor repository root:
fusor build --package catalog-site --locked
fusor preview examples/islands/site/dist --port 8094
```

> The catalog also contains an async designer test fixture whose API responses are supplied
> by browser tests. The native cart/activation example runs from the static build; use
> `just test islands` to exercise the controlled async scenarios.

## Follow the packages through the build {#packages}

The native site links descriptors and native-renderable views. Each browser unit links the
interactive view implementation it needs. A shared loader connects the resulting HTML hosts
to those units. This explicit split is what avoids sending every feature in one initial Wasm
bundle.

```text title=Repository example layout
examples/islands/
  types/           Cart, CartProps, Designer: shared contracts
  views/           CartView: state + shared cart.html template
  cart-web/        browser registration of CartView
  designer-views/ browser designer implementation
  designer-web/   browser registration of Designer
  site/            native renderer + delivery configuration
  grouped-web/    alternative grouping for cost comparison
```

## Put dependencies in the packages that use them {#dependencies}

The existing fixture inherits paths from the workspace. For your own workspace, define the
registry dependencies below in its root `Cargo.toml` and inherit them in member packages.

The descriptor crate uses islands and serde; the native site uses server and islands; a
browser unit enables dom/islands on fusor and browser on fusor-islands. Keep each unit’s
generated HTML setup and build dependency as in a normal app.

```toml title=Cargo.toml · root of a separate workspace
[workspace.dependencies]
fusor-core = { version = "=0.1.5" }
fusor-build = { version = "=0.1.5" }
fusor-islands = { version = "=0.1.5" }
fusor-server = { version = "=0.1.5" }
serde = { version = "1", features = ["derive"] }
wasm-bindgen = "=0.2.117"
```

> In the browser member’s `[dependencies]`, use
> `fusor-core = { workspace = true, features = ["islands"] }` and
> `fusor-islands = { workspace = true, features = ["browser"] }`. The setting
> `workspace = true` requires those dependencies to be defined in the workspace manifest.

## Define what Cart means {#units}

`Cart` here is a descriptor, not the `CartView` state type that owns the template.

Its associated `Props` type defines the serialized input; `UNIT` selects a separately
compiled browser package, and `SCHEMA` names the props contract.

The default mode attaches to matching native HTML. The full fixture defines `Designer` with
an explicit Preview mode as well.

```rust title=types/src/lib.rs · Cart definition
use fusor_islands::Island;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct CartProps {
    pub product_id: u64,
    pub title: String,
    pub quantity: String,
}
pub struct Cart;
impl Island for Cart {
    type Props = CartProps;
    const NAME: &'static str = "catalog.cart";
    const UNIT: &'static str = "cart";
    const SCHEMA: &'static str = "catalog.cart.props.v1";
}
```

> A descriptor is a small Rust type that identifies the feature across the native renderer
> and browser bundle. It does not own the UI state. `CartView` owns the template and
> signals. For named tag inputs, `Props` is a struct with public named fields; rustc checks
> the supplied fields and their types.

- [All catalog descriptors](/docs/source/catalog-types.rs.txt)
- [CartView state](/docs/source/catalog-views.rs.txt)
- [CartView template](/docs/source/catalog-views.html.txt)

## Connect native and browser implementations {#renderer}

`fusor-server` renders resolved data into HTML in native Rust; it does not require an HTTP
framework.

The site registers `Cart` with a `CartView` factory and renders using a manifest plus that
registry. cart-web exports the corresponding browser factory below.

Native rendering returns `fusor_server::Result<T>` with a typed `Error`. Serialization and
delivery failures retain their error sources. A failed or panicking island render releases
its instance ID and restores the context, so the caller can reuse it after handling failure.

The delivery metadata maps the unit name cart to that Cargo package. This is extra package
setup, not an attribute that automatically splits an ordinary app.

```rust title=cart-web/src/lib.rs · browser registration
fusor_islands::export!(
    fusor_islands::browser::Unit::new()
        .entry::<catalog_types::Cart, catalog_views::CartView>(
            |_, props| catalog_views::CartView::new(props)
        )
);
```

> The native site executable implements the CLI render protocol; copying this registration
> alone is not a complete native app. Use the linked executable and manifest as the working
> starting point. Unit registration stays side-effect free; activation constructs state.

- [Complete native rendering executable](/docs/source/catalog-site.rs.txt)
- [Site and delivery manifest](/docs/source/catalog-site.toml.txt)

## Control an island from already active Rust {#controls}

An active component can look up an island by descriptor and instance id.

The methods below belong inside an async task started after that caller’s owner activates.
The handle uses the same loader as HTML policies.

Lookup/status do not download code. A dropped waiter or disposed caller cancels its wait; it
does not dispose an already active target.

Load and binding failures retain the registry message and original JavaScript `Error.cause`
in `RegistryError`. A failed preview commit reports a failed fallback restoration too.

```rust title=Rust · inside an active caller’s async task
let cart = fusor_islands::browser::get::<Cart>(&owner, "cart-42")?;
cart.prefetch().await?;
cart.activate().await?;
```

> Add `hydrate:id="cart-42"` to that `Cart` tag to make this lookup work.

## Communicate between active islands {#communication}

`fusor_islands::browser::emit::<Descriptor, Payload>` sends an explicitly named message with
a serializable payload. `browser::listen::<Descriptor, Payload>` registers an owner-scoped
listener that decodes it.

The callback receives `Result<Payload, fusor_islands::Error>`: `MessagePayload` rejects
non-text payloads, and `Decode` retains the JSON decoding cause. Delivery manifest errors
also identify the invalid metadata or unregistered descriptor and unit.

The receiver decides how to update its own signals.

Messages do not carry signal handles and do not `hydrate` a sleeping receiver. Put shared,
tightly coupled reactive state inside one island instead of treating events as shared
memory.

> Listeners are cleaned up with their owner. The payload types must agree between sender and
> receiver; the descriptor identifies the event namespace. The linked API source contains
> the exact signatures and lifecycle rules.

- [Typed island event and control APIs](/docs/source/island-browser.rs.txt)
- [State inside and between islands](/docs/islands#boundaries)
