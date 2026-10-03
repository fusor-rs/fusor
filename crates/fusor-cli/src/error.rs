use std::{fmt, process::ExitCode};

pub type Result<T = ()> = std::result::Result<T, Error>;

/// Each kind has its own exit code, so a script can tell a mistyped flag from
/// a compile failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The command line was wrong.
    Usage,
    /// The application, its manifest, its locks or its output.
    Project,
    /// An external tool is missing, the wrong version, or failed.
    Tooling,
    /// Rust or HTML compilation, or island registration, failed.
    Compile,
    /// A bug in this CLI.
    Internal,
}

impl Kind {
    /// Exit codes are part of the CLI's contract; see the CLI reference guide.
    fn exit_code(self) -> u8 {
        match self {
            Self::Usage => 2,
            Self::Project => 3,
            Self::Tooling => 4,
            Self::Compile => 5,
            Self::Internal => 70,
        }
    }
}

pub struct Error {
    message: String,
    remedy: Option<String>,
    kind: Kind,
}

impl Error {
    fn new(kind: Kind, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            remedy: None,
            kind,
        }
    }

    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self::new(Kind::Usage, message)
    }
    pub(crate) fn project(message: impl Into<String>) -> Self {
        Self::new(Kind::Project, message)
    }
    pub(crate) fn tooling(message: impl Into<String>) -> Self {
        Self::new(Kind::Tooling, message)
    }
    pub(crate) fn compile(message: impl Into<String>) -> Self {
        Self::new(Kind::Compile, message)
    }
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(Kind::Internal, message)
    }

    /// One imperative sentence, printed as `help:` under the message.
    pub(crate) fn remedy(mut self, remedy: impl Into<String>) -> Self {
        self.remedy = Some(remedy.into());
        self
    }

    /// Apply at command and stage boundaries only, so chains stay short.
    pub(crate) fn context(mut self, context: impl fmt::Display) -> Self {
        self.message = format!("{context}: {}", self.message);
        self
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn report(&self) -> ExitCode {
        eprintln!("fusor: {}", self.message);
        if let Some(remedy) = &self.remedy {
            eprintln!("  help: {remedy}");
        }
        ExitCode::from(self.kind.exit_code())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)?;
        if let Some(remedy) = &self.remedy {
            write!(formatter, "\n  help: {remedy}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {self}", self.kind)
    }
}

// Foreign errors are almost always about the user's files, so `?` classifies
// them as `Kind::Project`. Construct the error explicitly where the kind
// matters. The list is explicit so a new dependency is not classified by
// accident.
macro_rules! from_foreign {
    ($($type:ty),* $(,)?) => {
        $(impl From<$type> for Error {
            fn from(error: $type) -> Self {
                Self::project(error.to_string())
            }
        })*
    };
}

from_foreign!(
    std::io::Error,
    std::path::StripPrefixError,
    serde_json::Error,
    toml::de::Error,
    toml_edit::TomlError,
    semver::Error,
    glob::PatternError,
    glob::GlobError,
    fusor_build::SourceMapError,
    fusor_build::BuildError,
    fusor_npm::BundleError,
);

/// `fusor_islands` reports validation failures as text.
impl From<String> for Error {
    fn from(message: String) -> Self {
        Self::project(message)
    }
}
