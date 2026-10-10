//! Shared route transactions; platform history publishes only after preparation.
use super::{Boundary, INACTIVE, PreparedNavigation, REENTRANT, RouteScope, Views, boundary::Plan};
use crate::AppUrl;
use fusor::{ContextKey, OwnerHandle, Registration, Signal, batch, signal};
use std::{
    cell::{Cell, RefCell},
    marker::PhantomData,
    rc::{Rc, Weak},
};

struct TreeContext<S>(PhantomData<S>);
impl<S: RouteScope> ContextKey for TreeContext<S> {
    type Value = Weak<Tree<S>>;
}

pub(crate) struct Tree<S: RouteScope> {
    root: Rc<Boundary<S>>,
    location: Signal<AppUrl>,
    disposed: Cell<bool>,
    staged: Cell<bool>,
    activating: Cell<bool>,
    kept: RefCell<Vec<Registration>>,
}

impl<S: RouteScope> Tree<S> {
    pub(crate) fn mount(
        parent: &OwnerHandle,
        target: &S::Target,
        views: Box<dyn Views<S>>,
        url: AppUrl,
    ) -> Result<Rc<Self>, S::Error> {
        Self::mount_with(parent, target, views, url, |_| Ok(())).map(|(tree, ())| tree)
    }

    pub(crate) fn mount_with<T>(
        parent: &OwnerHandle,
        target: &S::Target,
        views: Box<dyn Views<S>>,
        url: AppUrl,
        setup: impl FnOnce(&Rc<Self>) -> Result<T, S::Error>,
    ) -> Result<(Rc<Self>, T), S::Error> {
        if parent.is_disposed() {
            return Err(S::error("router parent is disposed"));
        }
        let tree = Rc::new(Self {
            root: Boundary::new(parent.clone(), target.clone(), views, 0),
            location: signal(url.clone()),
            disposed: Cell::new(false),
            staged: Cell::new(false),
            activating: Cell::new(false),
            kept: RefCell::new(Vec::new()),
        });
        parent
            .provide::<TreeContext<S>>(Rc::downgrade(&tree))
            .map_err(|error| S::error(&error.to_string()))?;
        let mounted = setup(&tree)?;
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
        Ok((tree, mounted))
    }

    pub(crate) fn from_owner(owner: &OwnerHandle) -> Option<Rc<Self>> {
        if owner.is_disposed() {
            return None;
        }
        owner
            .context::<TreeContext<S>>()?
            .upgrade()
            .filter(|tree| !tree.disposed.get())
    }

    fn keep(&self, value: Registration) {
        self.kept.borrow_mut().push(value);
    }

    pub(crate) fn location(&self) -> Signal<AppUrl> {
        self.location.clone()
    }

    #[cfg(feature = "browser")]
    pub(crate) fn accepts_link(&self, url: &AppUrl) -> bool {
        self.root.views.accepts_link(url)
    }

    pub(crate) fn idle(&self) -> Result<(), S::Error> {
        if self.disposed.get() {
            return Err(S::error(INACTIVE));
        }
        if self.staged.get() || self.activating.get() {
            return Err(S::error(REENTRANT));
        }
        Ok(())
    }

    pub(crate) fn prepare_navigation(
        self: &Rc<Self>,
        url: &AppUrl,
    ) -> Result<Box<dyn PreparedNavigation>, S::Error> {
        self.idle()?;
        let pending = Pending::new(self.clone());
        Ok(Box::new(Transaction {
            plan: self.root.prepare(self, url)?,
            url: url.clone(),
            _pending: pending,
        }))
    }

    fn activate(&self) {
        if self.activating.replace(true) {
            return;
        }
        struct Reset<'a>(&'a Cell<bool>);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let _activating = Reset(&self.activating);
        self.root.activate();
    }

    pub(crate) fn dispose(&self) {
        if self.disposed.replace(true) {
            return;
        }
        self.root.clear();
        drop(self.kept.take());
    }
}

impl<S: RouteScope> Drop for Tree<S> {
    fn drop(&mut self) {
        self.dispose();
    }
}

struct Pending<S: RouteScope>(Rc<Tree<S>>);
impl<S: RouteScope> Pending<S> {
    fn new(tree: Rc<Tree<S>>) -> Self {
        tree.staged.set(true);
        Self(tree)
    }
}
impl<S: RouteScope> Drop for Pending<S> {
    fn drop(&mut self) {
        self.0.staged.set(false);
    }
}

struct Transaction<S: RouteScope> {
    plan: Plan<S>,
    url: AppUrl,
    _pending: Pending<S>,
}
impl<S: RouteScope> PreparedNavigation for Transaction<S> {
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
