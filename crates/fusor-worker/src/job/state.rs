use crate::{
    JobError, Message, WorkerError,
    endpoint::Endpoint,
    shared::{Codec, Payload},
};
use fusor::{OwnerHandle, Registration};
use fusor_async::{CancelRegistration, CancellationToken};
use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::{Rc, Weak},
    task::Waker,
};

pub(crate) type Arguments = Box<dyn FnOnce(&Codec) -> Result<Vec<Payload>, WorkerError>>;
type Progress = Box<dyn FnMut(Payload, &Codec) -> Result<(), WorkerError>>;

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum Event {
    Item(Payload),
    Result(Result<Payload, JobError<Payload>>),
    End(Result<(), JobError<Payload>>),
    Progress(Payload),
}
#[derive(Clone)]
pub(crate) enum Target {
    Shared,
    Dedicated,
    Bound {
        endpoint: Weak<Endpoint>,
        instance: u64,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Requirement {
    Ordinary,
    Pool,
}
impl From<bool> for Requirement {
    fn from(pool: bool) -> Self {
        if pool { Self::Pool } else { Self::Ordinary }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobKind {
    Single,
    Stream,
}

pub(crate) enum Completion {
    Pending,
    Ready,
    Aborted(WorkerError),
    Consumed,
}

impl Completion {
    pub fn error(&self) -> Option<&WorkerError> {
        match self {
            Self::Aborted(error) => Some(error),
            _ => None,
        }
    }

    pub fn take_error(&mut self) -> Option<WorkerError> {
        match std::mem::replace(self, Self::Consumed) {
            Self::Aborted(error) => Some(error),
            previous => {
                *self = previous;
                None
            }
        }
    }
}

enum Submission {
    Waiting(Arguments),
    Encoding(Arguments),
    Sent,
}

pub(crate) struct Admission {
    pub id: u64,
    endpoint: EndpointLink,
}

enum EndpointLink {
    Borrowed(Weak<Endpoint>),
    Owned(Rc<Endpoint>),
}

impl Admission {
    pub fn new(id: u64, endpoint: &Rc<Endpoint>) -> Self {
        Self {
            id,
            endpoint: EndpointLink::Borrowed(Rc::downgrade(endpoint)),
        }
    }

    pub fn retain(&mut self, endpoint: Rc<Endpoint>) {
        self.endpoint = EndpointLink::Owned(endpoint);
    }

    fn endpoint(&self) -> Option<Rc<Endpoint>> {
        match &self.endpoint {
            EndpointLink::Borrowed(endpoint) => endpoint.upgrade(),
            EndpointLink::Owned(endpoint) => Some(Rc::clone(endpoint)),
        }
    }
}

pub(crate) struct State {
    pub owner: OwnerHandle,
    pub entry: &'static str,
    pub requirement: Requirement,
    pub target: Target,
    pub admission: Option<Admission>,
    submission: Submission,
    pub events: VecDeque<Event>,
    pub completion: Completion,
    pub kind: JobKind,
    pub capacity: usize,
    pub batch_bytes: usize,
    pub waker: Option<Waker>,
    progress: Option<Progress>,
    owners: Vec<Registration>,
    tokens: Vec<(CancellationToken, CancelRegistration)>,
}
impl State {
    pub fn new(
        owner: &OwnerHandle,
        entry: &'static str,
        requirement: Requirement,
        arguments: Arguments,
        kind: JobKind,
    ) -> Rc<RefCell<Self>> {
        let state = Rc::new(RefCell::new(Self {
            owner: owner.clone(),
            entry,
            requirement,
            target: Target::Shared,
            admission: None,
            submission: Submission::Waiting(arguments),
            events: VecDeque::new(),
            completion: Completion::Pending,
            kind,
            capacity: 4,
            batch_bytes: 1024 * 1024,
            waker: None,
            progress: None,
            owners: Vec::new(),
            tokens: Vec::new(),
        }));
        Self::scope(&state, owner);
        state
    }
    pub fn endpoint(&self) -> Option<Rc<Endpoint>> {
        match &self.admission {
            Some(admission) => admission.endpoint(),
            None => match &self.target {
                Target::Bound { endpoint, .. } => endpoint.upgrade(),
                _ => None,
            },
        }
    }

    pub fn take_arguments(&mut self) -> Arguments {
        let Submission::Encoding(arguments) =
            std::mem::replace(&mut self.submission, Submission::Sent)
        else {
            unreachable!("started jobs encode their arguments once")
        };
        arguments
    }

    pub fn scope(state: &Rc<RefCell<Self>>, owner: &OwnerHandle) {
        let weak = Rc::downgrade(state);
        let registration = owner.on_cleanup(move || {
            if let Some(state) = weak.upgrade() {
                Self::abort(&state, WorkerError::OwnerDisposed);
            }
        });
        state.borrow_mut().owners.push(registration);
    }
    pub fn abort(state: &Rc<RefCell<Self>>, error: WorkerError) {
        let (endpoint, id, wake, discarded) = {
            let mut state = state.borrow_mut();
            if !matches!(state.completion, Completion::Pending) {
                return;
            }
            state.completion = Completion::Aborted(error);
            (
                state.endpoint(),
                state.admission.as_ref().map(|admission| admission.id),
                state.waker.take(),
                std::mem::take(&mut state.events),
            )
        };
        if let Some(endpoint) = endpoint {
            for event in discarded {
                endpoint.discard(event);
            }
            if let Some(id) = id {
                endpoint.cancel(id);
            }
        }
        if let Some(wake) = wake {
            wake.wake();
        }
    }
    pub fn configure(state: &Rc<RefCell<Self>>) -> bool {
        if !matches!(state.borrow().submission, Submission::Waiting(_)) {
            Self::abort(
                state,
                crate::error::configuration("builders must be configured before their first poll"),
            );
            false
        } else {
            true
        }
    }
    pub fn token(state: &Rc<RefCell<Self>>, token: &CancellationToken) {
        let weak = Rc::downgrade(state);
        let registration = token.on_cancel(move || {
            if let Some(state) = weak.upgrade() {
                Self::abort(&state, WorkerError::Cancelled);
            }
        });
        state
            .borrow_mut()
            .tokens
            .push((token.clone(), registration));
    }
    pub fn receive(state: &Rc<RefCell<Self>>, event: Event) {
        let (endpoint, ignored) = {
            let state = state.borrow();
            (
                state.endpoint(),
                !matches!(state.completion, Completion::Pending),
            )
        };
        let Some(endpoint) = endpoint else {
            return;
        };
        if ignored {
            endpoint.discard(event);
            return;
        }
        if let Event::Progress(payload) = event {
            let progress = state.borrow_mut().progress.take();
            if let Some(mut progress) = progress {
                let result = progress(payload, &endpoint.codec);
                state.borrow_mut().progress = Some(progress);
                if let Err(error) = result {
                    Self::abort(state, error);
                }
            } else {
                endpoint.codec.discard(payload);
            }
        } else {
            if matches!(event, Event::Result(_)) {
                state.borrow_mut().completion = Completion::Ready;
            }
            state.borrow_mut().events.push_back(event);
            let wake = state.borrow_mut().waker.take();
            if let Some(wake) = wake {
                wake.wake();
            }
        }
    }
    pub fn start(state: &Rc<RefCell<Self>>) {
        if !matches!(state.borrow().submission, Submission::Waiting(_)) {
            return;
        }
        {
            let mut state = state.borrow_mut();
            let Submission::Waiting(arguments) =
                std::mem::replace(&mut state.submission, Submission::Sent)
            else {
                unreachable!("only waiting jobs begin submission")
            };
            state.submission = Submission::Encoding(arguments);
        }
        if state.borrow().completion.error().is_some() {
            return;
        }
        let result = Endpoint::submit(state);
        if let Err(error) = result {
            Self::abort(state, error);
        }
    }
    pub(super) fn progress<P: Message>(
        state: &Rc<RefCell<Self>>,
        mut callback: impl FnMut(P) + 'static,
    ) {
        state.borrow_mut().progress = Some(Box::new(move |payload, codec| {
            callback(P::decode(payload, codec)?);
            Ok(())
        }));
    }
}
impl Drop for State {
    fn drop(&mut self) {
        if let Some(endpoint) = self.endpoint() {
            if let Some(admission) = &self.admission {
                endpoint.forget(admission.id);
            }
            if !matches!(self.completion, Completion::Consumed | Completion::Ready) {
                if let Some(admission) = &self.admission {
                    endpoint.cancel(admission.id);
                }
            }
            for event in self.events.drain(..) {
                endpoint.discard(event);
            }
        }
    }
}
