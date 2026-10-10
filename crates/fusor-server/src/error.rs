use std::{error::Error as StdError, fmt};

/// A native rendering failure, retaining serialization and delivery causes.
#[derive(Debug)]
pub enum Error {
    Render(String),
    Islands(fusor_islands::Error),
    Json(serde_json::Error),
    Construction {
        component: &'static str,
        error_type: &'static str,
        reason: Option<String>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Render(message) => formatter.write_str(message),
            Self::Islands(error) => error.fmt(formatter),
            Self::Json(error) => error.fmt(formatter),
            Self::Construction {
                component,
                error_type,
                reason,
            } => {
                write!(
                    formatter,
                    "component {component} input construction failed ({error_type})"
                )?;
                if let Some(reason) = reason {
                    write!(formatter, ": {reason}")?;
                }
                Ok(())
            }
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Islands(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Render(_) | Self::Construction { .. } => None,
        }
    }
}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self::Render(message)
    }
}
impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Self::Render(message.into())
    }
}
impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
impl From<fusor_islands::Error> for Error {
    fn from(error: fusor_islands::Error) -> Self {
        Self::Islands(error)
    }
}

/// Compiler adapter: retain Display diagnostics without constraining FromInputs.
#[doc(hidden)]
pub struct InputError<T>(pub T);
#[doc(hidden)]
pub trait InputErrorDiagnostic {
    fn diagnostic(self, component: &'static str) -> Error;
}
impl<T: fmt::Display> InputErrorDiagnostic for InputError<T> {
    fn diagnostic(self, component: &'static str) -> Error {
        Error::Construction {
            component,
            error_type: std::any::type_name::<T>(),
            reason: Some(self.0.to_string()),
        }
    }
}
impl<T> InputErrorDiagnostic for &InputError<T> {
    fn diagnostic(self, component: &'static str) -> Error {
        Error::Construction {
            component,
            error_type: std::any::type_name::<T>(),
            reason: None,
        }
    }
}
