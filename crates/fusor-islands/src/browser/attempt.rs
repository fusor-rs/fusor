//! One activation attempt. Pending previews own prepared scopes, not visible DOM.
use crate::attributes;
use fusor::{
    Effect,
    dom::{Scope, delivery::PreviewReadiness},
    effect, untrack,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::JsValue;
use web_sys::Element;

pub(super) enum Prepared {
    /// Adopted the server-rendered root in place, so it commits at once.
    Attach(Scope),
    /// Built detached; replaces the server-rendered preview once its initial
    /// coherent regions are ready.
    Preview(Scope, Preview),
}

impl Prepared {
    fn scope(&self) -> &Scope {
        match self {
            Self::Attach(scope) | Self::Preview(scope, _) => scope,
        }
    }
}

/// The server-rendered preview a candidate replaces, and the registration it
/// was prepared for.
pub(super) struct Preview {
    host: Element,
    initial: Element,
    readiness: PreviewReadiness,
    attributes: Vec<(&'static str, Option<String>)>,
    props: Element,
    text: String,
}

impl Preview {
    pub(super) fn new(
        host: &Element,
        initial: Element,
        readiness: PreviewReadiness,
        text: &str,
    ) -> Result<Self, JsValue> {
        let props = host
            .query_selector(&format!(":scope > {}", attributes::PROPS_SCRIPT))?
            .ok_or_else(|| JsValue::from_str("missing island props"))?;
        let attributes = attributes::METADATA
            .into_iter()
            .map(|name| (name, host.get_attribute(name)))
            .collect();
        Ok(Self {
            host: host.clone(),
            initial,
            readiness,
            attributes,
            props,
            text: text.into(),
        })
    }

    /// The registry's own validity check, repeated after the asynchronous
    /// wait: the host still carries the same registration and props.
    fn valid(&self) -> bool {
        self.host.is_connected()
            && self.initial.parent_element().as_ref() == Some(&self.host)
            && self.props.parent_element().as_ref() == Some(&self.host)
            && self
                .props
                .matches(attributes::PROPS_SCRIPT)
                .unwrap_or(false)
            && self.props.text_content().as_deref() == Some(self.text.as_str())
            && self
                .attributes
                .iter()
                .all(|(name, value)| self.host.get_attribute(name) == *value)
    }

    /// Replace the preview with the prepared `scope` and activate it.
    fn publish(&self, scope: &Scope) -> Result<(), JsValue> {
        if !self.valid() || scope.owner().is_disposed() {
            return Err(JsValue::from_str(
                "preview registration changed while loading",
            ));
        }
        self.host
            .append_child(scope.root().expect("island entries have one native root"))?;
        // Fallible setup runs while the native fallback is still owned by its
        // host; failure removes the candidate and leaves that fallback intact.
        scope.finish_prepare()?;
        if !self.valid() {
            return Err(JsValue::from_str(
                "preview registration changed during setup",
            ));
        }
        self.initial.remove();
        let result = scope.try_commit();
        if let Err(commit) = &result {
            // Removing the host also removes the need for its native fallback.
            if !self.host.is_connected() {
                return result;
            }
            if let Err(restoration) = self.host.insert_before(
                &self.initial,
                Some(scope.root().expect("island entries have one native root")),
            ) {
                let error = js_sys::Error::new(&format!(
                    "island commit failed: {commit:?}; fallback restoration failed: {restoration:?}"
                ));
                error.set_cause(&js_sys::Array::of2(commit, &restoration));
                return Err(error.into());
            }
        }
        result
    }
}

pub(super) struct Attempt {
    prepared: Prepared,
    promise: js_sys::Promise,
    resolve: js_sys::Function,
    reject: js_sys::Function,
    driver: RefCell<Option<Effect>>,
    scheduled: Cell<bool>,
    done: Cell<bool>,
}

impl Attempt {
    pub(super) fn new(prepared: Prepared) -> Rc<Self> {
        let mut callbacks = None;
        let promise =
            js_sys::Promise::new(&mut |resolve, reject| callbacks = Some((resolve, reject)));
        let (resolve, reject) = callbacks.expect("Promise executor is synchronous");
        Rc::new(Self {
            prepared,
            promise,
            resolve,
            reject,
            driver: RefCell::new(None),
            scheduled: Cell::new(false),
            done: Cell::new(false),
        })
    }

    pub(super) fn promise(&self) -> js_sys::Promise {
        self.promise.clone()
    }

    pub(super) fn start(self: &Rc<Self>) {
        let Prepared::Preview(_, preview) = &self.prepared else {
            self.finish(self.publish());
            return;
        };
        let readiness = preview.readiness.clone();
        let weak = Rc::downgrade(self);
        let driver = effect(move || {
            let Some(attempt) = weak.upgrade().filter(|attempt| !attempt.done.get()) else {
                return;
            };
            // Poll on every run: it subscribes this effect to readiness.
            if matches!(readiness.poll(), Ok(false)) {
                return;
            }
            if attempt.scheduled.replace(true) {
                return;
            }
            let (weak, readiness) = (weak.clone(), readiness.clone());
            // Let structural preparation and reactive propagation settle. Recheck
            // readiness and identity before publishing; no user callbacks run in
            // the gap between native inspection and initial DOM replacement.
            wasm_bindgen_futures::spawn_local(async move {
                let Some(attempt) = weak.upgrade().filter(|attempt| !attempt.done.get()) else {
                    return;
                };
                attempt.scheduled.set(false);
                match untrack(|| readiness.poll()) {
                    Ok(false) => {}
                    Ok(true) => attempt.finish(attempt.publish()),
                    Err(error) => attempt.finish(Err(JsValue::from_str(&format!(
                        "initial coherent view failed: {error}"
                    )))),
                }
            });
        });
        *self.driver.borrow_mut() = Some(driver);
    }

    /// Make the prepared view visible and active.
    fn publish(&self) -> Result<(), JsValue> {
        match &self.prepared {
            Prepared::Attach(scope) => scope.try_commit(),
            Prepared::Preview(scope, preview) => preview.publish(scope),
        }
    }

    fn finish(&self, result: Result<(), JsValue>) {
        if self.done.replace(true) {
            return;
        }
        self.driver.borrow_mut().take();
        if let Prepared::Preview(_, preview) = &self.prepared {
            preview.readiness.close();
        }
        match result {
            Ok(()) => {
                let _ = self.resolve.call0(&JsValue::UNDEFINED);
            }
            Err(error) => {
                self.prepared.scope().dispose();
                // An attached root is the server's DOM, which the page keeps.
                // A preview candidate was never shown in its place.
                if let Prepared::Preview(scope, _) = &self.prepared {
                    scope
                        .root()
                        .expect("island entries have one native root")
                        .remove();
                }
                let _ = self.reject.call1(&JsValue::UNDEFINED, &error);
            }
        }
    }

    pub(super) fn dispose(&self) {
        if self.done.get() {
            self.prepared.scope().dispose();
        } else {
            self.finish(Err(JsValue::from_str("island activation was cancelled")));
        }
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        self.dispose();
    }
}
