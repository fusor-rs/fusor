//! Staged view replacement and nested route retention.
use super::{
    RouteScope, Tree,
    views::{Identity, Selection, Views},
};
use crate::AppUrl;
use fusor::{ContextKey, Owner, OwnerHandle, untrack};
use std::{
    cell::RefCell,
    marker::PhantomData,
    rc::{Rc, Weak},
};

type Children<S> = Rc<RefCell<Vec<Weak<Boundary<S>>>>>;

/// Provided on each view's owner: where boundaries nested in the view attach.
struct BranchContext<S>(PhantomData<S>);
impl<S: RouteScope> ContextKey for BranchContext<S> {
    type Value = Branch<S>;
}
struct Branch<S: RouteScope> {
    tree: Weak<Tree<S>>,
    prefix: usize,
    children: Children<S>,
    /// The URL the view was prepared for, reported until the view activates.
    initial: AppUrl,
    owner: OwnerHandle,
}

/// The router view that `owner` belongs to: that view's owner and the URL it
/// was prepared for.
pub(crate) fn enclosing_view<S: RouteScope>(owner: &OwnerHandle) -> Option<(OwnerHandle, AppUrl)> {
    owner
        .context::<BranchContext<S>>()
        .map(|branch| (branch.owner.clone(), branch.initial.clone()))
}

pub(crate) struct Boundary<S: RouteScope> {
    parent: OwnerHandle,
    target: S::Target,
    pub(super) views: Box<dyn Views<S>>,
    prefix: usize,
    current: RefCell<Option<Rc<View<S>>>>,
    registrations: RefCell<Vec<fusor::Registration>>,
}

pub(crate) struct View<S: RouteScope> {
    identity: Box<dyn Identity>,
    scope: S,
    owner: Owner,
    children: Children<S>,
}

impl<S: RouteScope> View<S> {
    fn children(&self) -> Vec<Rc<Boundary<S>>> {
        self.children
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|child| !child.parent.is_disposed())
            .collect()
    }
}

/// A staged change to a boundary and the boundaries nested in its view.
pub(crate) enum Plan<S: RouteScope> {
    Keep(Vec<Plan<S>>),
    Replace(Rc<Boundary<S>>, Option<Rc<View<S>>>),
}

impl<S: RouteScope> Plan<S> {
    pub(super) fn apply(self) {
        match self {
            Self::Keep(children) => {
                for child in children {
                    child.apply();
                }
            }
            Self::Replace(boundary, view) => {
                if !boundary.parent.is_disposed() {
                    drop(boundary.current.replace(view));
                }
            }
        }
    }
}

impl<S: RouteScope> Boundary<S> {
    pub(super) fn new(
        parent: OwnerHandle,
        target: S::Target,
        views: Box<dyn Views<S>>,
        prefix: usize,
    ) -> Rc<Self> {
        Rc::new(Self {
            parent,
            target,
            views,
            prefix,
            current: RefCell::new(None),
            registrations: RefCell::new(Vec::new()),
        })
    }
    /// Mount a boundary for `views` inside the router view that owns `scope`,
    /// matching after that view's prefix.
    pub(crate) fn nest(
        parent: &OwnerHandle,
        target: &S::Target,
        views: Box<dyn Views<S>>,
    ) -> Result<Rc<Self>, S::Error> {
        let branch = parent
            .context::<BranchContext<S>>()
            .ok_or_else(|| S::error("nested routes need an enclosing router view"))?;
        let tree = branch
            .tree
            .upgrade()
            .ok_or_else(|| S::error("parent router is disposed"))?;
        let boundary = Self::new(parent.clone(), target.clone(), views, branch.prefix);
        let url = if branch.owner.is_active() {
            tree.location().get_untracked()
        } else {
            branch.initial.clone()
        };
        boundary.prepare(&tree, &url)?.apply();
        let mut children = branch.children.borrow_mut();
        children.retain(|child| child.strong_count() != 0);
        children.push(Rc::downgrade(&boundary));
        drop(children);
        let weak = Rc::downgrade(&boundary);
        let activation = parent.on_activate(move || {
            if let Some(boundary) = weak.upgrade() {
                boundary.activate();
            }
        });
        let weak = Rc::downgrade(&boundary);
        let cleanup = parent.on_cleanup(move || {
            if let Some(boundary) = weak.upgrade() {
                boundary.clear();
            }
        });
        boundary
            .registrations
            .borrow_mut()
            .extend([activation, cleanup]);
        Ok(boundary)
    }
    /// Stage what `url` shows here: keep an unchanged view and plan for its
    /// nested boundaries, or build a replacement.
    pub(super) fn prepare(
        self: &Rc<Self>,
        tree: &Rc<Tree<S>>,
        url: &AppUrl,
    ) -> Result<Plan<S>, S::Error> {
        let selection = self.views.select(url, self.prefix)?;
        let current = self.current.borrow().clone();
        if let (Some(view), Some(selection)) = (&current, &selection) {
            if view.identity.same(&*selection.identity) {
                return view
                    .children()
                    .iter()
                    .map(|child| child.prepare(tree, url))
                    .collect::<Result<_, _>>()
                    .map(Plan::Keep);
            }
        }
        let view = selection
            .map(|selection| self.build(tree, url, selection))
            .transpose()?;
        Ok(Plan::Replace(self.clone(), view))
    }
    /// Prepare the selected view, attached but inactive.
    fn build(
        &self,
        tree: &Rc<Tree<S>>,
        url: &AppUrl,
        selection: Selection<'_, S>,
    ) -> Result<Rc<View<S>>, S::Error> {
        let owner = Owner::child(&self.parent);
        let children = Children::<S>::default();
        owner
            .handle()
            .provide::<BranchContext<S>>(Branch {
                tree: Rc::downgrade(tree),
                prefix: selection.consumed,
                children: children.clone(),
                initial: url.clone(),
                owner: owner.handle(),
            })
            .map_err(|error| S::error(&error.to_string()))?;
        let mut scope = untrack(|| (selection.render)(&owner.handle()))?;
        let child = scope.owner();
        if child.is_active() || child.is_disposed() || !child.is_child_of(&owner.handle()) {
            return Err(S::error(
                "route views must return a detached, prepared child of their supplied owner",
            ));
        }
        scope.prepare_at(&self.target, self.parent.is_active())?;
        Ok(Rc::new(View {
            identity: selection.identity,
            scope,
            owner,
            children,
        }))
    }
    pub(super) fn activate(&self) {
        let Some(view) = self.current.borrow().clone() else {
            return;
        };
        view.owner.commit();
        view.scope.commit();
        // Activation callbacks can replace or dispose this view and its children.
        let children = view.children();
        drop(view);
        for child in children {
            if !child.parent.is_disposed() {
                child.activate();
            }
        }
    }
    pub(super) fn clear(&self) {
        drop(self.current.take());
    }
}
