//! Typed, revisioned drafts and explicit submission snapshots. Query results do
//! not implicitly reset fields. An immutable form identity belongs to one entity.
mod field;
pub use field::{FieldStamp, Fields, TextField};
#[cfg(feature = "actions")]
mod submission;
#[cfg(feature = "actions")]
pub use submission::{Acknowledgment, Rejection, SubmitError};

use fusor::{OwnerHandle, Registration, Signal, batch, signal, untrack};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    rc::Rc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormError {
    Invalid,
    ChangedDuringPreparation,
    WrongForm,
    StaleSnapshot,
    DuplicateField,
    UnknownField,
    DuplicateMapping,
    StaleBaseline,
    VersionRegression,
    Busy,
    NotActive,
    Disposed,
    Unresolved,
    WrongAction,
    Mapping(String),
}
impl std::fmt::Display for FormError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "form validation failed; correct the reported fields",
            Self::ChangedDuringPreparation => {
                "form changed while preparing; prepare a fresh snapshot"
            }
            Self::WrongForm => "snapshot belongs to another form; prepare it from this form",
            Self::StaleSnapshot => "snapshot is stale; prepare it again after the latest edit",
            Self::DuplicateField => "form contains a repeated field; include each field once",
            Self::UnknownField => {
                "response maps a field outside this form; correct the response mapping"
            }
            Self::DuplicateMapping => "response maps a field twice; map each field once",
            Self::StaleBaseline => {
                "confirmed state changed; reconcile before applying this response"
            }
            Self::VersionRegression => {
                "response lowers the confirmed version; reconcile server state"
            }
            Self::Busy => "form is saving; wait for the pending submission",
            Self::NotActive => "form owner is not active; commit its owner before submitting",
            Self::Disposed => "editing session is closed; create a new form",
            Self::Unresolved => "save outcome is unresolved; reconcile before submitting again",
            Self::WrongAction => {
                "form outcome belongs to another action; reconcile its original action"
            }
            Self::Mapping(message) => {
                return write!(
                    f,
                    "response mapping failed: {message}; reconcile server state"
                );
            }
        })
    }
}
impl std::error::Error for FormError {}
#[cfg(feature = "browser")]
impl From<FormError> for wasm_bindgen::JsValue {
    fn from(error: FormError) -> Self {
        Self::from_str(&error.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmissionStatus {
    Idle,
    Pending,
    Accepted,
    Rejected,
    Conflict,
    Unknown,
    PublicationFailed,
    Disposed,
}
impl SubmissionStatus {
    pub fn unresolved(self) -> bool {
        matches!(
            self,
            Self::Conflict | Self::Unknown | Self::PublicationFailed
        )
    }
}
pub(super) struct SnapshotInfo {
    id: u64,
    form: u64,
    generation: u64,
    entity: Rc<str>,
    fields: Vec<FieldStamp>,
}

/// A single-use immutable command and field revision set. Rejected admission
/// returns this snapshot. A later edit/reset makes it stale; prepare again.
/// Command builders must capture owned values, not live signals or externally
/// mutable command payloads; Rust interior mutability cannot be frozen by this API.
pub struct Snapshot<C> {
    pub(super) info: Rc<SnapshotInfo>,
    pub(super) command: Rc<C>,
}
impl<C> Snapshot<C> {
    pub fn id(&self) -> u64 {
        self.info.id
    }
    pub fn form_id(&self) -> u64 {
        self.info.form
    }
    pub fn generation(&self) -> u64 {
        self.info.generation
    }
    pub fn entity(&self) -> &str {
        &self.info.entity
    }
    pub fn command(&self) -> &C {
        &self.command
    }
}
#[derive(Clone)]
struct State {
    status: SubmissionStatus,
    snapshot: Option<Rc<SnapshotInfo>>,
    validation: Option<Rc<field::Issue>>,
    publication_error: Option<FormError>,
}
type Validator<V> = dyn Fn(&V) -> Result<(), String>;
struct Inner<F: Fields, C> {
    id: u64,
    entity: Rc<str>,
    owner: OwnerHandle,
    fields: F,
    build: Box<dyn Fn(F::Values) -> C>,
    validators: RefCell<Vec<Rc<Validator<F::Values>>>>,
    state: Signal<State>,
    pending: Cell<Option<u64>>,
    generation: Cell<u64>,
    #[cfg(feature = "actions")]
    action: Cell<Option<u64>>,
    registration: RefCell<Option<Registration>>,
}
pub struct Form<F: Fields, C>(Rc<Inner<F, C>>);
impl<F: Fields, C> Clone for Form<F, C> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<F: Fields, C: 'static> Form<F, C> {
    /// `entity` and the command builder belong to one immutable editing session.
    /// Change entities by creating a new form, not by mutating captured IDs.
    pub fn new(
        owner: &OwnerHandle,
        entity: impl Into<String>,
        fields: F,
        build: impl Fn(F::Values) -> C + 'static,
    ) -> Result<Self, FormError> {
        let stamps = fields.stamps();
        if stamps
            .iter()
            .map(|stamp| stamp.id)
            .collect::<BTreeSet<_>>()
            .len()
            != stamps.len()
        {
            return Err(FormError::DuplicateField);
        }
        let inner = Rc::new(Inner {
            id: crate::identity::next(),
            entity: Rc::from(entity.into()),
            owner: owner.clone(),
            fields,
            build: Box::new(build),
            validators: RefCell::new(Vec::new()),
            state: signal(State {
                status: SubmissionStatus::Idle,
                snapshot: None,
                validation: None,
                publication_error: None,
            }),
            pending: Cell::new(None),
            generation: Cell::new(0),
            #[cfg(feature = "actions")]
            action: Cell::new(None),
            registration: RefCell::new(None),
        });
        let weak = Rc::downgrade(&inner);
        let registration = owner.on_cleanup(move || {
            if let Some(inner) = weak.upgrade() {
                inner.pending.set(None);
                inner.state.update(|state| {
                    state.status = SubmissionStatus::Disposed;
                    state.snapshot = None;
                    state.validation = None;
                });
            }
        });
        *inner.registration.borrow_mut() = Some(registration);
        Ok(Self(inner))
    }
    pub fn validate(self, validator: impl Fn(&F::Values) -> Result<(), String> + 'static) -> Self {
        self.0.validators.borrow_mut().push(Rc::new(validator));
        self.advance_generation();
        self.0.state.update(|state| state.validation = None);
        self
    }
    fn advance_generation(&self) {
        self.0.generation.set(
            self.0
                .generation
                .get()
                .checked_add(1)
                .expect("form generation overflow"),
        );
    }
    pub fn entity(&self) -> &str {
        &self.0.entity
    }
    pub fn dirty(&self) -> bool {
        self.0.fields.dirty()
    }
    pub fn status(&self) -> SubmissionStatus {
        self.0.state.with(|state| state.status)
    }
    pub fn validation_message(&self) -> String {
        self.0
            .state
            .with(|state| state.validation.clone())
            .filter(|issue| issue.current())
            .map_or_else(String::new, |issue| issue.message.clone())
    }
    pub fn publication_error(&self) -> Option<FormError> {
        self.0.state.with(|state| state.publication_error.clone())
    }
    pub fn submission_message(&self) -> String {
        let state = self.0.state.get();
        match state.status {
            SubmissionStatus::Pending => {
                if state.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.fields.iter().any(|stamp| !stamp.current(true))
                }) {
                    "Saving an earlier edit; newer changes exist"
                } else {
                    "Saving"
                }
            }
            SubmissionStatus::Rejected => "Save rejected; review the submitted errors",
            SubmissionStatus::Conflict => {
                "Version conflict; review the server state before saving again"
            }
            SubmissionStatus::Unknown => "Save outcome unknown; reconcile before saving again",
            SubmissionStatus::PublicationFailed => {
                "Server response could not update this session; reconcile before saving again"
            }
            SubmissionStatus::Disposed => "Editing session closed",
            _ if self.dirty() => "Unsaved changes",
            _ => "Saved",
        }
        .to_owned()
    }
    fn check_preparation(
        &self,
        generation: u64,
        stamps: &[field::FieldStamp],
    ) -> Result<(), FormError> {
        if self.0.owner.is_disposed() {
            return Err(FormError::Disposed);
        }
        if generation != self.0.generation.get() || !stamps.iter().all(|stamp| stamp.current(false))
        {
            return Err(FormError::ChangedDuringPreparation);
        }
        Ok(())
    }

    pub fn prepare(&self) -> Result<Snapshot<C>, FormError> {
        untrack(|| batch(|| self.prepare_snapshot()))
    }

    fn prepare_snapshot(&self) -> Result<Snapshot<C>, FormError> {
        if self.0.owner.is_disposed() {
            return Err(FormError::Disposed);
        }
        let stamps = self.0.fields.stamps();
        let generation = self.0.generation.get();
        self.0.fields.mark_submitted();
        self.0.state.update(|state| state.validation = None);
        let values = self.0.fields.values();
        self.check_preparation(generation, &stamps)?;
        let values = values?;
        let validators = self.0.validators.borrow().clone();
        for validator in validators {
            let result = validator(&values);
            self.check_preparation(generation, &stamps)?;
            if let Err(message) = result {
                self.0.state.update(|state| {
                    state.validation = Some(Rc::new(field::Issue {
                        message,
                        dependencies: stamps,
                        #[cfg(feature = "actions")]
                        origin: crate::identity::next(),
                    }))
                });
                return Err(FormError::Invalid);
            }
        }
        let command = (self.0.build)(values);
        self.check_preparation(generation, &stamps)?;
        Ok(Snapshot {
            info: Rc::new(SnapshotInfo {
                id: crate::identity::next(),
                form: self.0.id,
                generation,
                entity: self.0.entity.clone(),
                fields: stamps,
            }),
            command: Rc::new(command),
        })
    }
}
