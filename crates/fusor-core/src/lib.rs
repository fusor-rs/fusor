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
pub mod authoring;
pub mod bind;
mod cleanup;
pub mod coherence;
mod owner;
mod reactive;
pub use cleanup::{Cleanup, CleanupEffect, effect_with_cleanup};
pub use owner::{ContextError, ContextKey, Owner, OwnerHandle, Registration};
#[doc(hidden)]
pub use reactive::versions;

/// Versioned contract shared by the HTML compiler and the DOM runtime.
/// This is an implementation interface, not application authoring syntax.
#[doc(hidden)]
pub mod template;
pub use reactive::{
    Derived, Effect, Memo, Signal, batch, derived, effect, memo, memo_with_eq, signal, untrack,
};

#[cfg(feature = "dom")]
pub mod dom;

#[cfg(feature = "dom")]
pub use dom::FromInputs;
#[cfg(feature = "dom")]
pub use fusor_macros::FromInputs;

#[cfg(feature = "javascript")]
pub mod js;
#[cfg(feature = "javascript")]
pub use fusor_macros::JsInputs;
#[cfg(feature = "javascript")]
pub use js::JsInputs;

/// The small set of types needed by a browser application.
pub mod prelude {
    #[cfg(feature = "dom")]
    pub use crate::FromInputs;
    #[cfg(feature = "javascript")]
    pub use crate::JsInputs;
    #[cfg(feature = "dom")]
    pub use crate::dom::{Component, Content, Scope, TemplateComponent, document, element};
    pub use crate::{Cleanup, CleanupEffect, effect_with_cleanup};
    pub use crate::{ContextError, ContextKey, Owner, OwnerHandle};
    pub use crate::{
        Derived, Effect, Memo, Signal, batch, derived, effect, memo, memo_with_eq, signal, untrack,
    };
}
