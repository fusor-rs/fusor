use fusor::{Signal, batch, signal, untrack};
use std::{
    cell::RefCell,
    fmt::Display,
    rc::{Rc, Weak},
    str::FromStr,
};

#[derive(Clone)]
pub(super) struct State {
    pub raw: String,
    pub baseline: String,
    pub revision: u64,
    pub baseline_revision: u64,
    pub touched: bool,
    pub submitted: bool,
    pub server: Option<Rc<Issue>>,
}
pub(super) struct Core {
    pub id: u64,
    pub state: Signal<State>,
}

/// Opaque identity/revision captured by a form. Values remain in the typed command.
#[doc(hidden)]
#[derive(Clone)]
pub struct FieldStamp {
    pub(super) id: u64,
    pub(super) revision: u64,
    pub(super) baseline_revision: u64,
    pub(super) core: Weak<Core>,
}
impl FieldStamp {
    pub(super) fn current(&self, tracked: bool) -> bool {
        self.core.upgrade().is_some_and(|core| {
            let read = |state: &State| {
                state.revision == self.revision && state.baseline_revision == self.baseline_revision
            };
            if tracked {
                core.state.with(read)
            } else {
                core.state.with_untracked(read)
            }
        })
    }
    #[cfg(feature = "actions")]
    pub(super) fn baseline_current(&self) -> bool {
        self.core.upgrade().is_some_and(|core| {
            core.state
                .with_untracked(|state| state.baseline_revision == self.baseline_revision)
        })
    }
}
pub(super) struct Issue {
    pub message: String,
    pub dependencies: Vec<FieldStamp>,
    #[cfg(feature = "actions")]
    pub origin: u64,
}
impl Issue {
    pub fn current(&self) -> bool {
        self.dependencies.iter().all(|stamp| {
            stamp
                .core
                .upgrade()
                .is_some_and(|core| core.state.with(|state| state.revision == stamp.revision))
        })
    }
    #[cfg(feature = "actions")]
    pub fn superseded_by(&self, submitted: &[FieldStamp], submission: u64) -> bool {
        self.origin <= submission
            && self.dependencies.iter().all(|dependency| {
                submitted
                    .iter()
                    .any(|old| old.id == dependency.id && old.revision >= dependency.revision)
            })
    }
}
type Parser<T> = dyn Fn(&str) -> Result<T, String>;
type Validator<T> = dyn Fn(&T) -> Result<(), String>;
struct Inner<T> {
    core: Rc<Core>,
    parse: Box<Parser<T>>,
    format: Box<dyn Fn(&T) -> String>,
    validators: RefCell<Vec<Rc<Validator<T>>>>,
}

/// Raw text and a separately confirmed baseline. Invalid intermediate text is
/// preserved. Clone shares field identity; two views can edit the same field.
pub struct TextField<T>(Rc<Inner<T>>);
impl<T> Clone for TextField<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: FromStr + Display + 'static> TextField<T>
where
    T::Err: Display,
{
    pub fn new(initial: T) -> Self {
        Self::with_parser(
            initial,
            |raw| raw.parse::<T>().map_err(|error| error.to_string()),
            ToString::to_string,
        )
    }
}
impl<T: 'static> TextField<T> {
    pub fn with_parser(
        initial: T,
        parse: impl Fn(&str) -> Result<T, String> + 'static,
        format: impl Fn(&T) -> String + 'static,
    ) -> Self {
        let raw = format(&initial);
        Self(Rc::new(Inner {
            core: Rc::new(Core {
                id: crate::identity::next(),
                state: signal(State {
                    baseline: raw.clone(),
                    raw,
                    revision: 0,
                    baseline_revision: 0,
                    touched: false,
                    submitted: false,
                    server: None,
                }),
            }),
            parse: Box::new(parse),
            format: Box::new(format),
            validators: RefCell::new(Vec::new()),
        }))
    }
    /// Configure before sharing the field. Validators must be pure. They may
    /// read other fields; preparation detects changes to registered revisions.
    pub fn validate(self, validator: impl Fn(&T) -> Result<(), String> + 'static) -> Self {
        self.0.validators.borrow_mut().push(Rc::new(validator));
        self.0.core.state.update(|state| {
            state.revision = state
                .revision
                .checked_add(1)
                .expect("field revision overflow")
        });
        self
    }
    pub fn raw(&self) -> String {
        self.0.core.state.with(|state| state.raw.clone())
    }
    pub fn baseline(&self) -> String {
        self.0.core.state.with(|state| state.baseline.clone())
    }
    pub fn revision(&self) -> u64 {
        self.0.core.state.with(|state| state.revision)
    }
    pub fn dirty(&self) -> bool {
        self.0.core.state.with(|state| state.raw != state.baseline)
    }
    pub fn touched(&self) -> bool {
        self.0.core.state.with(|state| state.touched)
    }
    pub fn touch(&self) {
        self.0.core.state.update(|state| state.touched = true);
    }

    /// Every edit advances the revision, even an equal-value edit. This protects
    /// a newly composing draft from an older normalization response.
    pub fn edit(&self, raw: impl Into<String>) {
        let raw = raw.into();
        self.0.core.state.update(|state| {
            state.revision = state
                .revision
                .checked_add(1)
                .expect("field revision overflow");
            state.raw = raw;
        });
    }
    pub fn parsed(&self) -> Result<T, String> {
        let raw = self.raw();
        (self.0.parse)(&raw)
    }
    pub fn validated(&self) -> Result<T, String> {
        let value = self.parsed()?;
        let validators = self.0.validators.borrow().clone();
        for validator in validators {
            validator(&value)?;
        }
        let server = self.0.core.state.with(|state| state.server.clone());
        if let Some(issue) = server.filter(|issue| issue.current()) {
            return Err(issue.message.clone());
        }
        Ok(value)
    }
    pub fn invalid(&self) -> bool {
        self.error().is_some()
    }
    pub fn error(&self) -> Option<String> {
        self.validated().err()
    }
    /// Errors are visible after blur or submission. Validity itself is independent
    /// of visibility. Old server errors expire when any submitted dependency edits.
    pub fn message(&self) -> String {
        if self
            .0
            .core
            .state
            .with(|state| state.touched || state.submitted)
        {
            self.error().unwrap_or_default()
        } else {
            String::new()
        }
    }
    /// Explicitly discard the draft and establish a reviewed server baseline.
    /// Invalidates older snapshots. A pending accepted response cannot undo this.
    pub fn reset(&self, value: T) {
        let raw = untrack(|| (self.0.format)(&value));
        self.0.core.state.update(|state| {
            state.revision = state
                .revision
                .checked_add(1)
                .expect("field revision overflow");
            state.baseline_revision = state
                .baseline_revision
                .checked_add(1)
                .expect("baseline revision overflow");
            state.raw = raw.clone();
            state.baseline = raw;
            state.touched = false;
            state.submitted = false;
            state.server = None;
        });
    }
    pub(super) fn stamp(&self) -> FieldStamp {
        self.0.core.state.with_untracked(|state| FieldStamp {
            id: self.0.core.id,
            revision: state.revision,
            baseline_revision: state.baseline_revision,
            core: Rc::downgrade(&self.0.core),
        })
    }
    pub(super) fn submitted(&self) {
        self.0.core.state.update(|state| state.submitted = true);
    }
    #[cfg(feature = "actions")]
    pub(super) fn formatted(&self, value: &T) -> String {
        (self.0.format)(value)
    }
    #[cfg(feature = "actions")]
    pub(super) fn core(&self) -> Rc<Core> {
        self.0.core.clone()
    }
}

/// `bind` edits the raw text; parsing and validation stay with the field.
impl<T: 'static> fusor::bind::TextValue for TextField<T> {
    fn text(&self) -> String {
        self.raw()
    }
    fn shows(&self, text: &str) -> bool {
        self.0.core.state.with(|state| state.raw == text)
    }
    fn edit(&self, text: String) {
        TextField::edit(self, text);
    }
    fn touch(&self) {
        TextField::touch(self);
    }
}

mod sealed {
    pub trait Sealed {}
}
/// Supported typed field collections. Tuples of one through eight text fields
/// collect all values without reflection over arbitrary application structs.
pub trait Fields: sealed::Sealed + Clone + 'static {
    type Values;
    #[doc(hidden)]
    fn stamps(&self) -> Vec<FieldStamp>;
    #[doc(hidden)]
    fn values(&self) -> Result<Self::Values, crate::forms::FormError>;
    #[doc(hidden)]
    fn mark_submitted(&self);
    fn dirty(&self) -> bool;
}
macro_rules! fields {
    ($($type:ident:$value:ident:$index:tt),+) => {
        impl<$($type: 'static),+> sealed::Sealed for ($(TextField<$type>,)+) {}
        impl<$($type: 'static),+> Fields for ($(TextField<$type>,)+) {
            type Values = ($($type,)+);
            fn stamps(&self) -> Vec<FieldStamp> { vec![$(self.$index.stamp()),+] }
            fn values(&self) -> Result<Self::Values, $crate::forms::FormError> {
                $(let $value = self.$index.validated();)+
                Ok(($($value.map_err(|_| $crate::forms::FormError::Invalid)?,)+))
            }
            fn mark_submitted(&self) { batch(|| { $(self.$index.submitted();)+ }); }
            fn dirty(&self) -> bool { $(self.$index.dirty())|+ }
        }
    };
}
mod tuples {
    use super::*;
    fields!(A:a:0);
    fields!(A:a:0,B:b:1);
    fields!(A:a:0,B:b:1,C:c:2);
    fields!(A:a:0,B:b:1,C:c:2,D:d:3);
    fields!(A:a:0,B:b:1,C:c:2,D:d:3,E:e:4);
    fields!(A:a:0,B:b:1,C:c:2,D:d:3,E:e:4,F:f:5);
    fields!(A:a:0,B:b:1,C:c:2,D:d:3,E:e:4,F:f:5,G:g:6);
    fields!(A:a:0,B:b:1,C:c:2,D:d:3,E:e:4,F:f:5,G:g:6,H:h:7);
}
