//! The view tree behind every router: nested boundaries, each showing the
//! view its URL selects, changed only through staged transactions.
mod boundary;
mod views;
pub(super) use boundary::{Boundary, enclosing_view};
pub(super) use views::{Selection, Views};

use super::{
    Flag, INACTIVE, NavigateOptions, PreparedNavigation, REENTRANT, context_error, error,
    history::Browser,
};
use crate::{AppUrl, BasePath};
use boundary::Plan;
use fusor::dom::{MountPoint, Scope};
use fusor::{ContextKey, OwnerHandle, Signal, batch, signal};
use std::{
    any::Any,
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};
use wasm_bindgen::JsValue;
use web_sys::Element;

/// Provided on the owner that mounts a tree.
struct TreeContext;
impl ContextKey for TreeContext {
    type Value = Weak<Tree>;
}

pub(super) struct Tree {
    root: Rc<Boundary>,
    location: Signal<AppUrl>,
    history: RefCell<Option<Rc<Browser>>>,
    disposed: Cell<bool>,
    /// A transaction is staged; only one may exist at a time.
    staged: Cell<bool>,
    activating: Cell<bool>,
    /// Registrations and anchors that live as long as the tree.
    kept: RefCell<Vec<Box<dyn Any>>>,
}

impl Tree {
    /// Mount `views` at `target` for `url`. `parent` provides the tree to
    /// descendants and gates its activation; its disposal disposes the tree.
    pub(super) fn mount(
        parent: &OwnerHandle,
        target: &MountPoint,
        views: Box<dyn Views>,
        url: AppUrl,
    ) -> Result<Rc<Self>, JsValue> {
        let tree = Rc::new(Self {
            root: Boundary::new(parent.clone(), target.clone(), views, 0),
            location: signal(url.clone()),
            history: RefCell::new(None),
            disposed: Cell::new(false),
            staged: Cell::new(false),
            activating: Cell::new(false),
            kept: RefCell::new(Vec::new()),
        });
        parent
            .provide::<TreeContext>(Rc::downgrade(&tree))
            .map_err(context_error)?;
        {
            let _pending = Pending::new(tree.clone());
            tree.root.prepare(&tree, &url)?.apply();
        }
        let weak = Rc::downgrade(&tree);
        tree.keep(parent.on_activate(move || {
            if let Some(tree) = weak.upgrade() {
                tree.activate();
            }
        }));
        let weak = Rc::downgrade(&tree);
        tree.keep(parent.on_cleanup(move || {
            if let Some(tree) = weak.upgrade() {
                tree.dispose();
            }
        }));
        Ok(tree)
    }
    /// Mount `views` as the page's router, owning browser history. Finish the
    /// browser's mount before navigating.
    pub(super) fn mount_root(
        parent: &OwnerHandle,
        target: &MountPoint,
        base: &str,
        presentation: Element,
        views: Box<dyn Views>,
    ) -> Result<(Rc<Self>, Rc<Browser>), JsValue> {
        let browser = Browser::prepare(parent, base, presentation)?;
        let url = browser.location.borrow().clone();
        let tree = Self::mount(parent, target, views, url)?;
        browser.drive(&tree);
        *tree.history.borrow_mut() = Some(browser.clone());
        Ok((tree, browser))
    }
    /// [`Self::mount_root`] in `scope`: the mount finishes when the scope
    /// commits, and dropping the scope disposes the router.
    pub(super) fn mount_root_in(
        scope: &mut Scope,
        target: &MountPoint,
        base: &str,
        presentation: Element,
        views: Box<dyn Views>,
    ) -> Result<(), JsValue> {
        let (tree, browser) = Self::mount_root(&scope.owner(), target, base, presentation, views)?;
        browser.finish_on_commit(scope)?;
        scope.retain(Mounted(tree));
        Ok(())
    }
    pub(super) fn from_owner(owner: &OwnerHandle) -> Option<Rc<Self>> {
        if owner.is_disposed() {
            return None;
        }
        owner
            .context::<TreeContext>()?
            .upgrade()
            .filter(|tree| !tree.disposed.get())
    }
    pub(super) fn keep(&self, value: impl Any) {
        self.kept.borrow_mut().push(Box::new(value));
    }
    pub(super) fn location(&self) -> Signal<AppUrl> {
        self.location.clone()
    }
    pub(super) fn base(&self) -> Result<BasePath, JsValue> {
        let history = self.history.borrow();
        let browser = history
            .as_ref()
            .ok_or_else(|| error("router does not own browser history"))?;
        Ok(browser.base.clone())
    }
    pub(super) fn last_error(&self) -> Option<JsValue> {
        self.history.borrow().as_ref()?.error.get()
    }
    pub(super) fn accepts_link(&self, url: &AppUrl) -> bool {
        self.root.views.accepts_link(url)
    }
    /// Navigate through browser history when the tree owns it, else directly.
    pub(super) fn navigate_url(
        self: &Rc<Self>,
        url: &str,
        options: NavigateOptions,
    ) -> Result<(), JsValue> {
        self.idle()?;
        let browser = self.history.borrow().clone();
        match browser {
            Some(browser) => browser.navigate(url, options),
            None => self.navigate(AppUrl::parse(url)?),
        }
    }
    pub(super) fn navigate(self: &Rc<Self>, url: AppUrl) -> Result<(), JsValue> {
        self.prepare_navigation(&url)?.commit();
        Ok(())
    }
    fn idle(&self) -> Result<(), JsValue> {
        if self.disposed.get() {
            return Err(error(INACTIVE));
        }
        if self.staged.get() || self.activating.get() {
            return Err(error(REENTRANT));
        }
        Ok(())
    }
    /// Stage the views `url` shows. Dropping the result rolls back.
    pub(super) fn prepare_navigation(
        self: &Rc<Self>,
        url: &AppUrl,
    ) -> Result<Box<dyn PreparedNavigation>, JsValue> {
        self.idle()?;
        let pending = Pending::new(self.clone());
        Ok(Box::new(Transaction {
            plan: self.root.prepare(self, url)?,
            url: url.clone(),
            _pending: pending,
        }))
    }
    fn activate(&self) {
        let Ok(_activating) = Flag::take(&self.activating, REENTRANT) else {
            return;
        };
        self.root.activate();
    }
    pub(super) fn dispose(&self) {
        if self.disposed.replace(true) {
            return;
        }
        self.root.clear();
        if let Some(browser) = self.history.take() {
            browser.dispose();
        }
        drop(self.kept.take());
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        self.dispose();
    }
}

/// Disposes a tree when the scope that mounted it drops, even while other
/// handles to the tree survive.
pub(super) struct Mounted(pub(super) Rc<Tree>);
impl Drop for Mounted {
    fn drop(&mut self) {
        self.0.dispose();
    }
}

/// Marks the tree's one staged transaction. A failed or abandoned preparation
/// drops it, leaving the current views and URL unchanged.
struct Pending(Rc<Tree>);
impl Pending {
    fn new(tree: Rc<Tree>) -> Self {
        tree.staged.set(true);
        Self(tree)
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.0.staged.set(false);
    }
}

struct Transaction {
    plan: Plan,
    url: AppUrl,
    _pending: Pending,
}
impl PreparedNavigation for Transaction {
    fn commit(self: Box<Self>) {
        let Self {
            plan,
            url,
            _pending: pending,
        } = *self;
        let tree = &pending.0;
        if tree.disposed.get() {
            return;
        }
        batch(|| {
            plan.apply();
            tree.location.set(url);
            tree.activate();
        });
    }
}
