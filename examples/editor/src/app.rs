use crate::{
    api,
    session::{Projects, Session},
    views::{Away, EditorPage},
};
use fusor::prelude::*;
use fusor_std::query::{Freshness, Query, QueryOptions, QueryState, browser::client};
use std::{cell::Cell, num::NonZeroUsize, rc::Rc, time::Duration};
use wasm_bindgen::JsValue;

pub struct Store {
    pub projects: Projects,
    pub remote: Query<u64, api::Project, String>,
    pub session: Signal<Option<Rc<Session>>>,
    pub notice: Signal<String>,
    owner: OwnerHandle,
}
pub struct Editing;
impl ContextKey for Editing {
    type Value = Rc<Store>;
}
impl Store {
    pub fn open(&self) {
        if self.session.with_untracked(Option::is_some) {
            return;
        }
        if let Some(data) = self.remote.get().data() {
            match Session::new(&self.owner, &data.value) {
                Ok(session) => {
                    let retired = self.session.update(|current| current.replace(session));
                    drop(retired);
                }
                Err(error) => self.notice.set(format!("{error:?}")),
            }
        }
    }
    pub fn close(&self) {
        let retired = self.session.update(Option::take);
        drop(retired);
    }
    pub fn remote_text(&self) -> String {
        let state = self.remote.get();
        if let Some(data) = state.data() {
            format!("{} · version {}", data.value.title, data.value.version)
        } else {
            match state {
                QueryState::Error { error, .. } => format!("Unable to load project: {error}"),
                QueryState::Disposed => "Application closed".into(),
                QueryState::Capacity { .. } => "Project unavailable".into(),
                _ => "Loading project…".into(),
            }
        }
    }
    pub fn context(owner: &OwnerHandle) -> Result<Rc<Self>, JsValue> {
        owner
            .context::<Editing>()
            .map(|store| (*store).clone())
            .ok_or_else(|| JsValue::from_str("Missing application editing store"))
    }
}
pub(crate) struct App {
    store: Rc<Store>,
    _initialize: Effect,
}
impl App {
    fn new(owner: OwnerHandle) -> Result<Self, JsValue> {
        let projects = client(
            &owner,
            QueryOptions {
                freshness: Freshness::For(Duration::from_secs(60)),
                retention: Duration::from_secs(120),
                capacity: NonZeroUsize::MIN,
            },
            api::load,
        );
        let remote = projects.observe(&owner, || Some(7));
        let store = Rc::new(Store {
            projects,
            remote,
            session: signal(None),
            notice: signal(String::new()),
            owner: owner.clone(),
        });
        owner
            .provide::<Editing>(store.clone())
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let captured = store.clone();
        let initialized = Cell::new(false);
        let initialize = effect(move || {
            if !initialized.get() && captured.remote.get().data().is_some() {
                initialized.set(true);
                captured.open();
            }
        });
        Ok(Self {
            store,
            _initialize: initialize,
        })
    }
}
fusor::bindings!(app);
