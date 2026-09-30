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
pub(crate) struct State {
    pub owner: OwnerHandle,
    pub entry: &'static str,
    pub pool_required: bool,
    pub endpoint: Weak<Endpoint>,
    pub keepalive: Option<Rc<Endpoint>>,
    pub dedicated: bool,
    pub bound: bool,
    pub instance: u64,
    pub id: Option<u64>,
    pub arguments: Option<Arguments>,
    pub events: VecDeque<Event>,
    pub abort: Option<WorkerError>,
    pub committed: bool,
    pub terminal: bool,
    pub submitted: bool,
    pub stream: bool,
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
        pool_required: bool,
        arguments: Arguments,
        stream: bool,
    ) -> Rc<RefCell<Self>> {
        let state = Rc::new(RefCell::new(Self {
            owner: owner.clone(),
            entry,
            pool_required,
            endpoint: Weak::new(),
            keepalive: None,
            dedicated: false,
            bound: false,
            instance: 0,
            id: None,
            arguments: Some(arguments),
            events: VecDeque::new(),
            abort: None,
            committed: false,
            terminal: false,
            submitted: false,
            stream,
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
            if state.committed || state.terminal || state.abort.is_some() {
                return;
            }
            state.abort = Some(error);
            (
                state.endpoint.upgrade(),
                state.id,
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
        if state.borrow().submitted {
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
                state.endpoint.upgrade(),
                state.committed || state.terminal || state.abort.is_some(),
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
                state.borrow_mut().terminal = true;
            }
            state.borrow_mut().events.push_back(event);
            let wake = state.borrow_mut().waker.take();
            if let Some(wake) = wake {
                wake.wake();
            }
        }
    }
    pub fn start(state: &Rc<RefCell<Self>>) {
        if state.borrow().submitted {
            return;
        }
        state.borrow_mut().submitted = true;
        if state.borrow().abort.is_some() {
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
        if let Some(endpoint) = self.endpoint.upgrade() {
            if let Some(id) = self.id {
                endpoint.forget(id);
            }
            if !self.committed && !self.terminal {
                if let Some(id) = self.id {
                    endpoint.cancel(id);
                }
            }
            for event in self.events.drain(..) {
                endpoint.discard(event);
            }
        }
    }
}
