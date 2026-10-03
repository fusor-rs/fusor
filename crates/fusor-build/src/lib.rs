//! Compile ordinary Rust embedded in `<script type="text/rust">` HTML elements.
//!
//! Rust bodies are copied verbatim. HTML bindings lower to native Rust token
//! trees; rustc checks every expression and its types. Template descriptions
//! share a typed contract with the DOM runtime.

pub mod app;
#[doc(hidden)]
pub mod backend;
mod bindings;
mod build_error;
mod cargo;
mod extract;
mod html;
mod javascript;
mod source_map;

pub use app::SourceError;
pub use build_error::BuildError;
pub use cargo::{compile, compile_app};
pub use extract::extract;
pub use source_map::{SourceMap, SourceMapError};

use std::{error::Error, fmt, ops::Range};

/// The original byte ranges of a Rust script's element and its content.
#[derive(Debug, Clone)]
pub struct RustBlock {
    pub element: Range<usize>,
    pub content: Range<usize>,
    pub external: Option<ExternalRust>,
}

/// An explicit relationship to an authored, ordinary Cargo module.
#[derive(Debug, Clone)]
pub struct ExternalRust {
    pub src: String,
    pub module: String,
}

/// A component-local native JavaScript module, before path resolution.
#[derive(Debug, Clone)]
pub struct JavaScriptModule {
    pub id: String,
    pub component: String,
    pub src: Option<String>,
    pub content: String,
    pub line: usize,
    pub column: usize,
}

/// An HTML page and the ordinary Rust module extracted from its script blocks.
#[derive(Debug)]
pub struct Page {
    /// Line-preserved Rust bodies followed by generated component implementations.
    pub rust: String,
    /// Native binding tokens used for compatible refresh, without delivery-only
    /// HTML literals. Island delivery always publishes a complete generation.
    pub fingerprint: String,
    /// HTML with Rust scripts removed and binding declarations replaced by DOM markers.
    pub html: String,
    pub blocks: Vec<RustBlock>,
    pub locations: Vec<BindingLocation>,
    pub javascript: Vec<JavaScriptModule>,
    pub(crate) loader_offset: Option<usize>,
    pub(crate) component_count: usize,
    pub(crate) app_offset: Option<usize>,
}

/// A generated binding's source location in the authored HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingLocation {
    pub generated_start: usize,
    /// Exclusive end line.
    pub generated_end: usize,
    pub line: usize,
    pub column: usize,
}

impl Page {
    /// Insert the module loader near the first Rust block, outside inert templates.
    /// Module scripts defer execution until the document has been parsed.
    pub fn with_loader(&self) -> String {
        let mut html = self.html.clone();
        if let Some(offset) = self.loader_offset {
            html.insert_str(offset, r#"<script type="module" src="./boot.js"></script>"#);
        }
        html
    }
}

/// An invalid HTML container or unbalanced Rust binding tokens.
/// Rust expression grammar, types, and borrowing are checked by rustc.
#[derive(Debug, PartialEq, Eq)]
pub struct ExtractError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ExtractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl Error for ExtractError {}

/// The 1-based line and character column of a byte offset, clamped to the source.
pub(crate) fn location(source: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(source.len());
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, column)
}

fn error(source: &str, offset: usize, message: impl Into<String>) -> ExtractError {
    let (line, column) = location(source, offset);
    ExtractError {
        line,
        column,
        message: message.into(),
    }
}
