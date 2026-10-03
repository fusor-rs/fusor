//! Compiler conversion for native Text. Only exact small integers bypass Rust
//! formatting; all other values retain their ordinary ToString implementation.

pub struct Value<'a, T: ?Sized>(pub &'a T);

pub enum Output {
    String(String),
    Integer(f64),
}

/// The generated receiver is `&Value(&expression)`. Exact primitive impls match
/// directly; an extra autoref selects the generic fallback. Borrowing expression
/// preserves field access and temporary destruction before the native write.
pub trait Convert {
    fn __fusor_into_text(self) -> Output;
}

impl<T: ToString + ?Sized> Convert for &&Value<'_, T> {
    fn __fusor_into_text(self) -> Output {
        Output::String(self.0.to_string())
    }
}

macro_rules! integers {
    ($($ty:ty),* $(,)?) => {$(
        impl Convert for &Value<'_, $ty> {
            fn __fusor_into_text(self) -> Output {
                Output::Integer(f64::from(*self.0))
            }
        }
    )*};
}
// Every value is exactly representable, and decimal Number conversion uses no
// exponent notation in these ranges. Floats and wider integers must fall back.
integers!(u8, u16, u32, i8, i16, i32);

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn exact_integer_dispatch_and_generic_fallback_keep_formatting_contract() {
        type Count = u32;
        assert!(matches!(
            (&Value(&42)).__fusor_into_text(),
            Output::Integer(42.0)
        ));
        assert!(matches!(
            (&Value(&(42 as Count))).__fusor_into_text(),
            Output::Integer(42.0)
        ));
        let number = 42u32;
        assert!(matches!((&Value(&&number)).__fusor_into_text(), Output::String(s) if s == "42"));
        assert!(
            matches!((&Value(&u32::MAX)).__fusor_into_text(), Output::Integer(n) if n == 4294967295.0)
        );
        assert!(
            matches!((&Value(&i32::MIN)).__fusor_into_text(), Output::Integer(n) if n == -2147483648.0)
        );
        for output in [
            (&Value(&u64::MAX)).__fusor_into_text(),
            (&Value(&-0.0f64)).__fusor_into_text(),
            (&Value(&"日本語<&")).__fusor_into_text(),
        ] {
            assert!(matches!(output, Output::String(_)));
        }
        // Generic functions cannot assume the concrete type and keep fallback.
        fn generic<T: ToString>(value: &T) -> Output {
            (&Value(value)).__fusor_into_text()
        }
        assert!(matches!(generic(&42u32), Output::String(s) if s == "42"));
        // Borrowed fields must not move, and ToString-only values need no Display.
        struct Custom(Rc<RefCell<Vec<&'static str>>>);
        #[expect(
            clippy::to_string_trait_impl,
            reason = "the public fallback accepts ToString without Display"
        )]
        impl ToString for Custom {
            fn to_string(&self) -> String {
                self.0.borrow_mut().push("format");
                "custom".into()
            }
        }
        impl Drop for Custom {
            fn drop(&mut self) {
                self.0.borrow_mut().push("drop");
            }
        }
        let events = Rc::new(RefCell::new(Vec::new()));
        let read = || (&Value(&Custom(events.clone()))).__fusor_into_text();
        let output = read();
        assert_eq!(*events.borrow(), ["format", "drop"]);
        assert!(matches!(output, Output::String(s) if s == "custom"));
        let value = Custom(events.clone());
        let _ = (&Value(&value)).__fusor_into_text();
        let _ = (&Value(&value)).__fusor_into_text();
        assert_eq!(*events.borrow(), ["format", "drop", "format", "format"]);
    }
}
