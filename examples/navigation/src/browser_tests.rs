//! Optional browser-test driver; application code needs no JavaScript exports.
use fusor::dom::application;
use fusor_router::browser::{NavigateOptions, Router, declarative::Navigation, mount_outlet};
use std::cell::{Cell, RefCell};
use wasm_bindgen::prelude::*;

use fusor::{
    OwnerHandle,
    dom::{Component, ComponentFactory, Scope},
};

struct StartupProbe(u8);

thread_local! { static TYPED: Cell<bool> = const { Cell::new(false) }; }
impl Component for StartupProbe {
    fn mount(self) -> Result<Scope, JsValue> {
        Self::try_mount_with(|_| Ok(self))
    }
    fn prepare_component(
        parent: Option<&OwnerHandle>,
        make: ComponentFactory<'_, Self>,
    ) -> Result<Scope, JsValue> {
        let root = fusor::dom::document()?.create_element("main")?;
        let mut scope = Scope::new(root.clone());
        scope.prepare_owner(parent);
        fusor_router::browser::mount_outlet(
            &mut scope,
            &root,
            env!("FUSOR_BASE_PATH"),
            crate::app::render_page,
        )?;
        let probe = make(scope.owner())?;
        match probe.0 {
            0 => return Err(JsValue::from_str("expected preparation failure")),
            1 => scope.before_commit(|| Err(JsValue::from_str("expected commit failure")))?,
            _ => {
                let registration = scope.owner().on_activate(|| {
                    assert!(
                        application::owner().is_some(),
                        "root retained before activation"
                    );
                    assert!(start().is_err(), "reentrant start rejected");
                    assert!(unmount().is_err(), "reentrant teardown rejected");
                });
                scope.retain(registration);
            }
        }
        Ok(scope)
    }
}

#[wasm_bindgen]
pub fn probe_startup(mode: u8) -> Result<(), JsValue> {
    application::mount(|_| Ok(StartupProbe(mode)))
}

/// Recommitting an active root must still prepare newly ready descendants.
#[wasm_bindgen]
pub fn probe_commit_queue() -> Result<(), JsValue> {
    use std::{cell::Cell, rc::Rc};
    let document = fusor::dom::document()?;
    let mut root = Scope::new(document.create_element("div")?);
    root.prepare_owner(None);
    root.try_commit()?;
    root.try_commit()?;

    let mut child = Scope::new(document.create_element("div")?);
    child.prepare_owner(Some(&root.owner()));
    let mut descendant = Scope::new(document.create_element("div")?);
    descendant.prepare_owner(Some(&child.owner()));
    let prepared = Rc::new(Cell::new(0));
    let calls = prepared.clone();
    descendant.before_commit(move || {
        calls.set(calls.get() + 1);
        Ok(())
    })?;
    descendant.finish_prepare()?; // Inactive parent defers the ready action.
    assert_eq!(prepared.get(), 0);
    root.try_commit()?;
    assert_eq!(prepared.get(), 1);
    assert!(!descendant.owner().is_active());
    child.try_commit()?;
    descendant.try_commit()?;
    assert!(descendant.owner().is_active());
    root.try_commit()?;
    assert_eq!(prepared.get(), 1);

    drop(root);
    assert!(child.try_commit().is_err());
    Ok(())
}

#[wasm_bindgen]
pub fn start() -> Result<(), JsValue> {
    if TYPED.get() {
        application::mount_scope(|| {
            let root = fusor::dom::document()?
                .query_selector("#outlet")?
                .ok_or_else(|| JsValue::from_str("missing outlet"))?;
            let mut scope = Scope::new(root.clone());
            scope.prepare_owner(None);
            mount_outlet(
                &mut scope,
                &root,
                crate::routes::BASE,
                crate::app::render_page,
            )?;
            Ok(scope)
        })
    } else {
        crate::app::__fusor_mount()
    }
}

/// Run the same real pages and transport tests through the typed outlet.
#[wasm_bindgen]
pub fn use_typed_router() -> Result<(), JsValue> {
    unmount()?;
    TYPED.set(true);
    start()
}

#[wasm_bindgen]
pub fn navigate(url: &str, replace: bool) -> Result<(), JsValue> {
    let owner =
        application::owner().ok_or_else(|| JsValue::from_str("application is unmounted"))?;
    let options = NavigateOptions {
        replace,
        keep_focus: true,
        keep_scroll: true,
    };
    if let Some(router) = Router::<crate::routes::Page>::from_owner(&owner) {
        router.navigate_url(url, options)
    } else {
        Navigation::from_owner(&owner)
            .ok_or_else(|| JsValue::from_str("missing navigation"))?
            .navigate_url(url, options)
    }
}

#[wasm_bindgen]
pub fn unmount() -> Result<(), JsValue> {
    application::unmount()
}

// A route is allowed to include query and fragment in its identity. Keep a
// surviving handle to check that parent disposal still releases history ownership.
#[derive(Clone, PartialEq)]
struct ExactRoute(fusor_router::AppUrl);
impl fusor_router::Route for ExactRoute {
    fn parse(url: &fusor_router::AppUrl) -> Option<Self> {
        Some(Self(url.clone()))
    }
    fn path(&self) -> String {
        self.0.to_string()
    }
}
thread_local! {
    static EXACT: RefCell<Option<Router<ExactRoute>>> = const { RefCell::new(None) };
}

#[wasm_bindgen]
pub fn start_exact_router() -> Result<(), JsValue> {
    unmount()?;
    application::mount_scope(|| {
        let root = fusor::dom::document()?.query_selector("#outlet")?.unwrap();
        let mut scope = Scope::new(root.clone());
        scope.prepare_owner(None);
        // Use the direct mounting API as well as the mount_outlet path above.
        let router = Router::mount(&scope.owner(), crate::routes::BASE, root, |context| {
            let router = Router::<ExactRoute>::from_owner(&context.parent)
                .expect("router context exists during initial preparation");
            let url = context.location.get().url;
            let root = fusor::dom::document()?.create_element("section")?;
            root.set_inner_html("<h1>Exact route</h1><input id=exact-note><p id=end style='margin-top:2000px'>End</p>");
            root.set_attribute("data-url", &url.to_string())?;
            let mut scope = Scope::new(root);
            scope.prepare_owner(Some(&context.parent));
            let mode = url.query_first("mode").unwrap_or_default();
            if mode == "fail" || url.fragment() == "fail" {
                scope.before_commit(|| Err(JsValue::from_str("exact preparation failed")))?;
            } else if mode == "reenter" {
                scope.retain(scope.owner().on_activate(move || {
                    assert!(
                        router
                            .navigate_url("/reader/", NavigateOptions::default())
                            .is_err()
                    );
                }));
            } else if mode == "dispose" {
                scope.retain(scope.owner().on_activate(move || router.dispose()));
            }
            Ok(scope)
        })?;
        EXACT.with(|slot| *slot.borrow_mut() = Some(router));
        Ok(scope)
    })
}

#[wasm_bindgen]
pub fn exact_navigate(url: &str, keep: bool) -> Result<(), JsValue> {
    let router = EXACT.with(|slot| slot.borrow().clone()).unwrap();
    router.navigate_url(
        url,
        NavigateOptions {
            keep_focus: keep,
            keep_scroll: keep,
            ..Default::default()
        },
    )
}

#[wasm_bindgen]
pub fn exact_dispose() {
    let router = EXACT.with(|slot| slot.borrow().clone()).unwrap();
    router.dispose();
}

#[wasm_bindgen]
pub fn exact_last_error() -> Option<String> {
    EXACT.with(|slot| {
        slot.borrow()
            .as_ref()
            .expect("the exact router has been mounted")
            .last_error()
            .map(|error| format!("{error:?}"))
    })
}
