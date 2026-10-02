#![cfg(feature = "derive")]

use fusor::{FromInputs, Owner, Signal, signal};
use std::cell::Cell;

// These constructors compile and execute with no DOM feature or mounting trait.

mod child {
    use super::*;
    #[derive(FromInputs)]
    pub struct Counter {
        #[input]
        pub(super) count: Signal<i32>,
        #[local(init = signal(0))]
        pub(super) clicks: Signal<i32>,
    }
}

#[test]
fn parent_constructs_inputs_and_each_instance_gets_local_state() {
    let owner = Owner::new();
    let count = signal(0);
    let first = child::Counter::from_inputs(
        child::CounterInputs {
            count: count.clone(),
        },
        owner.handle(),
    )
    .unwrap();
    let second = child::Counter::from_inputs(
        child::CounterInputs {
            count: count.clone(),
        },
        owner.handle(),
    )
    .unwrap();
    first.count.set(7);
    first.clicks.set(2);
    assert_eq!(second.count.get(), 7);
    assert_eq!(second.clicks.get(), 0);
}

thread_local! { static CALLS: Cell<u32> = const { Cell::new(0) }; }
fn next() -> u32 {
    CALLS.with(|calls| {
        let next = calls.get() + 1;
        calls.set(next);
        next
    })
}
#[derive(FromInputs)]
struct Local {
    #[local(init = next())]
    first: u32,
    #[local(init = Self::initial())]
    second: u32,
}
impl Local {
    fn initial() -> u32 {
        next()
    }
}
#[derive(FromInputs)]
struct Empty;
#[derive(FromInputs)]
struct Braces {}
#[derive(FromInputs)]
struct Recursive {
    #[input]
    nested: Option<Box<Self>>,
    #[input]
    r#type: &'static str,
    #[input]
    bytes: [u8; Self::SIZE],
}
impl Recursive {
    const SIZE: usize = 2;
}

#[test]
fn local_expressions_run_once_in_declaration_order() {
    CALLS.with(|calls| calls.set(0));
    let value = Local::from_inputs(LocalInputs {}, Owner::new().handle()).unwrap();
    assert_eq!((value.first, value.second), (1, 2));
    let value = Local::from_inputs(LocalInputs, Owner::new().handle()).unwrap();
    assert_eq!((value.first, value.second), (3, 4));
    Empty::from_inputs(EmptyInputs {}, Owner::new().handle()).unwrap();
    Braces::from_inputs(BracesInputs {}, Owner::new().handle()).unwrap();
}

#[test]
fn input_types_preserve_self_and_raw_identifiers() {
    let leaf = Recursive::from_inputs(
        RecursiveInputs {
            nested: None,
            r#type: "leaf",
            bytes: [1, 2],
        },
        Owner::new().handle(),
    )
    .unwrap();
    let parent = Recursive::from_inputs(
        RecursiveInputs {
            nested: Some(Box::new(leaf)),
            r#type: "parent",
            bytes: [3, 4],
        },
        Owner::new().handle(),
    )
    .unwrap();
    assert_eq!(parent.bytes, [3, 4]);
    assert_eq!(parent.nested.unwrap().r#type, "leaf");
}

mod renamed {
    use fusor as rf;
    #[derive(rf::FromInputs)]
    #[from_inputs(crate = rf)]
    pub(super) struct Renamed {
        #[input]
        pub(super) value: String,
    }
}

#[test]
fn explicit_runtime_path_works_with_reexports() {
    let value = renamed::Renamed::from_inputs(
        renamed::RenamedInputs {
            value: "hello".into(),
        },
        Owner::new().handle(),
    )
    .unwrap();
    assert_eq!(value.value, "hello");
}

#[derive(FromInputs)]
struct Conditional {
    #[cfg(not(feature = "derive"))]
    absent: MissingType,
    #[cfg_attr(feature = "derive", input)]
    value: bool,
    #[cfg(feature = "derive")]
    #[local(init = 3)]
    local: u32,
}

#[test]
fn conditional_fields_follow_rust_configuration() {
    let value =
        Conditional::from_inputs(ConditionalInputs { value: true }, Owner::new().handle()).unwrap();
    assert!(value.value);
    assert_eq!(value.local, 3);
}

#[derive(Debug, PartialEq)]
enum InputError {
    Negative,
}

struct Manual {
    value: i32,
    _effect: fusor::Effect,
    _cleanup: fusor::Registration,
}

struct ManualInputs {
    value: i32,
    observed: Signal<Vec<&'static str>>,
}

impl FromInputs for Manual {
    type Inputs = ManualInputs;
    type Error = InputError;

    fn from_inputs(inputs: Self::Inputs, owner: fusor::OwnerHandle) -> Result<Self, Self::Error> {
        if inputs.value < 0 {
            return Err(InputError::Negative);
        }
        let observed = inputs.observed.clone();
        let subscription = fusor::effect(move || observed.update(|log| log.push("effect")));
        let cleanup = owner.on_cleanup(move || inputs.observed.update(|log| log.push("cleanup")));
        Ok(Self {
            value: inputs.value,
            _effect: subscription,
            _cleanup: cleanup,
        })
    }
}

#[test]
fn manual_errors_are_portable_and_construction_does_not_defer_ordinary_effects() {
    let owner = Owner::new();
    let observed = signal(Vec::new());
    assert!(matches!(
        Manual::from_inputs(
            ManualInputs {
                value: -1,
                observed: observed.clone(),
            },
            owner.handle(),
        ),
        Err(InputError::Negative)
    ));
    assert!(observed.get().is_empty());
    let state = Manual::from_inputs(
        ManualInputs {
            value: 3,
            observed: observed.clone(),
        },
        owner.handle(),
    )
    .unwrap();
    assert_eq!(state.value, 3);
    assert!(!owner.handle().is_active());
    assert_eq!(observed.get(), ["effect"]);
    owner.commit();
    assert_eq!(observed.get(), ["effect"]);
    owner.dispose();
    assert_eq!(observed.get(), ["effect", "cleanup"]);
}

#[cfg(feature = "dom")]
#[test]
fn old_manual_jsvalue_error_and_explicit_browser_error_conversions_compile() {
    use fusor::dom::{FromInputs as BrowserFromInputs, IntoMountError, JsValue};

    struct OldBrowserComponent;
    impl BrowserFromInputs for OldBrowserComponent {
        type Inputs = ();
        type Error = JsValue;

        fn from_inputs(_: (), _: fusor::OwnerHandle) -> Result<Self, JsValue> {
            Ok(Self)
        }
    }

    // Compile every documented conversion without invoking JS on a native test.
    fn accepts<E: IntoMountError>() {}
    accepts::<JsValue>();
    accepts::<std::convert::Infallible>();
    accepts::<String>();
    accepts::<&str>();
    let owner = Owner::new();
    assert!(OldBrowserComponent::from_inputs((), owner.handle()).is_ok());
    let derived: Result<Empty, JsValue> =
        Empty::from_inputs(EmptyInputs, owner.handle()).map_err(IntoMountError::into_mount_error);
    assert!(derived.is_ok());
}
