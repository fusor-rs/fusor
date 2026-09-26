//! Routing by URL pattern, shared by HTML routes and custom routers.
use super::{
    NavigateOptions, NavigationDriver, PreparedNavigation, error,
    tree::{Boundary, Mounted, Selection, Tree, Views, enclosing_view},
};
use crate::{
    AppUrl,
    pattern::{Match, Pattern, ambiguous},
    select::select,
};
use fusor::dom::{MountPoint, Scope};
use fusor::{Derived, OwnerHandle, derived};
use std::rc::Rc;
use wasm_bindgen::JsValue;

type Factory = dyn Fn(&OwnerHandle, &Match) -> Result<Scope, JsValue>;

/// A lazy branch factory. Unmatched branches are never constructed.
pub struct RouteView {
    pattern: Option<Pattern>,
    render: Box<Factory>,
}

impl RouteView {
    pub fn new(
        pattern: &str,
        render: impl Fn(&OwnerHandle, &Match) -> Result<Scope, JsValue> + 'static,
    ) -> Result<Self, JsValue> {
        Ok(Self {
            pattern: Some(Pattern::new(pattern)?),
            render: Box::new(render),
        })
    }
    pub fn fallback(
        render: impl Fn(&OwnerHandle, &Match) -> Result<Scope, JsValue> + 'static,
    ) -> Self {
        Self {
            pattern: None,
            render: Box::new(render),
        }
    }
}

/// A router's routes, checked to be unambiguous.
struct Routes(Vec<RouteView>);

impl Routes {
    fn new(routes: Vec<RouteView>) -> Result<Self, JsValue> {
        for (index, route) in routes.iter().enumerate() {
            let pattern = route.pattern.as_ref();
            if routes[..index]
                .iter()
                .any(|other| ambiguous(pattern, other.pattern.as_ref()))
            {
                return Err(error(
                    "Router contains ambiguous routes or multiple fallbacks",
                ));
            }
        }
        Ok(Self(routes))
    }
}

impl Views for Routes {
    fn select(&self, url: &AppUrl, prefix: usize) -> Result<Option<Selection<'_>>, JsValue> {
        let patterns = self.0.iter().map(|route| route.pattern.as_ref());
        Ok(select(patterns, url, prefix)?.map(|selected| {
            let route = &self.0[selected.index];
            Selection {
                consumed: selected.matched.consumed,
                render: Box::new({
                    let matched = selected.matched.clone();
                    move |owner| (route.render)(owner, &matched)
                }),
                identity: Box::new(selected),
            }
        }))
    }
}

/// Navigation shared by nested routers, independent of the declaring template file.
#[derive(Clone)]
pub struct Navigation {
    tree: Rc<Tree>,
    /// Looked up from a view that is still prepared: its owner and the URL it
    /// was prepared for, reported until the view activates.
    prepared: Option<(OwnerHandle, AppUrl)>,
}

impl Navigation {
    pub fn from_owner(owner: &OwnerHandle) -> Option<Self> {
        Some(Self {
            tree: Tree::from_owner(owner)?,
            prepared: enclosing_view(owner),
        })
    }
    pub fn location(&self) -> Derived<AppUrl> {
        let location = self.tree.location();
        let prepared = self.prepared.clone();
        derived(move || {
            let current = location.get();
            prepared
                .as_ref()
                .filter(|(owner, _)| !owner.is_active() && !owner.is_disposed())
                .map_or(current, |(_, initial)| initial.clone())
        })
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
            prepared: None,
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
    if enclosing_view(&scope.owner()).is_some() {
        return Boundary::nest(scope, target, routes);
    }
    Tree::mount_root_in(scope, target, base, target.parent_element()?, routes)
}
