//! Compiler conversion for an attribute written as one `{{ expression }}`.
//! `Option<T>` sets the attribute for `Some` and removes it for `None`; any
//! other value is always present.

pub struct Value<'a, T: ?Sized>(pub &'a T);

/// The generated receiver is `&Value(&expression)`. The `Option` impl matches
/// directly; an extra autoref selects the always-present fallback.
pub trait Present<'a> {
    type Inner: ?Sized;
    fn __fusor_present(self) -> Option<&'a Self::Inner>;
}

impl<'a, T: ?Sized> Present<'a> for &&Value<'a, T> {
    type Inner = T;
    fn __fusor_present(self) -> Option<&'a T> {
        Some(self.0)
    }
}

impl<'a, T> Present<'a> for &Value<'a, Option<T>> {
    type Inner = T;
    fn __fusor_present(self) -> Option<&'a T> {
        self.0.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_is_optional_and_other_values_are_present() {
        assert_eq!((&Value(&Some("/home"))).__fusor_present(), Some(&"/home"));
        assert_eq!((&Value(&None::<u32>)).__fusor_present(), None);
        assert_eq!((&Value(&"/home")).__fusor_present(), Some(&"/home"));
        assert_eq!((&Value(&7u32)).__fusor_present(), Some(&7));
    }
}
