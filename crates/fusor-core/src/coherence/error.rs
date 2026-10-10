use std::{any::Any, error, fmt, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Contract,
    Read,
    Renderer,
}

/// A coherent-view failure. Read and renderer failures retain their original
/// value for [`Self::downcast_ref`], including values that implement only Display.
/// Equality compares the category and diagnostic, not the retained value.
/// `source()` exposes a Display adapter; retrieve the original value through
/// `downcast_ref` because that value need not implement `std::error::Error`.
#[derive(Clone)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    cause: Option<Rc<dyn Cause>>,
}

trait Cause: error::Error {
    fn value(&self) -> &dyn Any;
    fn as_error(&self) -> &(dyn error::Error + 'static);
}

struct DisplayCause<T>(Rc<T>);
impl<T: fmt::Display> fmt::Display for DisplayCause<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl<T: fmt::Display> fmt::Debug for DisplayCause<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}
impl<T: fmt::Display> error::Error for DisplayCause<T> {}
impl<T: fmt::Display + 'static> Cause for DisplayCause<T> {
    fn value(&self) -> &dyn Any {
        self.0.as_ref()
    }
    fn as_error(&self) -> &(dyn error::Error + 'static) {
        self
    }
}

impl Error {
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn read<T: fmt::Display + 'static>(cause: Rc<T>) -> Self {
        Self::retained(ErrorKind::Read, cause)
    }

    pub fn renderer<T: fmt::Display + 'static>(cause: T) -> Self {
        Self::retained(ErrorKind::Renderer, Rc::new(cause))
    }

    fn retained<T: fmt::Display + 'static>(kind: ErrorKind, cause: Rc<T>) -> Self {
        Self {
            kind,
            message: cause.to_string(),
            cause: Some(Rc::new(DisplayCause(cause))),
        }
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.cause.as_ref()?.value().downcast_ref()
    }
}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self {
            kind: ErrorKind::Contract,
            message,
            cause: None,
        }
    }
}
impl From<&str> for Error {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Error")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .finish()
    }
}
impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.message == other.message
    }
}
impl Eq for Error {}
impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.cause.as_ref().map(|cause| cause.as_error())
    }
}
