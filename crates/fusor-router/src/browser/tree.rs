//! Browser history and DOM preparation around the shared route tree.
use super::{NavigateOptions, PreparedNavigation, context_error, error, history::Browser};
use crate::{AppUrl, BasePath, view};
use fusor::dom::{MountPoint, Scope};
use fusor::{ContextKey, OwnerHandle, Signal};
use std::{
    any::Any,
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};
pub(super) use view::{Selection, Views};
use wasm_bindgen::JsValue;
use web_sys::Element;

impl view::RouteScope for Scope {
    type Target = MountPoint;
    type Error = JsValue;

    fn error(message: &str) -> JsValue {
        error(message)
    }

    fn prepare_at(&mut self, target: &MountPoint, parent_active: bool) -> Result<(), JsValue> {
        if !self.is_detached() {
            return Err(error(
                "route views must return a detached, prepared child of their supplied owner",
            ));
        }
        self.attach_at(target)?;
        self.finish_prepare()?;
        if parent_active {
            self.finish_prepare_subtree()?;
        }
        Ok(())
    }

    fn commit(&self) {
        Scope::commit(self);
    }
}

struct TreeContext;
impl ContextKey for TreeContext {
    type Value = Weak<Tree>;
}

pub(super) struct Tree {
    inner: Rc<view::Tree<Scope>>,
    history: RefCell<Option<Rc<Browser>>>,
    disposed: Cell<bool>,
    kept: RefCell<Vec<Box<dyn Any>>>,
}

impl Tree {
    pub(super) fn mount(
        parent: &OwnerHandle,
        target: &MountPoint,
        views: Box<dyn Views<Scope>>,
        url: AppUrl,
    ) -> Result<Rc<Self>, JsValue> {
        let (_, tree) = view::Tree::mount_with(parent, target, views, url, |inner| {
            let tree = Rc::new(Self {
                inner: inner.clone(),
                history: RefCell::new(None),
                disposed: Cell::new(false),
                kept: RefCell::new(Vec::new()),
            });
            parent
                .provide::<TreeContext>(Rc::downgrade(&tree))
                .map_err(context_error)?;
            let weak = Rc::downgrade(&tree);
            tree.keep(parent.on_cleanup(move || {
                if let Some(tree) = weak.upgrade() {
                    tree.dispose();
                }
            }));
            Ok(tree)
        })?;
        Ok(tree)
    }

    pub(super) fn mount_root(
        parent: &OwnerHandle,
        target: &MountPoint,
        base: &str,
        presentation: Element,
        views: Box<dyn Views<Scope>>,
    ) -> Result<(Rc<Self>, Rc<Browser>), JsValue> {
        let browser = Browser::prepare(parent, base, presentation)?;
        let url = browser.location.borrow().clone();
        let tree = Self::mount(parent, target, views, url)?;
        browser.drive(&tree);
        *tree.history.borrow_mut() = Some(browser.clone());
        Ok((tree, browser))
    }

    pub(super) fn mount_root_in(
        scope: &mut Scope,
        target: &MountPoint,
        base: &str,
        presentation: Element,
        views: Box<dyn Views<Scope>>,
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
    pub(super) fn navigation(&self) -> view::Navigation<Scope> {
        view::Navigation::new(self.inner.clone(), None)
    }
    pub(super) fn location(&self) -> Signal<AppUrl> {
        self.inner.location()
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
        self.inner.accepts_link(url)
    }
    pub(super) fn navigate_url(
        self: &Rc<Self>,
        url: &str,
        options: NavigateOptions,
    ) -> Result<(), JsValue> {
        self.inner.idle()?;
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
    pub(super) fn prepare_navigation(
        self: &Rc<Self>,
        url: &AppUrl,
    ) -> Result<Box<dyn PreparedNavigation>, JsValue> {
        self.inner.prepare_navigation(url)
    }
    pub(super) fn dispose(&self) {
        if self.disposed.replace(true) {
            return;
        }
        self.inner.dispose();
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

pub(super) struct Mounted(pub(super) Rc<Tree>);
impl Drop for Mounted {
    fn drop(&mut self) {
        self.0.dispose();
    }
}
