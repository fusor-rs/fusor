//! Reactive Rust for HTML script blocks, compiled by Cargo without a Rust DSL.
//!
//! ```
//! use fusor::{signal, derived, effect, batch};
//! let count = signal(0);
//! let doubled = derived({ let count = count.clone(); move || count.get() * 2 });
//! let subscription = effect(move || println!("{}", doubled.get()));
//! batch(|| { count.set(2); count.set(3); });
//! // Prints 0, then 6. Dropping the subscription stops it.
//! drop(subscription);
//! ```

#[doc(hidden)]
pub mod attribute;
#[doc(hidden)]
pub mod authoring;
pub mod bind;
mod cleanup;
pub mod coherence;
mod owner;
mod reactive;
#[doc(hidden)]
pub mod render;
pub use cleanup::{Cleanup, CleanupEffect, effect_with_cleanup};
pub use owner::{ContextError, ContextKey, Owner, OwnerHandle, Registration};
/// Source-version snapshots for supported renderer and async-read integration.
#[doc(hidden)]
pub use reactive::versions;

/// Versioned contract shared by the HTML compiler and the DOM runtime.
/// This is an implementation interface, not application authoring syntax.
#[doc(hidden)]
pub mod template;
pub use reactive::{
    Derived, Effect, Memo, Signal, batch, derived, effect, memo, memo_with_eq, signal, untrack,
};

/// The explicit input contract for compiler-resolved component tags.
///
/// `Inputs` is a named-field struct (or a unit struct for empty tags) that
/// implements [`Inputs`]. The compiler builds it through that trait's builder:
/// rustc checks every supplied input, its visibility, and its type, and rejects a
/// missing required input by name. Input expressions and this constructor run once
/// per mounted identity, untracked. Pass signals or memos for live shared inputs.
/// Construction is separate from rendering: browser mounts require `Component`.
/// `owner` is the new child's prepared owner. The caller owns preparation,
/// activation, rollback, and conversion of construction errors. This trait does
/// not itself defer ordinary effects or activate the owner.
///
/// Simple concrete components can use `#[derive(fusor::FromInputs)]`:
/// mark every field `#[input]` (from the parent; optional when it has a default,
/// see [`Inputs`]) or `#[local(init = expression)]` (initialized once per
/// instance). The derive generates a `TypeNameInputs` struct with the
/// component's visibility and public input fields, plus this trait
/// implementation. Original field visibility is unchanged. Unit and all-local
/// components get unit Inputs.
///
/// Local expressions execute in declaration order in the generated constructor.
/// In local and default expressions, `Self` refers to the component. They have
/// no implicit `inputs`/`owner` names and cannot access other instance fields.
/// Implement this trait manually for input-dependent initialization, owner-aware
/// setup, or generic components, and derive [`Inputs`] on the hand-written inputs
/// struct. The derive creates neither a `new` method nor a template association.
/// Use `#[from_inputs(crate = ::alias)]` when renaming the runtime dependency.
/// Enable the `derive` feature to use the derive without any browser dependencies;
/// `dom` also enables it. The derived error type is `std::convert::Infallible`.
///
/// # Migrating manual implementations
///
/// Existing browser implementations add `type Error = fusor::dom::JsValue;` and
/// keep their return signature. `fusor::dom::FromInputs` is this same trait.
/// Portable implementations choose their own error type. Browser mounting uses
/// `fusor::dom::IntoMountError`: it preserves `JsValue` and supports `Infallible`,
/// `String`, and `&str`. Implement it for custom browser errors; there is no
/// blanket `Display` conversion.
pub trait FromInputs: Sized {
    type Inputs: Inputs;
    type Error;
    fn from_inputs(inputs: Self::Inputs, owner: OwnerHandle) -> Result<Self, Self::Error>;
}

/// How the compiler builds [`FromInputs::Inputs`] from a component tag.
///
/// Generated code starts from [`Inputs::builder`], calls one setter per supplied
/// attribute, named after its field, then calls the builder's `build()`. Setters
/// keep each field's visibility and exact Rust type. An omitted input takes its
/// default; omitting a required input is a compile error that names the input.
///
/// Use `#[derive(fusor::Inputs)]` on a hand-written inputs struct. A field whose
/// type is written as `Option<T>` (or `std::option::Option<T>`) defaults to `None`
/// unless marked `#[input(required)]`; an alias of `Option` stays required. Other
/// fields are required unless marked `#[input(default)]`, which uses
/// `Default::default()`, or `#[input(default = expression)]`. Defaults are
/// evaluated by `build()`, only for omitted inputs, in field declaration order.
/// A field cannot be named `build`.
/// `#[derive(FromInputs)]` implements this trait for the struct it generates.
/// Use `#[inputs(crate = ::alias)]` when renaming the runtime dependency.
pub trait Inputs: Sized {
    /// A builder with every input unset.
    type Builder;
    fn builder() -> Self::Builder;
}

#[cfg(feature = "dom")]
pub mod dom;

#[cfg(feature = "derive")]
pub use fusor_macros::{FromInputs, Inputs};

#[cfg(feature = "javascript")]
pub mod js;
#[cfg(feature = "javascript")]
pub use fusor_macros::JsInputs;
#[cfg(feature = "javascript")]
pub use js::JsInputs;

/// The small set of types needed by a reactive application.
pub mod prelude {
    #[cfg(feature = "javascript")]
    pub use crate::JsInputs;
    #[cfg(feature = "dom")]
    pub use crate::dom::{Component, Content, Scope, TemplateComponent, document, element};
    pub use crate::{Cleanup, CleanupEffect, effect_with_cleanup};
    pub use crate::{ContextError, ContextKey, Owner, OwnerHandle};
    pub use crate::{
        Derived, Effect, Memo, Signal, batch, derived, effect, memo, memo_with_eq, signal, untrack,
    };
    pub use crate::{FromInputs, Inputs};
}
