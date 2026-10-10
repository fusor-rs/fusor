//! Renderer-independent route selection, nested retention and staged navigation.
//!
//! A renderer implements [`RouteScope`] and retains a [`ViewRouter`] for each
//! mounted outlet. Navigation sources prepare a transition before publishing
//! their own location, then commit it synchronously. Dropping a prepared
//! transition rolls back its candidate views.
mod boundary;
mod tree;
mod views;
pub(crate) use boundary::{Boundary, enclosing_view};
pub(crate) use tree::Tree;
pub(crate) use views::{Selection, Views};

use crate::{
    AppUrl,
    pattern::{Match, Pattern, ambiguous},
    select::select,
};
use fusor::{Derived, OwnerHandle, derived};
use std::rc::Rc;

pub(crate) const INACTIVE: &str = "router is not active";
pub(crate) const REENTRANT: &str = "reentrant navigation is not supported";

/// Version of the portable routing integration contract.
pub const VERSION: u32 = 1;

/// The renderer-specific part of preparing a route view.
/// Dropping a scope must remove its attached nodes and dispose its owner.
pub trait RouteScope: fusor::render::Scope + Sized + 'static {
    type Target: Clone + 'static;
    type Error: 'static;

    fn error(message: &str) -> Self::Error;

    /// Validate that the view is detached, attach it without activation, and
    /// finish fallible renderer setup. When `parent_active` is true, finish the
    /// complete candidate subtree before returning. A failure or later drop
    /// must undo attachment without disturbing the current view.
    ///
    /// Preparation may temporarily attach inactive nodes beside the current
    /// view. Finish or abandon the navigation synchronously before presenting.
    fn prepare_at(&mut self, target: &Self::Target, parent_active: bool)
    -> Result<(), Self::Error>;

    /// Activate the prepared scope after its route owner has activated.
    fn commit(&self);
}

/// Prepared view work. Dropping it preserves the current view and location.
pub trait PreparedNavigation {
    fn commit(self: Box<Self>);
}

type Factory<S> = dyn Fn(&OwnerHandle, &Match) -> Result<S, <S as RouteScope>::Error>;

/// A lazy route factory. Unmatched branches are never constructed.
pub struct RouteView<S: RouteScope> {
    pattern: Option<Pattern>,
    render: Box<Factory<S>>,
}

impl<S: RouteScope> RouteView<S> {
    pub fn new(
        pattern: &str,
        render: impl Fn(&OwnerHandle, &Match) -> Result<S, S::Error> + 'static,
    ) -> Result<Self, S::Error> {
        Ok(Self {
            pattern: Some(Pattern::new(pattern).map_err(|error| S::error(error.0))?),
            render: Box::new(render),
        })
    }

    pub fn fallback(
        render: impl Fn(&OwnerHandle, &Match) -> Result<S, S::Error> + 'static,
    ) -> Self {
        Self {
            pattern: None,
            render: Box::new(render),
        }
    }
}

pub(crate) struct Routes<S: RouteScope>(Vec<RouteView<S>>);

impl<S: RouteScope> Routes<S> {
    pub(crate) fn new(routes: Vec<RouteView<S>>) -> Result<Self, S::Error> {
        for (index, route) in routes.iter().enumerate() {
            if routes[..index]
                .iter()
                .any(|other| ambiguous(route.pattern.as_ref(), other.pattern.as_ref()))
            {
                return Err(S::error(
                    "Router contains ambiguous routes or multiple fallbacks",
                ));
            }
        }
        Ok(Self(routes))
    }
}

impl<S: RouteScope> Views<S> for Routes<S> {
    fn select(&self, url: &AppUrl, prefix: usize) -> Result<Option<Selection<'_, S>>, S::Error> {
        let patterns = self.0.iter().map(|route| route.pattern.as_ref());
        Ok(select(patterns, url, prefix)
            .map_err(|error| S::error(error.0))?
            .map(|selected| {
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

/// Shared navigation, independent of a platform's history or input mechanism.
pub struct Navigation<S: RouteScope> {
    tree: Rc<Tree<S>>,
    prepared: Option<(OwnerHandle, AppUrl)>,
}

impl<S: RouteScope> Clone for Navigation<S> {
    fn clone(&self) -> Self {
        Self {
            tree: self.tree.clone(),
            prepared: self.prepared.clone(),
        }
    }
}

impl<S: RouteScope> Navigation<S> {
    pub(crate) fn new(tree: Rc<Tree<S>>, prepared: Option<(OwnerHandle, AppUrl)>) -> Self {
        Self { tree, prepared }
    }

    pub fn from_owner(owner: &OwnerHandle) -> Option<Self> {
        Some(Self {
            tree: Tree::from_owner(owner)?,
            prepared: enclosing_view::<S>(owner),
        })
    }

    /// A prepared view reads its destination until activation; retained active
    /// views observe the published location.
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

    pub fn navigate(&self, url: AppUrl) -> Result<(), S::Error> {
        self.prepare_navigation(&url)?.commit();
        Ok(())
    }

    /// Only one stage may exist per tree. Errors and abandoned stages leave
    /// active views and the published location unchanged.
    pub fn prepare_navigation(
        &self,
        url: &AppUrl,
    ) -> Result<Box<dyn PreparedNavigation>, S::Error> {
        self.tree.prepare_navigation(url)
    }
}

/// Lifetime token for an outlet. Keep a clone in the containing render scope.
/// Dropping its last clone disposes the outlet even if navigation handles survive.
pub struct ViewRouter<S: RouteScope>(Rc<Mount<S>>);

struct Mount<S: RouteScope> {
    navigation: Navigation<S>,
    boundary: Option<Rc<Boundary<S>>>,
}

impl<S: RouteScope> Drop for Mount<S> {
    fn drop(&mut self) {
        if let Some(boundary) = &self.boundary {
            boundary.clear();
        } else {
            self.navigation.tree.dispose();
        }
    }
}

impl<S: RouteScope> Clone for ViewRouter<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S: RouteScope> ViewRouter<S> {
    /// Mount nested routes after the enclosing view's consumed path, or create
    /// a root tree at `url`. Nested outlets inherit the enclosing destination.
    pub fn mount(
        parent: &OwnerHandle,
        target: &S::Target,
        routes: Vec<RouteView<S>>,
        url: AppUrl,
    ) -> Result<Self, S::Error> {
        let routes = Box::new(Routes::new(routes)?);
        let prepared = enclosing_view::<S>(parent);
        let (tree, boundary) = if prepared.is_some() {
            let tree =
                Tree::from_owner(parent).ok_or_else(|| S::error("parent router is disposed"))?;
            (tree, Some(Boundary::nest(parent, target, routes)?))
        } else {
            (Tree::mount(parent, target, routes, url)?, None)
        };
        Ok(Self(Rc::new(Mount {
            navigation: Navigation::new(tree, prepared),
            boundary,
        })))
    }

    pub fn navigation(&self) -> Navigation<S> {
        self.0.navigation.clone()
    }
}
