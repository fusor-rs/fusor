use crate::coherence::{AsyncBoundary, BoundaryStatus};
use crate::dom::{Listener, Scope, document};
use crate::effect;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, Event, HtmlElement};

#[derive(Clone)]
pub(super) struct BlockingOverlay {
    root: Element,
    state: Rc<RefCell<Overlay>>,
}

struct Overlay {
    authored: bool,
    applied: bool,
    focus: Option<HtmlElement>,
    user_moved: bool,
}

impl BlockingOverlay {
    pub(super) fn new(root: &Element) -> Self {
        Self {
            root: root.clone(),
            state: Rc::new(RefCell::new(Overlay {
                authored: root.has_attribute("inert"),
                applied: false,
                focus: None,
                user_moved: false,
            })),
        }
    }

    pub(super) fn install(
        &self,
        region: &mut Scope,
        boundary: AsyncBoundary,
    ) -> Result<(), JsValue> {
        let root = &self.root;
        let overlay = &self.state;
        // Capture focus intent outside the stale region, including a click on
        // an unfocusable element. Never steal focus after such an interaction.
        for name in ["focusin", "pointerdown"] {
            let captured = overlay.clone();
            let root = root.clone();
            let listener = Listener::new(document()?.into(), name, move |event: Event| {
                let outside = event
                    .target()
                    .and_then(|target| target.dyn_into::<web_sys::Node>().ok())
                    .is_some_and(|target| !root.contains(Some(&target)));
                if outside && captured.borrow().applied {
                    captured.borrow_mut().user_moved = true;
                }
            })?;
            region.listeners.push(listener);
        }
        let root = root.clone();
        let captured = overlay.clone();
        let status = effect(move || {
            let blocked = !matches!(
                boundary.status(),
                BoundaryStatus::Ready | BoundaryStatus::Disposed
            );
            let mut overlay = captured.borrow_mut();
            let mut restore = None;
            if blocked && !overlay.applied {
                overlay.authored = root.has_attribute("inert");
                overlay.focus = document()
                    .ok()
                    .and_then(|doc| doc.active_element())
                    .filter(|node| root.contains(Some(node)))
                    .and_then(|node| node.dyn_into::<HtmlElement>().ok());
                overlay.user_moved = false;
                overlay.applied = true;
                let _ = root.set_attribute("inert", "");
                let _ = root.set_attribute("aria-busy", "true");
            } else if !blocked && overlay.applied {
                if !overlay.authored {
                    let _ = root.remove_attribute("inert");
                }
                let _ = root.remove_attribute("aria-busy");
                let focus = overlay.focus.take();
                if !overlay.user_moved && !overlay.authored {
                    restore = focus;
                }
                overlay.applied = false;
            }
            // focus() dispatches application events synchronously.
            drop(overlay);
            if let Some(focus) = restore.filter(|node| node.is_connected()) {
                if document()
                    .ok()
                    .and_then(|doc| doc.active_element())
                    .is_none_or(|node| {
                        node.local_name() == "body" || node.is_same_node(Some(&focus))
                    })
                {
                    let _ = focus.focus();
                }
            }
        });
        region.effects.push(status);
        Ok(())
    }

    pub(super) fn owns_root(&self, node: &Element) -> bool {
        node.is_same_node(Some(&self.root))
    }

    pub(super) fn authored_inert(&self) -> bool {
        self.state.borrow().authored
    }

    pub(super) fn set_authored_inert(&self, authored: bool) -> Result<(), JsValue> {
        let mut overlay = self.state.borrow_mut();
        overlay.authored = authored;
        if overlay.authored || overlay.applied {
            self.root.set_attribute("inert", "")
        } else {
            self.root.remove_attribute("inert")
        }
    }
}
