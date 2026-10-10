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
        self.capture_focus_intent(region)?;
        let captured = self.clone();
        let status = effect(move || {
            let blocked = !matches!(
                boundary.status(),
                BoundaryStatus::Ready | BoundaryStatus::Disposed
            );
            if let Err(error) = captured.update(blocked) {
                web_sys::console::error_1(&error);
            }
        });
        region.effects.push(status);
        Ok(())
    }

    fn update(&self, blocked: bool) -> Result<(), JsValue> {
        let mut overlay = self.state.borrow_mut();
        if blocked {
            let authored = self.root.has_attribute("inert");
            let focus = if overlay.applied {
                None
            } else {
                document()?
                    .active_element()
                    .filter(|node| self.root.contains(Some(node)))
                    .and_then(|node| node.dyn_ref::<HtmlElement>().cloned())
            };
            self.root.set_attribute("inert", "")?;
            if !overlay.applied {
                overlay.authored = authored;
                overlay.focus = focus;
                overlay.user_moved = false;
                overlay.applied = true;
            }
            self.root.set_attribute("aria-busy", "true")?;
            return Ok(());
        }
        if !overlay.applied {
            return Ok(());
        }
        if !overlay.authored {
            self.root.remove_attribute("inert")?;
        }
        self.root.remove_attribute("aria-busy")?;
        let restore = overlay
            .focus
            .take()
            .filter(|_| !overlay.user_moved && !overlay.authored);
        overlay.applied = false;
        // focus() dispatches application events synchronously.
        drop(overlay);
        if let Some(focus) = restore.filter(|node| node.is_connected()) {
            if document()?
                .active_element()
                .is_none_or(|node| node.local_name() == "body" || node.is_same_node(Some(&focus)))
            {
                focus.focus()?;
            }
        }
        Ok(())
    }

    fn capture_focus_intent(&self, region: &mut Scope) -> Result<(), JsValue> {
        // An outside click on an unfocusable element also cancels focus restoration.
        for name in ["focusin", "pointerdown"] {
            let captured = Rc::clone(&self.state);
            let root = self.root.clone();
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
        if authored || overlay.applied {
            self.root.set_attribute("inert", "")?;
        } else {
            self.root.remove_attribute("inert")?;
        }
        overlay.authored = authored;
        Ok(())
    }
}
