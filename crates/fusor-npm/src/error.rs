use std::{error::Error, fmt, io, process::ExitStatus};

/// A rejected compiler input or a failure to run the JavaScript bundler.
#[derive(Debug)]
pub enum BundleError {
    InvalidManifest,
    InvalidEntry,
    Io(io::Error),
    Json(serde_json::Error),
    Node(io::Error),
    Failed { status: ExitStatus, message: String },
}

impl fmt::Display for BundleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManifest => formatter.write_str("invalid JavaScript module manifest"),
            Self::InvalidEntry => formatter.write_str("JavaScript entry has no parent"),
            Self::Io(error) => error.fmt(formatter),
            Self::Node(error) => write!(
                formatter,
                "JavaScript modules need Node.js 22 or newer: {error}"
            ),
            Self::Json(error) => error.fmt(formatter),
            Self::Failed { message, .. } => {
                write!(formatter, "Fusor JavaScript bundling failed:\n{message}")
            }
        }
    }
}

impl Error for BundleError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) | Self::Node(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for BundleError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for BundleError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
