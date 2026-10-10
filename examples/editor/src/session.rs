//! One entity, one explicitly retained owner, one editable session. Route views
//! share this state but never own its pending save. No automatic query rebasing.
use crate::api::{self, Project, SaveError, UpdateProject};
use fusor::prelude::*;
use fusor_std::{
    actions::{Action, Status, browser::action},
    forms::{Form, FormError, TextField},
    query::QueryClient,
};
use std::rc::Rc;
use wasm_bindgen::JsValue;

pub type Projects = QueryClient<u64, Project, String>;
fn admission_message(error: &FormError) -> &'static str {
    match error {
        FormError::Invalid => "Review the highlighted fields before saving.",
        FormError::Busy => "A save is already running. Your newer edits remain in the draft.",
        FormError::StaleSnapshot | FormError::ChangedDuringPreparation => {
            "The draft changed while preparing. Save again."
        }
        FormError::Unresolved => "Review the previous save outcome before saving again.",
        FormError::Disposed | FormError::NotActive => "This editing session is unavailable.",
        _ => "Unable to prepare this save for the current session.",
    }
}
type ProjectForm = Form<(TextField<String>, TextField<String>, TextField<u32>), UpdateProject>;
pub struct Session {
    pub title: TextField<String>,
    pub body: TextField<String>,
    pub seats: TextField<u32>,
    pub version: Signal<u64>,
    pub form: ProjectForm,
    pub save: Action<UpdateProject, Project, SaveError>,
    pub notice: Signal<String>,
    pub id: u64,
    // A weak OwnerHandle would not retain this lifetime.
    _owner: Owner,
}
impl Session {
    pub fn new(parent: &OwnerHandle, project: &Project) -> Result<Rc<Self>, JsValue> {
        let owner = Owner::child(parent);
        let title = TextField::new(project.title.clone()).validate(|value| {
            if value.trim().is_empty() {
                Err("Enter a project title".into())
            } else {
                Ok(())
            }
        });
        let body = TextField::new(project.body.clone());
        let seats = TextField::new(project.seats).validate(|value| {
            if *value == 0 {
                Err("Choose at least one seat".into())
            } else {
                Ok(())
            }
        });
        let version = signal(project.version);
        let current = version.clone();
        let id = project.id;
        // This demo protocol uses a per-session nonce plus local revisions as an
        // operation identity. It is not an authentication token or retry policy.
        let nonce = format!("{}-{}", js_sys::Date::now(), js_sys::Math::random());
        let revisions = (title.clone(), body.clone(), seats.clone());
        let form = Form::new(
            &owner.handle(),
            format!("project:{id}"),
            (title.clone(), body.clone(), seats.clone()),
            move |(title, body, seats)| {
                let expected_version = current.get_untracked();
                UpdateProject {
                    id,
                    expected_version,
                    title,
                    body,
                    seats,
                    operation: format!(
                        "{nonce}/{expected_version}/{}/{}/{}",
                        revisions.0.revision(),
                        revisions.1.revision(),
                        revisions.2.revision()
                    ),
                }
            },
        )?;
        let save = action(&owner.handle(), api::save);
        owner.commit();
        Ok(Rc::new(Self {
            title,
            body,
            seats,
            version,
            form,
            save,
            notice: signal(String::new()),
            id,
            _owner: owner,
        }))
    }
    pub fn submit(&self, projects: Projects) {
        self.notice.set(String::new());
        let snapshot = match self.form.prepare() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.notice.set(admission_message(&error).into());
                return;
            }
        };
        let title = self.title.clone();
        let body = self.body.clone();
        let seats = self.seats.clone();
        let version = self.version.clone();
        let id = self.id;
        let invalid_title = self.title.clone();
        let invalid_body = self.body.clone();
        let invalid_seats = self.seats.clone();
        let admitted = self.form.submit_with(
            &self.save,
            snapshot,
            move |ack, saved| {
                if saved.id != id {
                    ack.reject("Unexpected saved project identity");
                    return;
                }
                ack.field(&title, saved.title.clone());
                ack.field(&body, saved.body.clone());
                ack.field(&seats, saved.seats);
                ack.version(&version, saved.version);
                ack.after_commit(move || {
                    projects.invalidate(&id);
                });
            },
            move |errors, error| match error.field.as_deref() {
                Some("title") => errors.field(&invalid_title, &error.message),
                Some("body") => errors.field(&invalid_body, &error.message),
                Some("seats") => errors.field(&invalid_seats, &error.message),
                _ => errors.form(&error.message),
            },
        );
        if let Err(error) = admitted {
            self.notice.set(admission_message(&error.reason).into());
        }
    }
    /// User explicitly discards the draft after reviewing the displayed remote
    /// value. An unknown write needs positive evidence of its operation identity;
    /// an arbitrary later read alone cannot prove that the operation finished.
    pub fn reload_reviewed(&self, remote: &Project) {
        if self.save.pending() {
            self.notice
                .set("Wait for the active save before reloading".into());
            return;
        }
        if remote.id != self.id || remote.version < self.version.get_untracked() {
            self.notice
                .set("The displayed server result is older than this session".into());
            return;
        }
        let state = self.save.state();
        if state.status == Status::Unknown
            && !state.submission.as_ref().is_some_and(|submission| {
                remote.last_operation.as_deref() == Some(submission.command.operation.as_str())
            })
        {
            self.notice.set(
                "This server result does not establish the unknown operation's outcome".into(),
            );
            return;
        }
        batch(|| {
            self.title.reset(remote.title.clone());
            self.body.reset(remote.body.clone());
            self.seats.reset(remote.seats);
            self.version.set(remote.version);
            if state.submission.is_some() {
                if let Err(error) = self.form.reconcile(&self.save) {
                    self.notice.set(error.to_string());
                    return;
                }
            }
            self.notice
                .set("Reviewed server version loaded; draft discarded".into());
        });
    }
}
