//! A place in the page that shows one view at a time, and the staged plans
//! that replace its view.
use super::{
    Tree,
    views::{Identity, Selection, Views},
};
use crate::{
    AppUrl,
    browser::{context_error, error},
};
use fusor::dom::{MountPoint, Scope};
use fusor::{ContextKey, Owner, OwnerHandle, untrack};
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};
use wasm_bindgen::JsValue;

type Children = Rc<RefCell<Vec<Weak<Boundary>>>>;

/// Provided on each view's owner: where boundaries nested in the view attach.
struct BranchContext;
impl ContextKey for BranchContext {
    type Value = Branch;
}
struct Branch {
    tree: Weak<Tree>,
    prefix: usize,
    children: Children,
    /// The URL the view was prepared for, reported until the view activates.
    initial: AppUrl,
    owner: OwnerHandle,
}

/// The router view that `owner` belongs to: that view's owner and the URL it
/// was prepared for.
pub(in crate::browser) fn enclosing_view(owner: &OwnerHandle) -> Option<(OwnerHandle, AppUrl)> {
    owner
        .context::<BranchContext>()
        .map(|branch| (branch.owner.clone(), branch.initial.clone()))
}

pub(in crate::browser) struct Boundary {
    parent: OwnerHandle,
    target: MountPoint,
    pub(super) views: Box<dyn Views>,
    prefix: usize,
    current: RefCell<Option<Rc<View>>>,
}

pub(in crate::browser) struct View {
    identity: Box<dyn Identity>,
    scope: Scope,
    owner: Owner,
    children: Children,
}

impl View {
    fn children(&self) -> Vec<Rc<Boundary>> {
        self.children
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|child| !child.parent.is_disposed())
            .collect()
    }
}

/// A staged change to a boundary and the boundaries nested in its view.
pub(in crate::browser) enum Plan {
    Keep(Vec<Plan>),
    Replace(Rc<Boundary>, Option<Rc<View>>),
}

impl Plan {
    pub(super) fn apply(self) {
        match self {
            Self::Keep(children) => {
                for child in children {
                    child.apply();
                }
            }
            Self::Replace(boundary, view) => drop(boundary.current.replace(view)),
        }
    }
}

impl Boundary {
    pub(super) fn new(
        parent: OwnerHandle,
        target: MountPoint,
        views: Box<dyn Views>,
        prefix: usize,
    ) -> Rc<Self> {
        Rc::new(Self {
            parent,
            target,
            views,
            prefix,
            current: RefCell::new(None),
        })
    }
    /// Mount a boundary for `views` inside the router view that owns `scope`,
    /// matching after that view's prefix.
    pub(in crate::browser) fn nest(
        scope: &mut Scope,
        target: &MountPoint,
        views: Box<dyn Views>,
    ) -> Result<(), JsValue> {
        let branch = scope
            .owner()
            .context::<BranchContext>()
            .ok_or_else(|| error("nested routes need an enclosing router view"))?;
        let tree = branch
            .tree
            .upgrade()
            .ok_or_else(|| error("parent router is disposed"))?;
        let boundary = Self::new(scope.owner(), target.clone(), views, branch.prefix);
        let url = if branch.owner.is_active() {
            tree.location.get_untracked()
        } else {
            branch.initial.clone()
        };
        boundary.prepare(&tree, &url)?.apply();
        let mut children = branch.children.borrow_mut();
        children.retain(|child| child.strong_count() != 0);
        children.push(Rc::downgrade(&boundary));
        drop(children);
        let weak = Rc::downgrade(&boundary);
        scope.retain(scope.owner().on_activate(move || {
            if let Some(boundary) = weak.upgrade() {
                boundary.activate();
            }
        }));
        scope.retain(boundary);
        Ok(())
    }
    /// Stage what `url` shows here: keep an unchanged view and plan for its
    /// nested boundaries, or build a replacement.
    pub(super) fn prepare(self: &Rc<Self>, tree: &Rc<Tree>, url: &AppUrl) -> Result<Plan, JsValue> {
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
        tree: &Rc<Tree>,
        url: &AppUrl,
        selection: Selection<'_>,
    ) -> Result<Rc<View>, JsValue> {
        let owner = Owner::child(&self.parent);
        let children = Children::default();
        owner
            .handle()
            .provide::<BranchContext>(Branch {
                tree: Rc::downgrade(tree),
                prefix: selection.consumed,
                children: children.clone(),
                initial: url.clone(),
                owner: owner.handle(),
            })
            .map_err(context_error)?;
        let mut scope = untrack(|| (selection.render)(&owner.handle()))?;
        let child = scope.owner();
        if child.is_active()
            || child.is_disposed()
            || !child.is_child_of(&owner.handle())
            || scope.root().is_connected()
        {
            return Err(error(
                "route views must return a detached, prepared child of their supplied owner",
            ));
        }
        scope.attach_at(&self.target)?;
        scope.finish_prepare()?;
        if self.parent.is_active() {
            scope.finish_prepare_subtree()?;
        }
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
