//! Owned browser navigation and composable route views.
//!
//! One view tree does all routing. HTML `<Router>`/`<Route>` tags and
//! [`declarative::ViewRouter`] select views by URL pattern; the typed
//! [`Router`] selects them by [`Route`](crate::Route). The root of a tree can
//! own browser history, which feeds it URLs and presents each navigation.
pub mod declarative;
mod history;
mod tree;
mod typed;
pub use typed::{RouteContext, Router, mount_outlet};

use crate::view::{INACTIVE, REENTRANT};
use crate::{AppUrl, UrlError};
use fusor::ContextError;
use std::cell::Cell;
use wasm_bindgen::JsValue;

fn error(message: &str) -> JsValue {
    JsValue::from_str(message)
}
fn context_error(error: ContextError) -> JsValue {
    JsValue::from_str(&error.to_string())
}
impl From<UrlError> for JsValue {
    fn from(error: UrlError) -> Self {
        JsValue::from_str(error.0)
    }
}

/// Defaults match following a link to new content. Query updates may opt to
/// preserve focus and scroll. Back/forward uses native browser scroll restoration.
#[derive(Clone, Copy, Debug, Default)]
pub struct NavigateOptions {
    pub replace: bool,
    pub keep_focus: bool,
    pub keep_scroll: bool,
}

pub use crate::view::PreparedNavigation;

/// A view manager participating in the browser history transaction.
pub trait NavigationDriver {
    fn prepare_navigation(&self, url: &AppUrl) -> Result<Box<dyn PreparedNavigation>, JsValue>;
}

/// Holds a flag until dropped; taking a flag that is already held fails.
struct Flag<'a>(&'a Cell<bool>);
impl<'a> Flag<'a> {
    fn take(flag: &'a Cell<bool>, message: &str) -> Result<Self, JsValue> {
        if flag.replace(true) {
            return Err(error(message));
        }
        Ok(Self(flag))
    }
}
impl Drop for Flag<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}
