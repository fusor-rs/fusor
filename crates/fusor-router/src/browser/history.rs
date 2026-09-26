//! Browser history as a router's URL source: navigation, back and forward,
//! and link clicks become staged tree transactions.
mod links;
mod present;
mod state;

use super::{Flag, INACTIVE, NavigateOptions, PreparedNavigation, REENTRANT, error, tree::Tree};
use crate::{AppUrl, BasePath};
use fusor::dom::{Listener, Scope, document};
use fusor::{Owner, OwnerHandle, Registration, Signal, batch, signal};
use state::{Entries, Lease};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};
use wasm_bindgen::JsValue;
use web_sys::{Element, Event, EventTarget, History, Url, Window};

pub(super) struct Browser {
    owner: Owner,
    lease: Lease,
    window: Window,
    history: History,
    pub(super) base: BasePath,
    origin: String,
    entries: Entries,
    /// After a failed traversal, the index the router is returning to.
    recovering: Cell<Option<i32>>,
    busy: Cell<bool>,
    disposed: Cell<bool>,
    presentation: Element,
    tree: RefCell<Weak<Tree>>,
    /// The URL the router shows; the page's URL until the first navigation.
    pub(super) location: RefCell<AppUrl>,
    pub(super) error: Signal<Option<JsValue>>,
    listeners: RefCell<Vec<Listener>>,
    activation: RefCell<Option<Registration>>,
    /// The page's history state before mounting, restored if nothing activated.
    original_state: RefCell<Option<JsValue>>,
}

fn relative(base: &BasePath, url: &Url) -> Result<AppUrl, JsValue> {
    Ok(base.strip(&format!("{}{}{}", url.pathname(), url.search(), url.hash()))?)
}

impl Browser {
    pub(super) fn prepare(
        parent: &OwnerHandle,
        base: &str,
        presentation: Element,
    ) -> Result<Rc<Self>, JsValue> {
        if parent.is_disposed() {
            return Err(error("router parent is disposed"));
        }
        let lease = Lease::acquire()?;
        let base = BasePath::new(base)?;
        let window = web_sys::window().ok_or_else(|| error("router requires a browser"))?;
        let history = window.history()?;
        let url = Url::new(&window.location().href()?)?;
        let location = relative(&base, &url)?;
        Ok(Rc::new(Self {
            owner: Owner::child(parent),
            lease,
            window,
            history,
            base,
            origin: url.origin(),
            entries: Entries::new(),
            recovering: Cell::new(None),
            busy: Cell::new(false),
            disposed: Cell::new(false),
            presentation,
            tree: RefCell::new(Weak::new()),
            location: RefCell::new(location),
            error: signal(None),
            listeners: RefCell::new(Vec::new()),
            activation: RefCell::new(None),
            original_state: RefCell::new(None),
        }))
    }
    pub(super) fn drive(&self, tree: &Rc<Tree>) {
        *self.tree.borrow_mut() = Rc::downgrade(tree);
    }
    /// Finish mounting when `scope` commits, after its whole subtree prepared.
    pub(super) fn finish_on_commit(self: &Rc<Self>, scope: &mut Scope) -> Result<(), JsValue> {
        let weak = Rc::downgrade(self);
        scope.before_commit(move || {
            if let Some(browser) = weak.upgrade().filter(|browser| !browser.disposed.get()) {
                browser.finish_mount()?;
            }
            Ok(())
        })
    }
    /// Start following the browser: listen for navigation and mark the
    /// current entry as this router's.
    pub(super) fn finish_mount(self: &Rc<Self>) -> Result<(), JsValue> {
        if self.disposed.get() {
            return Err(error(INACTIVE));
        }
        self.listen(self.window.clone().into(), "popstate", |browser, _| {
            browser.pop()
        })?;
        self.listen(self.window.clone().into(), "hashchange", |browser, _| {
            browser.pop()
        })?;
        self.listen(document()?.into(), "click", Self::clicked)?;
        let original = self.history.state()?;
        self.history
            .replace_state_with_url(&self.entries.state(&self.history, 0)?, "", None)?;
        *self.original_state.borrow_mut() = Some(original);
        let weak = Rc::downgrade(self);
        *self.activation.borrow_mut() = Some(self.owner.handle().on_activate(move || {
            if let Some(browser) = weak.upgrade() {
                browser.original_state.borrow_mut().take();
            }
        }));
        self.owner.commit();
        Ok(())
    }
    fn listen(
        self: &Rc<Self>,
        target: EventTarget,
        name: &str,
        handle: fn(&Self, Event) -> Result<(), JsValue>,
    ) -> Result<(), JsValue> {
        let weak = Rc::downgrade(self);
        let listener = Listener::new(target, name, move |event| {
            let Some(browser) = weak
                .upgrade()
                .filter(|browser| browser.owner.handle().is_active())
            else {
                return;
            };
            if let Err(error) = handle(&browser, event) {
                browser.report(error);
            }
        })?;
        self.listeners.borrow_mut().push(listener);
        Ok(())
    }
    fn report(&self, error: JsValue) {
        web_sys::console::error_1(&error);
        self.error.set(Some(error));
    }
    fn tree(&self) -> Result<Rc<Tree>, JsValue> {
        self.tree.borrow().upgrade().ok_or_else(|| error(INACTIVE))
    }
    fn enter(&self) -> Result<Flag<'_>, JsValue> {
        if self.disposed.get() || !self.owner.handle().is_active() {
            return Err(error(INACTIVE));
        }
        if self.recovering.get().is_some() {
            return Err(error(
                "router is restoring browser history after a failed navigation",
            ));
        }
        Flag::take(&self.busy, REENTRANT)
    }
    /// Navigate to `href`, recording a failure as the router's last error.
    pub(super) fn navigate(&self, href: &str, options: NavigateOptions) -> Result<(), JsValue> {
        let result = self.try_navigate(href, options);
        if let Err(error) = &result {
            self.report(error.clone());
        }
        result
    }
    fn try_navigate(&self, href: &str, options: NavigateOptions) -> Result<(), JsValue> {
        let _busy = self.enter()?;
        let url = Url::new_with_base(href, &self.window.location().href()?)?;
        if url.origin() != self.origin || !url.username().is_empty() || !url.password().is_empty() {
            return Err(error("navigation must stay on the application origin"));
        }
        let next = relative(&self.base, &url)?;
        if *self.location.borrow() == next {
            return Ok(());
        }
        let index = if options.replace {
            self.entries.index.get()
        } else {
            self.entries
                .index
                .get()
                .checked_add(1)
                .ok_or_else(|| error("history index overflow"))?
        };
        // Preparation and append may fail. Neither changes history or disposes old state.
        let transaction = self.tree()?.prepare_navigation(&next)?;
        let state = self.entries.state(&self.history, index)?;
        if options.replace {
            self.history
                .replace_state_with_url(&state, "", Some(&url.href()))?;
        } else {
            self.history
                .push_state_with_url(&state, "", Some(&url.href()))?;
        }
        self.entries.index.set(index);
        self.commit(next.clone(), transaction);
        self.present(&next, options, false);
        Ok(())
    }
    fn commit(&self, next: AppUrl, transaction: Box<dyn PreparedNavigation>) {
        batch(|| {
            *self.location.borrow_mut() = next;
            self.error.set(None);
            transaction.commit();
        });
    }
    fn present(&self, url: &AppUrl, options: NavigateOptions, traversal: bool) {
        if self.disposed.get() {
            return;
        }
        if !options.keep_focus {
            present::focus_content(&self.presentation);
        }
        // Back and forward keep the browser's own scroll restoration.
        if !options.keep_scroll && !traversal {
            present::scroll_to(&self.window, url);
        }
    }
    /// Follow back, forward or a fragment change to the browser's current URL.
    fn pop(&self) -> Result<(), JsValue> {
        if self.disposed.get() {
            return Ok(());
        }
        let url = Url::new(&self.window.location().href()?)?;
        let Ok(next) = relative(&self.base, &url) else {
            return self.window.location().reload();
        };
        let index = self.entries.current(&self.history);
        if let Some(expected) = self.recovering.get() {
            if index == Some(expected) && *self.location.borrow() == next {
                self.recovering.set(None);
                return Ok(());
            }
            // A second traversal overtook restoration. Reload the actual URL;
            // never publish a view for a different address-bar location.
            return self.window.location().reload();
        }
        let current = self.location.borrow().clone();
        if current == next {
            return Ok(());
        }
        if current.path == next.path && current.query == next.query {
            return self.follow_fragment(next);
        }
        match index {
            Some(index) => self.traverse(next, index),
            // An entry this router did not make: load it as a new document.
            None => self.window.location().reload(),
        }
    }
    fn follow_fragment(&self, next: AppUrl) -> Result<(), JsValue> {
        let _busy = self.enter()?;
        // Native fragment entries need not carry our state or a unique index.
        // Start a new known segment rather than inventing a traversal delta.
        // Traversing beyond it can fall back to a document navigation.
        self.entries.restart()?;
        self.history
            .replace_state_with_url(&self.entries.state(&self.history, 0)?, "", None)?;
        let transaction = self.tree()?.prepare_navigation(&next)?;
        self.commit(next, transaction);
        Ok(())
    }
    /// Show the entry at `index`. On failure, return the browser to the
    /// entry the router still shows, or reload if that is impossible.
    fn traverse(&self, next: AppUrl, index: i32) -> Result<(), JsValue> {
        let _busy = self.enter()?;
        match self.tree()?.prepare_navigation(&next) {
            Ok(transaction) => {
                self.commit(next.clone(), transaction);
                self.entries.index.set(index);
                self.present(&next, NavigateOptions::default(), true);
                Ok(())
            }
            Err(failure) => {
                let shown = self.entries.index.get();
                if index != shown {
                    self.recovering.set(Some(shown));
                    self.history.go_with_delta(shown - index)?;
                } else {
                    self.window.location().reload()?;
                }
                Err(failure)
            }
        }
    }
    fn clicked(&self, event: Event) -> Result<(), JsValue> {
        let Some((anchor, url)) = links::router_link(&event, &self.origin)? else {
            return Ok(());
        };
        let Ok(next) = relative(&self.base, &url) else {
            return Ok(());
        };
        if !self.tree()?.accepts_link(&next) {
            return Ok(());
        }
        // The browser scrolls to fragments on the current page by itself.
        let fragment = anchor
            .get_attribute("href")
            .is_some_and(|href| href.starts_with('#'))
            || {
                let current = self.location.borrow();
                current.path == next.path
                    && current.query == next.query
                    && !next.fragment.is_empty()
            };
        if fragment {
            return Ok(());
        }
        event.prevent_default();
        self.try_navigate(&url.href(), NavigateOptions::default())
    }
    pub(super) fn dispose(&self) {
        if self.disposed.replace(true) {
            return;
        }
        self.owner.dispose();
        drop(self.listeners.take());
        if let Some(original) = self.original_state.take() {
            if self.entries.current(&self.history) == Some(0) {
                if let Err(error) = self.history.replace_state_with_url(&original, "", None) {
                    web_sys::console::error_1(&error);
                }
            }
        }
        self.lease.release();
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.dispose();
    }
}
