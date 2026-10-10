use std::{error::Error as StdError, fmt};

/// Invalid delivery metadata or an explicit cross-unit message.
#[derive(Debug)]
pub enum Error {
    Protocol(u32),
    Manifest {
        identity: String,
        reason: &'static str,
    },
    Unregistered {
        descriptor: &'static str,
        unit: &'static str,
    },
    DescriptorMismatch(&'static str),
    MessagePayload,
    Decode(serde_json::Error),
}
impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(version) => write!(
                formatter,
                "unsupported island delivery protocol {version}; expected {}",
                crate::PROTOCOL_VERSION
            ),
            Self::Manifest { identity, reason } => write!(formatter, "{identity}: {reason}"),
            Self::Unregistered { descriptor, unit } => {
                write!(formatter, "unregistered island {descriptor} in unit {unit}")
            }
            Self::DescriptorMismatch(descriptor) => {
                write!(formatter, "island descriptor/schema mismatch: {descriptor}")
            }
            Self::MessagePayload => formatter.write_str("island event requires opaque JSON text"),
            Self::Decode(error) => error.fmt(formatter),
        }
    }
}
impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            _ => None,
        }
    }
}
