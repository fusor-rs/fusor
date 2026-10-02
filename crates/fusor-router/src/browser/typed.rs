//! Routing by an application's own [`Route`] type, on the shared view tree.
use super::{
    NavigateOptions, error,
    tree::{Selection, Tree, Views},
};
use crate::{AppUrl, Location, Route};
use fusor::dom::{Content, MountPoint, Scope};
use fusor::{Derived, OwnerHandle, derived};
use std::{marker::PhantomData, rc::Rc};
use wasm_bindgen::JsValue;
use web_sys::Element;

/// Input to a route constructor. Use `Component::prepare(&context.parent, ...)`
/// to create the view. Location is reactive and belongs to this view instance.
pub struct RouteContext<R: Route> {
    pub parent: OwnerHandle,
    pub location: Derived<Location<R>>,
}

type Render<R> = dyn Fn(RouteContext<R>) -> Result<Scope, JsValue>;

/// One view per route; navigating to an equal route keeps the view.
struct Routes<R: Route>(Box<Render<R>>);

impl<R: Route> Views<Scope> for Routes<R> {
    fn select(&self, url: &AppUrl, prefix: usize) -> Result<Option<Selection<'_, Scope>>, JsValue> {
        let route = R::parse(url);
        // Unknown URLs are distinct destinations, even though each parses as None.
        let unknown = route.is_none().then(|| url.path.clone());
        Ok(Some(Selection {
            identity: Box::new((route, unknown)),
            consumed: prefix,
            render: Box::new(move |owner| {
                let location = super::declarative::Navigation::from_owner(owner)
                    .ok_or_else(|| error("route view has no router"))?
                    .location();
                (self.0)(RouteContext {
                    parent: owner.clone(),
                    location: derived(move || Location::new(location.get())),
                })
            }),
        }))
    }
    fn accepts_link(&self, url: &AppUrl) -> bool {
        R::parse(url).is_some()
    }
}

fn check_empty(container: &Element) -> Result<(), JsValue> {
    if container.child_element_count() != 0
        || container
            .text_content()
            .is_some_and(|text| !text.trim().is_empty())
    {
        return Err(error("router outlet must be empty"));
    }
    Ok(())
}

/// Mount a typed router from Rust. HTML applications normally use Router/Route.
/// Data loading remains in application constructors.
pub fn mount_outlet<R: Route>(
    scope: &mut Scope,
    container: &Element,
    base: &str,
    render: impl Fn(RouteContext<R>) -> Content + 'static,
) -> Result<(), JsValue> {
    check_empty(container)?;
    let target = scope.mount_point(container)?;
    let routes = Routes::<R>(Box::new(move |context| {
        let parent = context.parent.clone();
        render(context).prepare(&parent)
    }));
    Tree::mount_root_in(scope, &target, base, container.clone(), Box::new(routes))
}

/// A typed handle to the router that owns browser history. Clones share one
/// router. A router from [`Router::mount`] lives while a handle does; owner
/// disposal disposes it even if a handle survives.
#[derive(Clone)]
pub struct Router<R: Route> {
    tree: Rc<Tree>,
    route: PhantomData<fn() -> R>,
}

impl<R: Route> Router<R> {
    /// Mount into an empty element. The renderer must return a prepared child of
    /// the supplied owner; using `Component::mount` here is an error.
    pub fn mount(
        parent: &OwnerHandle,
        base: &str,
        container: Element,
        render: impl Fn(RouteContext<R>) -> Result<Scope, JsValue> + 'static,
    ) -> Result<Self, JsValue> {
        check_empty(&container)?;
        let (target, anchors): (MountPoint, _) = MountPoint::append(&container)?;
        let (tree, browser) = Tree::mount_root(
            parent,
            &target,
            base,
            container,
            Box::new(Routes::<R>(Box::new(render))),
        )?;
        tree.keep(anchors);
        browser.finish_mount()?;
        Ok(Self {
            tree,
            route: PhantomData,
        })
    }
    /// Obtain the router that the nearest enclosing router mount provides.
    /// Lookup is explicit and does not subscribe a reactive dependency.
    pub fn from_owner(owner: &OwnerHandle) -> Option<Self> {
        Tree::from_owner(owner).map(|tree| Self {
            tree,
            route: PhantomData,
        })
    }
    pub fn location(&self) -> Location<R> {
        Location::new(self.tree.location().get())
    }
    pub fn last_error(&self) -> Option<JsValue> {
        self.tree.last_error()
    }
    pub fn href(&self, route: &R) -> Result<String, JsValue> {
        Ok(self.tree.base()?.href(route)?)
    }
    pub fn navigate(&self, route: &R, options: NavigateOptions) -> Result<(), JsValue> {
        self.navigate_url(&self.href(route)?, options)
    }
    /// Resolve a browser URL, enforcing same origin and application base. Unknown
    /// routes reach the renderer's `None` branch; external URLs are rejected.
    pub fn navigate_url(&self, href: &str, options: NavigateOptions) -> Result<(), JsValue> {
        self.tree.navigate_url(href, options)
    }
    pub fn dispose(&self) {
        self.tree.dispose();
    }
}
