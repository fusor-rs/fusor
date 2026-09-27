//! The browser side of `bind`. Generated code installs one control per bound
//! element; when hydrating, it first adopts the native edits made before the
//! page ran.
use super::{ElementTarget, InputTarget, JsValue, Scope};
use crate::bind::{Checkbox, TextValue, selected};
use crate::template::InputKind;
use crate::{Signal, untrack};
use std::{cell::Cell, rc::Rc, str::FromStr};
use wasm_bindgen::JsCast;
use web_sys::{
    Element, HtmlInputElement, HtmlOptionElement, HtmlSelectElement, HtmlTextAreaElement,
};

fn touch_on_blur(
    scope: &mut Scope,
    target: impl ElementTarget,
    value: &impl TextValue,
) -> Result<(), JsValue> {
    let value = value.clone();
    scope.on(target, "blur", move |_| value.touch())
}

/// A control whose `value` property holds the text the user edits.
#[derive(Clone)]
enum Text {
    Input(HtmlInputElement),
    TextArea(HtmlTextAreaElement),
}

impl Text {
    fn resolve(element: &Element) -> Result<Self, JsValue> {
        if let Some(input) = element.dyn_ref::<HtmlInputElement>() {
            let kind = input.type_();
            if InputKind::of(&kind) != InputKind::Text {
                return Err(JsValue::from_str(&format!(
                    "fusor: bind cannot edit an input of type {kind} as text"
                )));
            }
            Ok(Self::Input(input.clone()))
        } else if let Some(textarea) = element.dyn_ref::<HtmlTextAreaElement>() {
            Ok(Self::TextArea(textarea.clone()))
        } else {
            Err(JsValue::from_str(
                "fusor: bind requires an input or textarea element",
            ))
        }
    }

    fn value(&self) -> String {
        match self {
            Self::Input(input) => input.value(),
            Self::TextArea(textarea) => textarea.value(),
        }
    }

    fn set_value(&self, value: &str) {
        match self {
            Self::Input(input) => input.set_value(value),
            Self::TextArea(textarea) => textarea.set_value(value),
        }
    }
}

pub fn adopt_text(
    scope: &Scope,
    target: impl ElementTarget,
    value: &impl TextValue,
) -> Result<(), JsValue> {
    let text = Text::resolve(&target.resolve(scope)?)?.value();
    if !untrack(|| value.shows(&text)) {
        value.edit(text);
    }
    Ok(())
}

/// Text-like and numeric inputs, and textarea. Composition owns the visible
/// draft until it ends: writes do not interrupt the IME, and its final text wins.
pub fn text(
    scope: &mut Scope,
    target: impl ElementTarget,
    value: impl TextValue,
) -> Result<(), JsValue> {
    let element = target.resolve(scope)?;
    let control = Text::resolve(&element)?;
    let composing = Rc::new(Cell::new(false));
    let edit = Rc::new({
        let (value, control) = (value.clone(), control.clone());
        move || value.edit(control.value())
    });
    for (event, active) in [("compositionstart", true), ("compositionend", false)] {
        let (composing, edit) = (composing.clone(), edit.clone());
        scope.on(&element, event, move |_| {
            composing.set(active);
            edit();
        })?;
    }
    scope.on(&element, "input", move |_| edit())?;
    touch_on_blur(scope, &element, &value)?;
    scope.bind_dom(move || {
        // Read before checking composition so the effect keeps its dependency.
        if !value.shows(&control.value()) && !composing.get() {
            control.set_value(&value.text());
        }
        Ok(())
    })
}

fn select_element(scope: &Scope, target: impl ElementTarget) -> Result<HtmlSelectElement, JsValue> {
    target
        .resolve(scope)?
        .dyn_into()
        .map_err(|_| JsValue::from_str("fusor: bind requires a select element"))
}

fn options(select: &HtmlSelectElement) -> impl Iterator<Item = HtmlOptionElement> {
    let options = select.options();
    (0..options.length())
        .filter_map(move |index| options.item(index))
        .filter_map(|option| option.dyn_into().ok())
}

/// Whether the user changed the options the server rendered selected. A single
/// select with none of them selected still shows its first option, unedited.
fn user_changed(select: &HtmlSelectElement) -> bool {
    (select.multiple() || options(select).any(|option| option.default_selected()))
        && options(select).any(|option| option.selected() != option.default_selected())
}

pub fn adopt_select(
    scope: &Scope,
    target: impl ElementTarget,
    value: &impl TextValue,
) -> Result<(), JsValue> {
    let select = select_element(scope, target)?;
    let text = select.value();
    if user_changed(&select) && !untrack(|| value.shows(&text)) {
        value.edit(text);
    }
    Ok(())
}

/// A single select.
pub fn select(
    scope: &mut Scope,
    target: impl ElementTarget,
    value: impl TextValue,
) -> Result<(), JsValue> {
    let select = select_element(scope, target)?;
    let element: &Element = select.as_ref();
    let (control, bound) = (select.clone(), value.clone());
    scope.on(element, "change", move |_| bound.edit(control.value()))?;
    touch_on_blur(scope, element, &value)?;
    scope.bind_dom(move || {
        if !value.shows(&select.value()) {
            select.set_value(&value.text());
        }
        Ok(())
    })
}

/// Replace the bound values with the selected options' values, in document order.
fn replace<T: FromStr + PartialEq>(values: &Signal<Vec<T>>, select: &HtmlSelectElement) {
    let chosen: Vec<T> = options(select)
        .filter(HtmlOptionElement::selected)
        .filter_map(|option| option.value().parse().ok())
        .collect();
    if values.with_untracked(|current| *current != chosen) {
        values.replace(chosen);
    }
}

pub fn adopt_select_multiple<T: FromStr + PartialEq + 'static>(
    scope: &Scope,
    target: impl ElementTarget,
    value: &Signal<Vec<T>>,
) -> Result<(), JsValue> {
    let select = select_element(scope, target)?;
    if user_changed(&select) {
        replace(value, &select);
    }
    Ok(())
}

/// A `<select multiple>`.
pub fn select_multiple<T: FromStr + PartialEq + 'static>(
    scope: &mut Scope,
    target: impl ElementTarget,
    value: Signal<Vec<T>>,
) -> Result<(), JsValue> {
    let select = select_element(scope, target)?;
    let element: &Element = select.as_ref();
    let (control, bound) = (select.clone(), value.clone());
    scope.on(element, "change", move |_| replace(&bound, &control))?;
    scope.bind_dom(move || {
        for option in options(&select) {
            let chosen = selected(&value, &option.value());
            if option.selected() != chosen {
                option.set_selected(chosen);
            }
        }
        Ok(())
    })
}

/// A radio as a checkbox that only ever checks: it chooses its `value`.
#[derive(Clone)]
struct Radio<V>(V);

impl<V: TextValue> Checkbox for Radio<V> {
    fn checked(&self, choice: &str) -> bool {
        self.0.shows(choice)
    }
    fn check(&self, choice: &str, checked: bool) {
        if checked && !self.0.shows(choice) {
            self.0.edit(choice.to_owned());
        }
    }
}

/// A checkbox or radio, which must still have the type its template declared.
fn toggle_input(
    scope: &Scope,
    target: impl InputTarget,
    kind: InputKind,
) -> Result<HtmlInputElement, JsValue> {
    let input = target.resolve_input(scope)?;
    if InputKind::of(&input.type_()) == kind {
        Ok(input)
    } else {
        Err(JsValue::from_str(&format!(
            "fusor: bind expected a {kind:?} input"
        )))
    }
}

fn adopt_toggle(
    scope: &Scope,
    target: impl InputTarget,
    kind: InputKind,
    choice: impl Fn() -> String,
    value: &impl Checkbox,
) -> Result<(), JsValue> {
    let input = toggle_input(scope, target, kind)?;
    untrack(|| value.check(&choice(), input.checked()));
    Ok(())
}

/// `choice` is the input's `value` attribute.
fn toggle(
    scope: &mut Scope,
    input: HtmlInputElement,
    choice: impl Fn() -> String + 'static,
    value: impl Checkbox,
) -> Result<(), JsValue> {
    let choice = Rc::new(choice);
    let (control, chosen, bound) = (input.clone(), choice.clone(), value.clone());
    scope.on(&input, "change", move |_| {
        bound.check(&chosen(), control.checked())
    })?;
    scope.bind_dom(move || {
        let checked = value.checked(&choice());
        if input.checked() != checked {
            input.set_checked(checked);
        }
        Ok(())
    })
}

pub fn adopt_checkbox(
    scope: &Scope,
    target: impl InputTarget,
    choice: impl Fn() -> String,
    value: &impl Checkbox,
) -> Result<(), JsValue> {
    adopt_toggle(scope, target, InputKind::Checkbox, choice, value)
}

pub fn checkbox(
    scope: &mut Scope,
    target: impl InputTarget,
    choice: impl Fn() -> String + 'static,
    value: impl Checkbox,
) -> Result<(), JsValue> {
    let input = toggle_input(scope, target, InputKind::Checkbox)?;
    toggle(scope, input, choice, value)
}

pub fn adopt_radio(
    scope: &Scope,
    target: impl InputTarget,
    choice: impl Fn() -> String,
    value: &impl TextValue,
) -> Result<(), JsValue> {
    adopt_toggle(
        scope,
        target,
        InputKind::Radio,
        choice,
        &Radio(value.clone()),
    )
}

/// One radio of a group bound to the same value. The browser clears the others
/// without events, so each radio follows the value rather than its own change.
pub fn radio(
    scope: &mut Scope,
    target: impl InputTarget,
    choice: impl Fn() -> String + 'static,
    value: impl TextValue,
) -> Result<(), JsValue> {
    let input = toggle_input(scope, target, InputKind::Radio)?;
    touch_on_blur(scope, &input, &value)?;
    toggle(scope, input, choice, Radio(value))
}
