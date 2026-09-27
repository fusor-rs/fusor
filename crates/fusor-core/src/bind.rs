//! What a `bind` attribute means for the Rust value it names. The HTML compiler
//! picks the control from the markup; the bound value's type decides how the
//! control reads and edits it, so rustc checks the pairing. Reads track, so
//! server rendering, hydration and browser effects all use the same methods.
use crate::Signal;
use std::{fmt::Display, str::FromStr};

/// A value edited as text: text-like and numeric inputs, textarea, a single
/// select, and the value a radio group chooses.
///
/// `Signal<T>` shows `T` with `Display` and parses edits with `FromStr`. Text
/// that does not parse, such as a half-typed number, leaves the signal unchanged.
#[diagnostic::on_unimplemented(
    message = "`bind` on this control needs a `Signal<T>` where `T: FromStr + Display + PartialEq`, or a form field",
    label = "`{Self}` cannot be edited as text"
)]
pub trait TextValue: Clone + 'static {
    /// The text the control shows.
    fn text(&self) -> String;
    /// Whether the control's `text` already shows this value. Such an edit is
    /// never written back, which keeps the cursor and partial input intact.
    fn shows(&self, text: &str) -> bool;
    /// Apply the control's text after a user edit.
    fn edit(&self, text: String);
    /// The control lost focus.
    fn touch(&self) {}
}

impl<T: FromStr + Display + PartialEq + 'static> TextValue for Signal<T> {
    fn text(&self) -> String {
        self.with(ToString::to_string)
    }
    fn shows(&self, text: &str) -> bool {
        text.parse::<T>()
            .is_ok_and(|parsed| self.with(|value| *value == parsed))
    }
    fn edit(&self, text: String) {
        if let Ok(value) = text.parse::<T>() {
            self.set(value);
        }
    }
}

/// A checkbox. `Signal<bool>` follows its checked state; `Signal<Vec<T>>` holds
/// the `value` of every checked box bound to it.
#[diagnostic::on_unimplemented(
    message = "`bind` on a checkbox needs a `Signal<bool>` or a `Signal<Vec<T>>`",
    label = "`{Self}` cannot follow a checkbox"
)]
pub trait Checkbox: Clone + 'static {
    /// Whether the box whose `value` is `choice` is checked.
    fn checked(&self, choice: &str) -> bool;
    /// The user checked or cleared the box whose `value` is `choice`.
    fn check(&self, choice: &str, checked: bool);
}

impl Checkbox for Signal<bool> {
    fn checked(&self, _: &str) -> bool {
        self.get()
    }
    fn check(&self, _: &str, checked: bool) {
        self.set(checked);
    }
}

impl<T: FromStr + PartialEq + 'static> Checkbox for Signal<Vec<T>> {
    fn checked(&self, choice: &str) -> bool {
        selected(self, choice)
    }
    fn check(&self, choice: &str, checked: bool) {
        let Ok(choice) = choice.parse::<T>() else {
            return;
        };
        if self.with_untracked(|values| values.contains(&choice)) != checked {
            self.update(|values| {
                if checked {
                    values.push(choice);
                } else {
                    values.retain(|value| *value != choice);
                }
            });
        }
    }
}

/// Whether values bound to a `<select multiple>` or to several checkboxes
/// include the option or box whose `value` is `choice`.
pub fn selected<T: FromStr + PartialEq>(values: &Signal<Vec<T>>, choice: &str) -> bool {
    choice
        .parse::<T>()
        .is_ok_and(|choice| values.with(|values| values.contains(&choice)))
}
