use super::{
    Form, FormError, Snapshot, SnapshotInfo, SubmissionStatus,
    field::{Core, FieldStamp, Fields, Issue, TextField},
};
use crate::actions::{Action, AdmissionError, Publication, Retired, Status};
use fusor::{Signal, batch, untrack};
use std::{collections::BTreeSet, rc::Rc};

pub struct SubmitError<C> {
    pub reason: FormError,
    pub snapshot: Snapshot<C>,
}
impl<C> std::fmt::Debug for SubmitError<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubmitError")
            .field("reason", &self.reason)
            .field("snapshot", &self.snapshot.id())
            .finish()
    }
}
impl<C> std::fmt::Display for SubmitError<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.reason.fmt(f)
    }
}
impl<C> std::error::Error for SubmitError<C> {}

type Commit = Box<dyn FnOnce(&mut Retired)>;
/// Staged acknowledgment, valid only for the supplied submission. Methods latch
/// errors. An invalid mapping publishes `PublicationFailed` without applying any
/// staged updates; inspect the form/action's publication error before reconciling.
pub struct Acknowledgment {
    snapshot: Rc<SnapshotInfo>,
    mapped: BTreeSet<u64>,
    checks: Vec<Box<dyn Fn() -> Result<(), FormError>>>,
    writes: Vec<Commit>,
    after: Vec<Box<dyn FnOnce()>>,
    error: Option<FormError>,
}
impl Acknowledgment {
    fn new(snapshot: Rc<SnapshotInfo>) -> Self {
        Self {
            snapshot,
            mapped: BTreeSet::new(),
            checks: Vec::new(),
            writes: Vec::new(),
            after: Vec::new(),
            error: None,
        }
    }
    /// Reject an invalid response (for example, a mismatched returned entity ID).
    pub fn reject(&mut self, message: impl Into<String>) {
        self.error = Some(FormError::Mapping(message.into()));
    }
    fn stamp<T: 'static>(&mut self, field: &TextField<T>) -> Option<FieldStamp> {
        let id = field.stamp().id;
        let Some(stamp) = self
            .snapshot
            .fields
            .iter()
            .find(|stamp| stamp.id == id)
            .cloned()
        else {
            self.error = Some(FormError::UnknownField);
            return None;
        };
        if !self.mapped.insert(id) {
            self.error = Some(FormError::DuplicateMapping);
            return None;
        }
        Some(stamp)
    }
    pub fn field<T: 'static>(&mut self, field: &TextField<T>, value: T) {
        let Some(stamp) = self.stamp(field) else {
            return;
        };
        let raw = field.formatted(&value);
        let core = field.core();
        let check = stamp.clone();
        self.checks.push(Box::new(move || {
            if check.baseline_current() {
                Ok(())
            } else {
                Err(FormError::StaleBaseline)
            }
        }));
        let submitted = self.snapshot.fields.clone();
        let submission = self.snapshot.id;
        self.writes.push(Box::new(move |_| {
            core.state.update(|state| {
                state.baseline = raw.clone();
                state.baseline_revision = state
                    .baseline_revision
                    .checked_add(1)
                    .expect("baseline revision overflow");
                if state.revision == stamp.revision && state.raw != raw {
                    state.raw = raw;
                    // Normalization changes the value that validation observed,
                    // even though it is not a new user keystroke.
                    state.revision = state
                        .revision
                        .checked_add(1)
                        .expect("field revision overflow");
                }
                if state
                    .server
                    .as_ref()
                    .is_some_and(|issue| issue.superseded_by(&submitted, submission))
                {
                    state.server = None;
                }
            });
        }));
    }
    /// Stage a scalar alongside fields. For a numeric server version use `version`
    /// so a response cannot lower the confirmed version or race another update.
    pub fn signal<T: 'static>(&mut self, signal: &Signal<T>, value: T) {
        let signal = signal.clone();
        self.writes.push(Box::new(move |retired| {
            retired.push(Box::new(signal.replace(value)));
        }));
    }
    pub fn version(&mut self, signal: &Signal<u64>, value: u64) {
        let expected = signal.get_untracked();
        if value < expected {
            self.error = Some(FormError::VersionRegression);
            return;
        }
        let guard = signal.clone();
        self.checks.push(Box::new(move || {
            if guard.get_untracked() == expected {
                Ok(())
            } else {
                Err(FormError::StaleBaseline)
            }
        }));
        self.signal(signal, value);
    }
    pub fn after_commit(&mut self, callback: impl FnOnce() + 'static) {
        self.after.push(Box::new(callback));
    }
    fn validate(&self) -> Result<(), FormError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        // A changed baseline in any submitted dependency invalidates the response,
        // even if only a subset of fields is explicitly mapped.
        if !self
            .snapshot
            .fields
            .iter()
            .all(FieldStamp::baseline_current)
        {
            return Err(FormError::StaleBaseline);
        }
        for check in &self.checks {
            check()?;
        }
        Ok(())
    }
}

/// Server validation belongs to the submitted dependency set, not later typing.
pub struct Rejection {
    snapshot: Rc<SnapshotInfo>,
    fields: Vec<(Rc<Core>, String)>,
    message: Option<String>,
    error: Option<FormError>,
    mapped: BTreeSet<u64>,
}
impl Rejection {
    fn new(snapshot: Rc<SnapshotInfo>) -> Self {
        Self {
            snapshot,
            fields: Vec::new(),
            message: None,
            error: None,
            mapped: BTreeSet::new(),
        }
    }
    pub fn field<T: 'static>(&mut self, field: &TextField<T>, message: impl Into<String>) {
        let id = field.stamp().id;
        if !self.snapshot.fields.iter().any(|stamp| stamp.id == id) {
            self.error = Some(FormError::UnknownField);
            return;
        }
        if !self.mapped.insert(id) {
            self.error = Some(FormError::DuplicateMapping);
            return;
        }
        self.fields.push((field.core(), message.into()));
    }
    pub fn form(&mut self, message: impl Into<String>) {
        self.message = Some(message.into());
    }
}

impl<F: Fields, C: 'static> Form<F, C> {
    pub fn submit<T: 'static, E: 'static>(
        &self,
        action: &Action<C, T, E>,
        snapshot: Snapshot<C>,
        accepted: impl FnOnce(&mut Acknowledgment, &T) + 'static,
    ) -> Result<u64, SubmitError<C>> {
        self.submit_with(action, snapshot, accepted, |_, _| {})
    }
    /// Prepare immediately before submission. Admission failure returns the
    /// unconsumed snapshot; stale snapshots must be prepared again.
    pub fn submit_with<T: 'static, E: 'static>(
        &self,
        action: &Action<C, T, E>,
        snapshot: Snapshot<C>,
        accepted: impl FnOnce(&mut Acknowledgment, &T) + 'static,
        rejected: impl FnOnce(&mut Rejection, &E) + 'static,
    ) -> Result<u64, SubmitError<C>> {
        let check = || {
            if snapshot.info.form != self.0.id || snapshot.info.entity != self.0.entity {
                return Err(FormError::WrongForm);
            }
            if self.0.owner.is_disposed() {
                return Err(FormError::Disposed);
            }
            if !self.0.owner.is_active() {
                return Err(FormError::NotActive);
            }
            if self.0.pending.get().is_some() {
                return Err(FormError::Busy);
            }
            if self
                .0
                .state
                .with_untracked(|state| state.status.unresolved())
            {
                return Err(FormError::Unresolved);
            }
            if snapshot.info.generation != self.0.generation.get()
                || !snapshot
                    .info
                    .fields
                    .iter()
                    .all(|stamp| stamp.current(false))
            {
                return Err(FormError::StaleSnapshot);
            }
            Ok(())
        };
        if let Err(reason) = check() {
            return Err(SubmitError { reason, snapshot });
        }
        untrack(|| {
            batch(|| {
                self.0.pending.set(Some(snapshot.id()));
                let previous = self.0.state.get_untracked();
                let previous_action = self.0.action.replace(Some(action.identity()));
                let previous_generation = self.0.generation.get();
                self.advance_generation();
                self.0.state.update(|state| {
                    state.status = SubmissionStatus::Pending;
                    state.snapshot = Some(snapshot.info.clone());
                    state.publication_error = None;
                });
                let form = self.clone();
                let cancelled_form = std::rc::Rc::downgrade(&self.0);
                let cancelled_id = snapshot.id();
                let info = snapshot.info.clone();
                let result = action.dispatch_with(
                    snapshot.command.clone(),
                    move |outcome| {
                        if !form.0.owner.is_active() || form.0.pending.get() != Some(info.id) {
                            return Err(FormError::Disposed.to_string());
                        }
                        let mut ack = Acknowledgment::new(info.clone());
                        let mut errors = Rejection::new(info.clone());
                        let mut status = match outcome.status {
                            Status::Accepted => {
                                // Dispose unused user captures before validating
                                // the final mapping, outside all internal borrows.
                                drop(rejected);
                                accepted(
                                    &mut ack,
                                    outcome.value.as_deref().expect("accepted value"),
                                );
                                SubmissionStatus::Accepted
                            }
                            Status::Rejected => {
                                drop(accepted);
                                rejected(&mut errors, outcome.error.as_deref().expect("rejection"));
                                SubmissionStatus::Rejected
                            }
                            Status::Conflict => {
                                drop(accepted);
                                drop(rejected);
                                SubmissionStatus::Conflict
                            }
                            Status::Unknown => {
                                drop(accepted);
                                drop(rejected);
                                SubmissionStatus::Unknown
                            }
                            _ => unreachable!("completed server outcome"),
                        };
                        if !form.0.owner.is_active() || form.0.pending.get() != Some(info.id) {
                            return Err(FormError::Disposed.to_string());
                        }
                        let failure = if status == SubmissionStatus::Accepted {
                            ack.validate().err()
                        } else {
                            errors.error.clone()
                        };
                        let failure = failure.map(|error| error.to_string());
                        if failure.is_some() {
                            status = SubmissionStatus::PublicationFailed;
                            ack.writes.clear();
                            ack.after.clear();
                            errors.fields.clear();
                            errors.message = None;
                        }
                        // Discarding failed staged values can run application
                        // destructors, including disposal of a separate form owner.
                        if !form.0.owner.is_active() || form.0.pending.get() != Some(info.id) {
                            return Err(FormError::Disposed.to_string());
                        }
                        let error_message = failure.clone();
                        Ok(Publication {
                            failure,
                            after: ack.after,
                            commit: Box::new(move |retired| {
                                for write in ack.writes {
                                    write(retired);
                                }
                                for (core, message) in errors.fields {
                                    let issue = Rc::new(Issue {
                                        message,
                                        dependencies: info.fields.clone(),
                                        origin: info.id,
                                    });
                                    core.state.update(|state| {
                                        if state.server.as_ref().is_none_or(|old| {
                                            old.superseded_by(&info.fields, info.id)
                                        }) {
                                            state.server = Some(issue);
                                        }
                                    });
                                }
                                form.0.pending.set(None);
                                form.0.state.update(|state| {
                                    state.status = status;
                                    state.publication_error = error_message;
                                    let superseded =
                                        state.validation.as_ref().is_none_or(|issue| {
                                            issue.superseded_by(&info.fields, info.id)
                                        });
                                    if superseded {
                                        if let Some(message) = errors.message {
                                            state.validation = Some(Rc::new(Issue {
                                                message,
                                                dependencies: info.fields.clone(),
                                                origin: info.id,
                                            }));
                                        } else if status == SubmissionStatus::Accepted {
                                            state.validation = None;
                                        }
                                    }
                                });
                            }),
                        })
                    },
                    move || {
                        if let Some(form) = cancelled_form
                            .upgrade()
                            .filter(|form| form.pending.get() == Some(cancelled_id))
                        {
                            form.pending.set(None);
                            form.state.update(|state| {
                                state.status = SubmissionStatus::Disposed;
                                state.snapshot = None;
                            });
                        }
                    },
                );
                match result {
                    Ok(id) => Ok(id),
                    Err(reason) => {
                        self.0.pending.set(None);
                        self.0.action.set(previous_action);
                        self.0.generation.set(previous_generation);
                        self.0.state.update(|state| *state = previous);
                        Err(SubmitError {
                            reason: match reason {
                                AdmissionError::Busy => FormError::Busy,
                                AdmissionError::Disposed => FormError::Disposed,
                                AdmissionError::NotActive => FormError::NotActive,
                                AdmissionError::Unresolved => FormError::Unresolved,
                            },
                            snapshot,
                        })
                    }
                }
            })
        })
    }
    /// The application has explicitly reconciled the uncertain/conflicting write
    /// and reviewed/reset its baseline. Unlock both form and its actual action.
    pub fn reconcile<T: 'static, E: 'static>(
        &self,
        action: &Action<C, T, E>,
    ) -> Result<(), FormError> {
        if self.0.action.get() != Some(action.identity()) {
            return Err(FormError::WrongAction);
        }
        if !self.0.owner.is_active() {
            return Err(FormError::Disposed);
        }
        if self.0.pending.get().is_some() {
            return Err(FormError::Busy);
        }
        batch(|| {
            action.reconcile().map_err(|error| match error {
                AdmissionError::Busy => FormError::Busy,
                AdmissionError::NotActive => FormError::NotActive,
                AdmissionError::Disposed => FormError::Disposed,
                AdmissionError::Unresolved => FormError::Unresolved,
            })?;
            self.advance_generation();
            self.0.state.update(|state| {
                state.status = SubmissionStatus::Idle;
                state.snapshot = None;
                state.publication_error = None;
            });
            Ok(())
        })
    }
}
