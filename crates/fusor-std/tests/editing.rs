#![cfg(all(feature = "forms", feature = "actions"))]

use fusor::{Owner, Signal, effect, signal};
use fusor_std::{
    actions::{Action, AdmissionError, Outcome, Status},
    forms::{Acknowledgment, Form, FormError, SubmissionStatus, TextField},
};
use fusor_test::{ControlledLoader, TestExecutor};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Debug, PartialEq)]
struct Command {
    id: u64,
    expected: u64,
    title: String,
    count: u32,
}
#[derive(Debug)]
struct Saved {
    version: u64,
    title: String,
    count: u32,
}
type EditorForm = Form<(TextField<String>, TextField<u32>), Command>;
type Save = Action<Command, Saved, String>;
type Backend = ControlledLoader<Rc<Command>, Outcome<Saved, String>, ()>;

type PublicationLog = Rc<RefCell<Vec<(String, String, u64, Status, SubmissionStatus)>>>;
struct Editor {
    owner: Rc<Owner>,
    executor: TestExecutor,
    backend: Backend,
    title: TextField<String>,
    count: TextField<u32>,
    version: Signal<u64>,
    form: EditorForm,
    save: Save,
}
impl Editor {
    fn new() -> Self {
        let owner = Rc::new(Owner::new());
        let executor = TestExecutor::new();
        let backend = Backend::new();
        let title = TextField::new("Thursday".to_owned()).validate(|value| {
            if value.trim().is_empty() {
                Err("Enter a title".into())
            } else {
                Ok(())
            }
        });
        let count = TextField::new(1_u32);
        let version = signal(10);
        let captured = version.clone();
        let form = Form::new(
            &owner.handle(),
            "project:7",
            (title.clone(), count.clone()),
            move |(title, count)| Command {
                id: 7,
                expected: captured.get_untracked(),
                title,
                count,
            },
        )
        .unwrap();
        let loader = backend.clone();
        let save = Action::new(
            &owner.handle(),
            move |command, context| {
                let future = loader.load(command, context);
                async move { future.await.unwrap() }
            },
            executor.spawner(),
        );
        owner.commit();
        Self {
            owner,
            executor,
            backend,
            title,
            count,
            version,
            form,
            save,
        }
    }
    fn observe_publication(&self) -> (PublicationLog, fusor::Effect) {
        let observed = Rc::new(RefCell::new(Vec::new()));
        let rows = observed.clone();
        let title = self.title.clone();
        let version = self.version.clone();
        let save = self.save.clone();
        let form = self.form.clone();
        let observer = effect(move || {
            rows.borrow_mut().push((
                title.raw(),
                title.baseline(),
                version.get(),
                save.state().status,
                form.status(),
            ))
        });
        (observed, observer)
    }

    fn submit_invalid_mapping(&self, failure: FormError) -> Rc<Cell<bool>> {
        let title = self.title.clone();
        let count = self.count.clone();
        let version = self.version.clone();
        let foreign = TextField::new("other".to_owned());
        let after = Rc::new(Cell::new(false));
        let after_copy = after.clone();
        self.form
            .submit(
                &self.save,
                self.form.prepare().unwrap(),
                move |ack, saved| {
                    ack.field(&title, saved.title.clone());
                    ack.field(&count, saved.count);
                    ack.version(&version, saved.version);
                    match failure {
                        FormError::UnknownField => ack.field(&foreign, "invalid".into()),
                        FormError::DuplicateMapping => ack.field(&title, "duplicate".into()),
                        FormError::VersionRegression => ack.version(&version, 9),
                        FormError::StaleBaseline => title.reset("reviewed".into()),
                        FormError::Mapping(message) => ack.reject(message),
                        _ => unreachable!("mapping rejection scenario"),
                    }
                    ack.after_commit(move || after_copy.set(true));
                },
            )
            .unwrap();
        after
    }

    fn mapping(&self) -> impl FnOnce(&mut Acknowledgment, &Saved) + 'static {
        let title = self.title.clone();
        let count = self.count.clone();
        let version = self.version.clone();
        move |ack, saved| {
            ack.field(&title, saved.title.clone());
            ack.field(&count, saved.count);
            ack.version(&version, saved.version);
        }
    }
    fn submit(&self) {
        self.form
            .submit(&self.save, self.form.prepare().unwrap(), self.mapping())
            .unwrap();
        self.executor.run_until_stalled();
    }
    fn accept(&self, title: &str, count: u32, version: u64) {
        self.complete(Outcome::Accepted(Saved {
            title: title.into(),
            count,
            version,
        }));
    }

    fn complete(&self, outcome: Outcome<Saved, String>) {
        self.backend
            .next_request()
            .unwrap()
            .complete(Ok(outcome))
            .unwrap();
        self.executor.run_until_stalled();
    }
}

#[test]
fn friday_then_monday_preserves_newer_draft_and_publishes_coherently() {
    let editor = Editor::new();
    let (observed, _observer) = editor.observe_publication();
    editor.title.edit("Friday");
    editor.count.edit("2");
    editor.submit();
    let request = editor.backend.next_request().unwrap();
    assert_eq!(
        *request.key,
        Command {
            id: 7,
            expected: 10,
            title: "Friday".into(),
            count: 2
        }
    );
    editor.title.edit("Monday");
    assert!(editor.form.submission_message().contains("earlier edit"));
    let snapshot = editor.form.prepare().unwrap();
    let id = snapshot.id();
    let error = editor
        .form
        .submit(&editor.save, snapshot, editor.mapping())
        .unwrap_err();
    assert_eq!(error.reason, FormError::Busy);
    assert_eq!(error.snapshot.id(), id);
    assert_eq!(
        editor.save.state().submission.unwrap().command.title,
        "Friday"
    );
    observed.borrow_mut().clear();
    request
        .complete(Ok(Outcome::Accepted(Saved {
            title: "Friday".into(),
            count: 2,
            version: 11,
        })))
        .unwrap();
    editor.executor.run_until_stalled();
    assert_eq!(
        *observed.borrow(),
        [(
            "Monday".into(),
            "Friday".into(),
            11,
            Status::Accepted,
            SubmissionStatus::Accepted
        )]
    );
    assert!(editor.form.dirty());
    assert!(!editor.count.dirty());
    editor.submit();
    assert_eq!(editor.save.state().submission.unwrap().command.expected, 11);
    editor.accept("Monday", 2, 12);
    assert!(!editor.form.dirty());
    assert_eq!(editor.form.submission_message(), "Saved");
    assert_eq!(editor.backend.counts().cancelled, 0);
}

#[test]
fn normalization_and_edit_reversion_use_raw_baseline_equality() {
    let editor = Editor::new();
    editor.title.edit(" Friday ");
    editor.submit();
    editor.accept("Friday", 1, 11);
    assert_eq!(editor.title.raw(), "Friday");
    assert!(!editor.title.dirty());
    editor.title.edit("Monday");
    editor.submit();
    editor.title.edit("Tuesday");
    editor.title.edit("Monday");
    editor.accept("Monday", 1, 12);
    assert!(!editor.title.dirty());
}

#[test]
fn invalid_drafts_remain_visible_and_cross_field_validation_is_scoped() {
    let editor = Editor::new();
    editor.count.edit("-");
    assert!(editor.count.parsed().is_err());
    assert_eq!(editor.count.message(), "");
    assert!(matches!(editor.form.prepare(), Err(FormError::Invalid)));
    assert_eq!(editor.count.raw(), "-");
    assert!(!editor.count.message().is_empty());
    assert_eq!(editor.backend.counts().started, 0);
    editor.count.edit("2");
    editor.title.edit("restricted");
    let form = editor.form.clone().validate(|(title, count)| {
        if title == "restricted" && *count > 1 {
            Err("Only one allowed".into())
        } else {
            Ok(())
        }
    });
    assert!(matches!(form.prepare(), Err(FormError::Invalid)));
    assert_eq!(form.validation_message(), "Only one allowed");
    editor.count.edit("1");
    assert_eq!(form.validation_message(), "");
    let snapshot = form.prepare().unwrap();
    assert_eq!(snapshot.command().title, "restricted");
    assert_eq!(snapshot.command().count, 1);
    assert_eq!(snapshot.command().id, 7);
    assert_eq!(snapshot.command().expected, 10);
}

#[test]
fn preparation_detects_reentrant_builder_and_validator_edits() {
    let editor = Editor::new();
    let title = editor.title.clone();
    let form = Form::new(
        &editor.owner.handle(),
        "7",
        (editor.title.clone(),),
        move |(value,)| {
            title.edit("changed");
            value
        },
    )
    .unwrap();
    assert!(matches!(
        form.prepare(),
        Err(FormError::ChangedDuringPreparation)
    ));
    let count = editor.count.clone();
    let form = editor.form.clone().validate(move |_| {
        count.edit("3");
        Ok(())
    });
    assert!(matches!(
        form.prepare(),
        Err(FormError::ChangedDuringPreparation)
    ));
}

#[test]
fn stale_foreign_and_reconfigured_snapshots_do_not_dispatch() {
    let editor = Editor::new();
    let snapshot = editor.form.prepare().unwrap();
    editor.title.edit("newer");
    let stale = editor
        .form
        .submit(&editor.save, snapshot, editor.mapping())
        .unwrap_err();
    assert_eq!(stale.reason, FormError::StaleSnapshot);
    assert_eq!(
        std::error::Error::source(&stale).and_then(|cause| cause.downcast_ref::<FormError>()),
        Some(&FormError::StaleSnapshot)
    );
    let other = Editor::new();
    assert_eq!(
        editor
            .form
            .submit(
                &editor.save,
                other.form.prepare().unwrap(),
                editor.mapping()
            )
            .unwrap_err()
            .reason,
        FormError::WrongForm
    );
    let snapshot = editor.form.prepare().unwrap();
    let _field = editor.title.clone().validate(|_| Ok(()));
    assert_eq!(
        editor
            .form
            .submit(&editor.save, snapshot, editor.mapping())
            .unwrap_err()
            .reason,
        FormError::StaleSnapshot
    );
    assert_eq!(editor.backend.counts().started, 0);
    assert!(matches!(
        Form::new(
            &editor.owner.handle(),
            "7",
            (editor.title.clone(), editor.title.clone()),
            |_| ()
        ),
        Err(FormError::DuplicateField)
    ));
}

#[test]
fn every_invalid_mapping_aborts_the_entire_local_publication() {
    for expected in [
        FormError::UnknownField,
        FormError::DuplicateMapping,
        FormError::VersionRegression,
        FormError::StaleBaseline,
        FormError::Mapping("wrong response entity".into()),
    ] {
        let editor = Editor::new();
        editor.title.edit("Friday");
        let after = editor.submit_invalid_mapping(expected.clone());
        editor.executor.run_until_stalled();
        editor.complete(Outcome::Accepted(Saved {
            title: "Friday".into(),
            count: 9,
            version: 11,
        }));
        assert_eq!(
            editor.title.baseline(),
            if expected == FormError::StaleBaseline {
                "reviewed"
            } else {
                "Thursday"
            }
        );
        assert_eq!(editor.count.baseline(), "1");
        assert_eq!(editor.version.get(), 10);
        assert_eq!(editor.form.status(), SubmissionStatus::PublicationFailed);
        assert_eq!(editor.save.state().status, Status::PublicationFailed);
        assert_eq!(editor.form.publication_error(), Some(expected.clone()));
        assert_eq!(
            editor.save.state().publication_error,
            Some(expected.clone())
        );
        assert!(!after.get());
        assert_eq!(
            editor
                .form
                .submit(
                    &editor.save,
                    editor.form.prepare().unwrap(),
                    editor.mapping()
                )
                .unwrap_err()
                .reason,
            FormError::Unresolved
        );
    }
}

#[test]
fn rejection_errors_expire_when_any_submitted_dependency_changes() {
    let editor = Editor::new();
    editor.title.edit("Friday");
    let title = editor.title.clone();
    editor
        .form
        .submit_with(
            &editor.save,
            editor.form.prepare().unwrap(),
            editor.mapping(),
            move |errors, error| {
                errors.field(&title, error);
                errors.form("Review both fields");
            },
        )
        .unwrap();
    editor.executor.run_until_stalled();
    editor.complete(Outcome::Rejected("Reserved title".into()));
    assert_eq!(editor.title.message(), "Reserved title");
    assert_eq!(editor.form.validation_message(), "Review both fields");
    editor.count.edit("2");
    assert_eq!(editor.title.message(), "");
    assert_eq!(editor.form.validation_message(), "");
    editor.submit();
    editor.title.edit("Monday");
    editor.complete(Outcome::Rejected("Old error".into()));
    assert_eq!(editor.title.raw(), "Monday");
    assert_eq!(editor.title.message(), "");
}

#[test]
fn acceptance_does_not_erase_newer_cross_field_validation() {
    let editor = Editor::new();
    let form = editor.form.clone().validate(|(title, _)| {
        if title == "invalid" {
            Err("Newer validation".into())
        } else {
            Ok(())
        }
    });
    editor.title.edit("Friday");
    editor.submit();
    editor.title.edit("invalid");
    assert!(matches!(form.prepare(), Err(FormError::Invalid)));
    editor.accept("Friday", 1, 11);
    assert_eq!(form.validation_message(), "Newer validation");
}

#[test]
fn newer_validation_of_unchanged_values_survives_an_older_acknowledgment() {
    let editor = Editor::new();
    editor.submit();
    let form = editor
        .form
        .clone()
        .validate(|_| Err("Updated validation policy".into()));
    assert!(matches!(form.prepare(), Err(FormError::Invalid)));
    editor.accept("Thursday", 1, 11);
    assert_eq!(form.validation_message(), "Updated validation policy");
}

#[test]
fn normalization_expires_validation_of_the_replaced_raw_value() {
    let editor = Editor::new();
    editor.title.edit(" Friday ");
    editor.submit();
    let form = editor.form.clone().validate(|(title, _)| {
        if title.starts_with(' ') {
            Err("Leading space".into())
        } else {
            Ok(())
        }
    });
    assert!(matches!(form.prepare(), Err(FormError::Invalid)));
    editor.accept("Friday", 1, 11);
    assert_eq!(editor.title.raw(), "Friday");
    assert_eq!(form.validation_message(), "");
}

#[test]
fn conflict_and_unknown_retain_the_command_until_explicit_reconciliation() {
    for outcome in [
        Outcome::Conflict("version mismatch".into()),
        Outcome::Unknown("connection lost".into()),
    ] {
        let editor = Editor::new();
        editor.title.edit("Friday");
        editor.submit();
        editor.complete(outcome);
        assert!(editor.form.status().unresolved());
        assert!(editor.save.state().status.unresolved());
        assert_eq!(
            editor.save.state().submission.unwrap().command.title,
            "Friday"
        );
        assert_eq!(
            editor
                .form
                .submit(
                    &editor.save,
                    editor.form.prepare().unwrap(),
                    editor.mapping()
                )
                .unwrap_err()
                .reason,
            FormError::Unresolved
        );
        assert_eq!(editor.backend.counts().started, 1);
        assert_eq!(editor.title.baseline(), "Thursday");
        let other = Editor::new();
        assert_eq!(
            editor.form.reconcile(&other.save),
            Err(FormError::WrongAction)
        );
        // Application reviews authoritative data, then explicitly unlocks saves.
        editor.title.reset("Friday".into());
        editor.version.set(11);
        editor.form.reconcile(&editor.save).unwrap();
        assert_eq!(editor.save.state().status, Status::Idle);
        assert!(editor.save.state().submission.is_none());
        editor.submit();
        assert_eq!(editor.save.state().submission.unwrap().command.expected, 11);
    }
}

#[test]
fn prepared_owner_has_no_io_and_disposal_releases_futures_at_executor_boundary() {
    let editor = Editor::new();
    let prepared = Owner::child(&editor.owner.handle());
    let backend = editor.backend.clone();
    let save = Action::new(
        &prepared.handle(),
        move |command, context| {
            let future = backend.load(command, context);
            async move { future.await.unwrap() }
        },
        editor.executor.spawner(),
    );
    let snapshot = editor.form.prepare().unwrap();
    let id = snapshot.id();
    let denied = editor
        .form
        .submit(&save, snapshot, editor.mapping())
        .unwrap_err();
    assert_eq!(denied.reason, FormError::NotActive);
    assert_eq!(denied.snapshot.id(), id);
    assert_eq!(editor.form.status(), SubmissionStatus::Idle);
    prepared.commit();
    editor
        .form
        .submit(&save, denied.snapshot, editor.mapping())
        .unwrap();
    editor.executor.run_until_stalled();
    let pending = editor.backend.next_request().unwrap();
    assert_eq!(editor.backend.counts().live, 1);
    save.dispose();
    assert!(pending.is_cancelled());
    assert_eq!(editor.form.status(), SubmissionStatus::Disposed);
    assert_eq!(editor.backend.counts().live, 1);
    editor.executor.run_until_stalled();
    assert_eq!(editor.backend.counts().live, 0);
    assert!(
        pending
            .complete(Ok(Outcome::Accepted(Saved {
                title: "late".into(),
                count: 1,
                version: 99
            })))
            .is_err()
    );
    assert_eq!(editor.title.raw(), "Thursday");
    assert_eq!(editor.version.get(), 10);
}

#[test]
fn retained_session_survives_view_disposal_but_not_application_disposal() {
    let editor = Editor::new();
    let view = Owner::child(&editor.owner.handle());
    view.commit();
    editor.title.edit("Friday");
    editor.submit();
    view.dispose();
    editor.accept("Friday", 1, 11);
    assert_eq!(editor.version.get(), 11);
    editor.title.edit("Monday");
    editor.submit();
    let request = editor.backend.next_request().unwrap();
    request
        .complete(Ok(Outcome::Accepted(Saved {
            title: "Monday".into(),
            count: 1,
            version: 12,
        })))
        .unwrap();
    editor.owner.dispose();
    editor.executor.run_until_stalled();
    assert_eq!(editor.form.status(), SubmissionStatus::Disposed);
    assert_eq!(editor.save.state().status, Status::Disposed);
    assert_eq!(editor.version.get(), 11);
}

#[test]
fn reentrant_mapping_is_busy_and_after_commit_can_start_the_next_save() {
    let editor = Editor::new();
    editor.title.edit("Friday");
    let save = editor.save.clone();
    let form = editor.form.clone();
    let mapping = editor.mapping();
    let next_mapping = editor.mapping();
    let version = editor.version.clone();
    let title = editor.title.clone();
    editor
        .form
        .submit(
            &editor.save,
            editor.form.prepare().unwrap(),
            move |ack, saved| {
                assert_eq!(
                    save.dispatch(Command {
                        id: 7,
                        expected: 10,
                        title: "wrong".into(),
                        count: 1
                    })
                    .unwrap_err()
                    .reason,
                    AdmissionError::Busy
                );
                mapping(ack, saved);
                ack.after_commit(move || {
                    assert_eq!(version.get(), 11);
                    assert_eq!(form.status(), SubmissionStatus::Accepted);
                    assert_eq!(save.state().status, Status::Accepted);
                    title.edit("Monday");
                    form.submit(&save, form.prepare().unwrap(), next_mapping)
                        .unwrap();
                });
            },
        )
        .unwrap();
    editor.executor.run_until_stalled();
    editor.accept("Friday", 1, 11);
    assert_eq!(editor.save.state().status, Status::Pending);
    assert_eq!(editor.backend.counts().started, 2);
    editor.accept("Monday", 1, 12);
    assert_eq!(editor.version.get(), 12);
    assert_eq!(editor.backend.counts().cancelled, 0);
}

#[test]
fn disposal_during_mapping_suppresses_every_staged_write() {
    let editor = Editor::new();
    let owner = editor.owner.clone();
    let mapping = editor.mapping();
    editor
        .form
        .submit(
            &editor.save,
            editor.form.prepare().unwrap(),
            move |ack, saved| {
                mapping(ack, saved);
                owner.dispose();
            },
        )
        .unwrap();
    editor.executor.run_until_stalled();
    editor.accept("late", 2, 11);
    assert_eq!(editor.version.get(), 10);
    assert_eq!(editor.title.raw(), "Thursday");
    assert_eq!(editor.form.status(), SubmissionStatus::Disposed);
}

struct OnDrop(Option<Box<dyn FnOnce()>>);
impl Drop for OnDrop {
    fn drop(&mut self) {
        if let Some(callback) = self.0.take() {
            callback();
        }
    }
}

#[test]
fn unused_mapper_captures_drop_before_acknowledgment_validation() {
    let editor = Editor::new();
    let title = editor.title.clone();
    let guard = OnDrop(Some(Box::new(move || title.reset("Reviewed".into()))));
    editor
        .form
        .submit_with(
            &editor.save,
            editor.form.prepare().unwrap(),
            editor.mapping(),
            move |_, _| {
                let _ = &guard;
            },
        )
        .unwrap();
    editor.executor.run_until_stalled();
    editor.accept("Stale", 2, 11);
    assert_eq!(editor.title.raw(), "Reviewed");
    assert_eq!(editor.title.baseline(), "Reviewed");
    assert_eq!(editor.version.get(), 10);
    assert_eq!(editor.form.status(), SubmissionStatus::PublicationFailed);
}

#[test]
fn discarded_mappings_cannot_revive_a_disposed_separate_form_owner() {
    let editor = Editor::new();
    let owner = Rc::new(Owner::child(&editor.owner.handle()));
    owner.commit();
    let form = Form::new(
        &owner.handle(),
        "project:7",
        (editor.title.clone(), editor.count.clone()),
        |(title, count)| Command {
            id: 7,
            expected: 10,
            title,
            count,
        },
    )
    .unwrap();
    let guard = OnDrop(Some(Box::new(move || owner.dispose())));
    let title = editor.title.clone();
    form.submit(&editor.save, form.prepare().unwrap(), move |ack, saved| {
        ack.field(&title, saved.title.clone());
        ack.field(&title, "Duplicate".into());
        ack.after_commit(move || drop(guard));
    })
    .unwrap();
    editor.executor.run_until_stalled();
    editor.accept("Stale", 2, 11);
    assert_eq!(form.status(), SubmissionStatus::Disposed);
    assert_eq!(editor.title.baseline(), "Thursday");
    assert_eq!(editor.save.state().status, Status::PublicationFailed);
}

#[test]
fn replaced_user_values_drop_only_after_fields_version_and_status_agree() {
    let editor = Editor::new();
    let mapping = editor.mapping();
    let title = editor.title.clone();
    let version = editor.version.clone();
    let form = editor.form.clone();
    let save = editor.save.clone();
    let dropped = Rc::new(Cell::new(false));
    let called = dropped.clone();
    let arbitrary = signal(OnDrop(Some(Box::new(move || {
        assert_eq!(title.baseline(), "Friday");
        assert_eq!(version.get(), 11);
        assert_eq!(form.status(), SubmissionStatus::Accepted);
        assert_eq!(save.state().status, Status::Accepted);
        called.set(true);
    }))));
    editor
        .form
        .submit(
            &editor.save,
            editor.form.prepare().unwrap(),
            move |ack, saved| {
                mapping(ack, saved);
                ack.signal(&arbitrary, OnDrop(None));
            },
        )
        .unwrap();
    editor.executor.run_until_stalled();
    editor.accept("Friday", 1, 11);
    assert!(dropped.get());
}
