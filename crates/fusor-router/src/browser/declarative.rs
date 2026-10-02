//! Browser navigation around the shared pattern route tree.
use super::{
    NavigateOptions, NavigationDriver, PreparedNavigation,
    tree::{Mounted, Tree},
};
use crate::{
    AppUrl,
    view::{self, Routes},
};
use fusor::dom::{MountPoint, Scope};
use fusor::{Derived, OwnerHandle};
use std::rc::Rc;
use wasm_bindgen::JsValue;

/// A lazy DOM route factory for [`ViewRouter`] or [`mount_routes`].
///
/// `RouteView::new(pattern, render)` validates a route pattern, returning
/// `Result<RouteView, JsValue>`. `RouteView::fallback(render)` creates the view
/// used when no pattern matches. Both accept a
/// `Fn(&OwnerHandle, &crate::pattern::Match) -> Result<Scope, JsValue> + 'static`.
/// Captures in `Match::params` are decoded. Unmatched factories do not run.
///
/// Return a detached scope prepared under the supplied owner. The router
/// attaches and activates it; the factory must not commit it itself.
///
/// ```no_run
/// use fusor::{OwnerHandle, dom::{Scope, document}};
/// use fusor_router::browser::declarative::RouteView;
/// use wasm_bindgen::JsValue;
///
/// fn page(owner: &OwnerHandle, text: &str) -> Result<Scope, JsValue> {
///     let element = document()?.create_element("p")?;
///     element.set_text_content(Some(text));
///     let mut scope = Scope::new(element);
///     scope.prepare_owner(Some(owner));
///     Ok(scope)
/// }
///
/// let routes = vec![
///     RouteView::new("/pages/:name", |owner, matched| {
///         page(owner, &matched.params["name"])
///     })?,
///     RouteView::fallback(|owner, _| page(owner, "Page not found")),
/// ];
/// # Ok::<(), JsValue>(())
/// ```
pub type RouteView = view::RouteView<Scope>;

/// Navigation shared by nested routers, independent of the declaring template file.
#[derive(Clone)]
pub struct Navigation {
    tree: Rc<Tree>,
    view: view::Navigation<Scope>,
}

impl Navigation {
    pub fn from_owner(owner: &OwnerHandle) -> Option<Self> {
        Some(Self {
            tree: Tree::from_owner(owner)?,
            view: view::Navigation::from_owner(owner)?,
        })
    }
    pub fn location(&self) -> Derived<AppUrl> {
        self.view.location()
    }
    pub fn navigate_url(&self, url: &str, options: NavigateOptions) -> Result<(), JsValue> {
        self.tree.navigate_url(url, options)
    }
    pub fn last_error(&self) -> Option<JsValue> {
        self.tree.last_error()
    }
}

/// Reusable view selector without browser history. Custom routers can stage a
/// transition, publish their own location, then commit; dropping the stage rolls back.
/// Finish a stage synchronously before yielding to the browser: preparation inserts
/// inactive DOM while keeping the previous view alive. Only one stage may exist per tree.
#[derive(Clone)]
pub struct ViewRouter(Rc<Tree>);

impl ViewRouter {
    pub fn mount(
        scope: &mut Scope,
        target: &MountPoint,
        routes: Vec<RouteView>,
        url: AppUrl,
    ) -> Result<Self, JsValue> {
        let tree = Tree::mount(&scope.owner(), target, Box::new(Routes::new(routes)?), url)?;
        scope.retain(Mounted(tree.clone()));
        Ok(Self(tree))
    }
    pub fn navigation(&self) -> Navigation {
        Navigation {
            tree: self.0.clone(),
            view: self.0.navigation(),
        }
    }
    pub fn navigate(&self, url: AppUrl) -> Result<(), JsValue> {
        self.0.navigate(url)
    }
}

impl NavigationDriver for ViewRouter {
    fn prepare_navigation(&self, url: &AppUrl) -> Result<Box<dyn PreparedNavigation>, JsValue> {
        self.0.prepare_navigation(url)
    }
}

/// Mount an HTML route selector. Inside a router view it matches after that
/// view's prefix; otherwise it becomes the page's router, owning browser
/// navigation and its cleanup.
pub fn mount_routes(
    scope: &mut Scope,
    target: &MountPoint,
    base: &str,
    routes: Vec<RouteView>,
) -> Result<(), JsValue> {
    let routes = Box::new(Routes::new(routes)?);
    if view::enclosing_view::<Scope>(&scope.owner()).is_some() {
        scope.retain(view::Boundary::nest(&scope.owner(), target, routes)?);
        return Ok(());
    }
    Tree::mount_root_in(scope, target, base, target.parent_element()?, routes)
}
