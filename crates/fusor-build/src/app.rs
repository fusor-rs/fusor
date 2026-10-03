//! Application configuration, generation and the versioned Cargo artifact contract.
//! Inline sources become `crate::ui` modules; external sources remain native modules.
mod artifact;
mod config;
mod error;
mod external;
mod generate;
mod includes;
mod validate;

pub use artifact::{
    ARTIFACT_VERSION, ArtifactManifest, JavaScriptArtifact, RegistrationArtifact, SourceArtifact,
};
pub(crate) use artifact::{HTML_FILE, MANIFEST_FILE, MODULE_FILE};
pub use config::{
    AppConfig, DeliveryConfig, DeliveryMode, DeliveryUnit, Source, SourceKind, valid_module_name,
};
pub use error::SourceError;
pub use generate::generate;
pub use includes::includes_foreign_file;
#[doc(hidden)]
pub use includes::includes_foreign_file_with;

type Result<T> = std::result::Result<T, crate::BuildError>;
