//! Explicit writes. Admission, server outcome and local publication are distinct.
//! Disposing an action suppresses local publication, never promises server rollback.
use fusor::{OwnerHandle, Registration, Signal, batch, signal, untrack};
pub use fusor_async::CancellationToken;
use fusor_async::{CancellationSource, Loader, Spawner, boxed_loader};
use futures_util::future::{AbortHandle, Abortable, LocalBoxFuture};
use std::{
    any::Any,
    cell::{Cell, RefCell},
    future::Future,
    rc::Rc,
};

#[cfg(feature = "browser")]
pub mod browser;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavePolicy {
    RejectWhilePending,
}

/// The transport classifies outcomes according to its server protocol. A failed
/// connection normally cannot establish rejection; use `Unknown` in that case.
#[derive(Debug)]
pub enum Outcome<T, E> {
    Accepted(T),
    Rejected(E),
    Conflict(E),
    Unknown(E),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Pending,
    Accepted,
    Rejected,
    Conflict,
    Unknown,
    PublicationFailed,
    Disposed,
}
impl Status {
    pub fn unresolved(self) -> bool {
        matches!(
            self,
            Self::Conflict | Self::Unknown | Self::PublicationFailed
        )
    }
}

/// Immutable identity and command of one admitted dispatch.
#[derive(Debug)]
pub struct Submission<C> {
    pub id: u64,
    pub command: Rc<C>,
}
impl<C> Clone for Submission<C> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            command: self.command.clone(),
        }
    }
}

/// Read-only state; payloads have no Clone requirement. An unresolved outcome
/// retains its submission until the application explicitly reconciles it.
#[derive(Debug)]
pub struct ActionState<C, T, E> {
    pub status: Status,
    pub submission: Option<Submission<C>>,
    pub value: Option<Rc<T>>,
    pub error: Option<Rc<E>>,
    pub publication_error: Option<String>,
}
impl<C, T, E> Clone for ActionState<C, T, E> {
    fn clone(&self) -> Self {
        Self {
            status: self.status,
            submission: self.submission.clone(),
            value: self.value.clone(),
            error: self.error.clone(),
            publication_error: self.publication_error.clone(),
        }
    }
}
impl<C, T, E> ActionState<C, T, E> {
    fn completed(submission: Submission<C>, outcome: Outcome<T, E>) -> Self {
        let mut next = Self {
            submission: Some(submission),
            ..Self::empty(Status::Idle)
        };
        match outcome {
            Outcome::Accepted(value) => {
                next.status = Status::Accepted;
                next.value = Some(Rc::new(value));
            }
            Outcome::Rejected(error) => {
                next.status = Status::Rejected;
                next.error = Some(Rc::new(error));
            }
            Outcome::Conflict(error) => {
                next.status = Status::Conflict;
                next.error = Some(Rc::new(error));
            }
            Outcome::Unknown(error) => {
                next.status = Status::Unknown;
                next.error = Some(Rc::new(error));
            }
        }
        next
    }

    fn empty(status: Status) -> Self {
        Self {
            status,
            submission: None,
            value: None,
            error: None,
            publication_error: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Busy,
    NotActive,
    Disposed,
    Unresolved,
}
impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "action admission: {self:?}")
    }
}
impl std::error::Error for AdmissionError {}

/// Failed admission returns ownership of the unsubmitted command.
#[derive(Debug)]
pub struct DispatchError<C> {
    pub reason: AdmissionError,
    pub command: C,
}
impl<C> std::fmt::Display for DispatchError<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.reason.fmt(f)
    }
}
impl<C: std::fmt::Debug> std::error::Error for DispatchError<C> {}

struct Request {
    abort: AbortHandle,
    cancellation: Option<CancellationSource>,
    on_dispose: Option<Box<dyn FnOnce()>>,
}
impl Request {
    fn complete(mut self) {
        self.on_dispose.take();
        if let Some(source) = self.cancellation.take() {
            source.complete();
        }
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        if let Some(callback) = self.on_dispose.take() {
            callback();
        }
        if let Some(source) = &self.cancellation {
            self.abort.abort();
            source.cancel();
        }
    }
}
struct Inner<C, T, E> {
    #[cfg(feature = "forms")]
    id: u64,
    owner: OwnerHandle,
    disposed: Cell<bool>,
    busy: Cell<bool>,
    state: Signal<ActionState<C, T, E>>,
    request: RefCell<Option<Request>>,
    registration: RefCell<Option<Registration>>,
    load: Rc<Loader<Rc<C>, Outcome<T, E>>>,
    spawn: Rc<Spawner>,
}

/// Owned local action with a single admitted command. Clones share admission.
/// The owner must be active; prepared owners return `NotActive` without queuing.
pub struct Action<C, T, E>(Rc<Inner<C, T, E>>);
impl<C, T, E> Clone for Action<C, T, E> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

// Staging is internal: forms validate the entire mapping before producing this
// infallible commit. Old user values are retired after all local state is coherent.
pub(crate) type Retired = Vec<Box<dyn Any>>;
pub(crate) struct Publication {
    pub commit: Box<dyn FnOnce(&mut Retired)>,
    pub after: Vec<Box<dyn FnOnce()>>,
    pub failure: Option<String>,
}
impl Default for Publication {
    fn default() -> Self {
        Self {
            commit: Box::new(|_| {}),
            after: Vec::new(),
            failure: None,
        }
    }
}

impl<C: 'static, T: 'static, E: 'static> Action<C, T, E> {
    pub fn new<F: Future<Output = Outcome<T, E>> + 'static>(
        owner: &OwnerHandle,
        _policy: SavePolicy,
        load: impl Fn(Rc<C>, CancellationToken) -> F + 'static,
        spawn: impl Fn(LocalBoxFuture<'static, ()>) + 'static,
    ) -> Self {
        let inner = Rc::new(Inner {
            #[cfg(feature = "forms")]
            id: crate::identity::next(),
            owner: owner.clone(),
            disposed: Cell::new(false),
            busy: Cell::new(false),
            state: signal(ActionState::empty(Status::Idle)),
            request: RefCell::new(None),
            registration: RefCell::new(None),
            load: boxed_loader(load),
            spawn: Rc::new(spawn),
        });
        let weak = Rc::downgrade(&inner);
        let registration = owner.on_cleanup(move || {
            if let Some(inner) = weak.upgrade() {
                dispose(&inner);
            }
        });
        *inner.registration.borrow_mut() = Some(registration);
        Self(inner)
    }

    pub fn state(&self) -> ActionState<C, T, E> {
        self.0.state.get()
    }
    pub fn pending(&self) -> bool {
        self.0.state.with(|state| state.status == Status::Pending)
    }
    pub fn dispose(&self) {
        dispose(&self.0);
    }

    /// Explicitly allow another write after resolving a conflict, uncertain
    /// outcome, or failed local mapping. Performs no I/O, retry or draft reset.
    /// The application must establish the server baseline before calling this.
    pub fn reconcile(&self) -> Result<(), AdmissionError> {
        self.available(true)?;
        drop(self.0.state.replace(ActionState::empty(Status::Idle)));
        Ok(())
    }

    pub fn dispatch(&self, command: C) -> Result<u64, DispatchError<C>> {
        if let Err(reason) = self.available(false) {
            return Err(DispatchError { reason, command });
        }
        // No callbacks or reactive writes occur between admission and reservation.
        Ok(self.start(Rc::new(command), |_| Ok(Publication::default()), || {}))
    }

    fn available(&self, allow_unresolved: bool) -> Result<(), AdmissionError> {
        if self.0.disposed.get() || self.0.owner.is_disposed() {
            return Err(AdmissionError::Disposed);
        }
        if !self.0.owner.is_active() {
            return Err(AdmissionError::NotActive);
        }
        if self.0.busy.get() {
            return Err(AdmissionError::Busy);
        }
        if !allow_unresolved
            && self
                .0
                .state
                .with_untracked(|state| state.status.unresolved())
        {
            return Err(AdmissionError::Unresolved);
        }
        Ok(())
    }

    #[cfg(feature = "forms")]
    pub(crate) fn identity(&self) -> u64 {
        self.0.id
    }

    #[cfg(feature = "forms")]
    pub(crate) fn dispatch_with(
        &self,
        command: Rc<C>,
        publish: impl FnOnce(&ActionState<C, T, E>) -> Result<Publication, String> + 'static,
        on_dispose: impl FnOnce() + 'static,
    ) -> Result<u64, AdmissionError> {
        self.available(false)?;
        Ok(self.start(command, publish, on_dispose))
    }

    fn start(
        &self,
        command: Rc<C>,
        publish: impl FnOnce(&ActionState<C, T, E>) -> Result<Publication, String> + 'static,
        on_dispose: impl FnOnce() + 'static,
    ) -> u64 {
        let submission = Submission {
            id: crate::identity::next(),
            command,
        };
        let id = submission.id;
        self.0.busy.set(true);
        let cancellation = CancellationSource::default();
        let token = cancellation.token();
        let (abort, registration) = AbortHandle::new_pair();
        *self.0.request.borrow_mut() = Some(Request {
            abort,
            cancellation: Some(cancellation),
            on_dispose: Some(Box::new(on_dispose)),
        });
        let old = self.0.state.update(|state| {
            std::mem::replace(
                state,
                ActionState {
                    submission: Some(submission.clone()),
                    ..ActionState::empty(Status::Pending)
                },
            )
        });
        drop(old);
        let weak = Rc::downgrade(&self.0);
        let load = self.0.load.clone();
        let work = async move {
            let Some(inner) = weak.upgrade().filter(|inner| current(inner, id)) else {
                return;
            };
            drop(inner);
            let outcome = untrack(|| load(submission.command.clone(), token)).await;
            let Some(inner) = weak.upgrade().filter(|inner| current(inner, id)) else {
                return;
            };
            let next = ActionState::completed(submission, outcome);
            let publication = untrack(|| publish(&next));
            if !current(&inner, id) {
                return;
            }
            commit(&inner, next, publication);
        };
        (self.0.spawn)(Box::pin(async move {
            let _ = Abortable::new(work, registration).await;
        }));
        id
    }
}

fn commit<C: 'static, T: 'static, E: 'static>(
    inner: &Inner<C, T, E>,
    mut next: ActionState<C, T, E>,
    publication: Result<Publication, String>,
) {
    let publication = match publication {
        Ok(publication) => publication,
        Err(error) => {
            next.status = Status::PublicationFailed;
            next.publication_error = Some(error);
            Publication::default()
        }
    };
    if let Some(error) = &publication.failure {
        next.status = Status::PublicationFailed;
        next.publication_error = Some(error.clone());
    }
    let request = inner.request.take();
    let mut retired: Retired = Vec::new();
    batch(|| {
        (publication.commit)(&mut retired);
        retired.push(Box::new(inner.state.replace(next)));
        inner.busy.set(false);
    });
    // Cancellation callbacks and user destructors cannot run amid commits.
    if let Some(request) = request {
        request.complete();
    }
    drop(retired);
    for callback in publication.after {
        if !inner.disposed.get() && inner.owner.is_active() {
            untrack(callback);
        }
    }
}

fn current<C, T, E>(inner: &Inner<C, T, E>, id: u64) -> bool {
    !inner.disposed.get()
        && inner.owner.is_active()
        && inner.busy.get()
        && inner.state.with_untracked(|state| {
            state
                .submission
                .as_ref()
                .is_some_and(|submission| submission.id == id)
        })
}
fn dispose<C, T, E>(inner: &Inner<C, T, E>) {
    if inner.disposed.replace(true) {
        return;
    }
    inner.busy.set(false);
    let request = inner.request.take();
    let old = inner.state.replace(ActionState::empty(Status::Disposed));
    drop(request);
    drop(old);
}
