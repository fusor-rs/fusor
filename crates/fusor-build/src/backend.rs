//! Versioned, in-process compiler integration for independently owned renderers.
//!
//! Fusor owns parsing, lexical captures, typed input construction and factories.
//! A backend owns static-node emission and the operations those factories call.
//! Browser builds select the internal `DomBackend`; this public facade adapts
//! external renderers to the same private compiler emission contract.

pub mod build;

use crate::{ExtractError, SourceMap};
use proc_macro2::TokenStream;
use syn::Path;

/// Changes when the backend callback or generated-code contract changes.
pub const VERSION: u32 = 4;

/// An authored location, using a byte offset and one-based line/character column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}
impl Origin {
    /// Attach a backend capability diagnostic to this authored location.
    pub fn error(&self, message: impl Into<String>) -> ExtractError {
        ExtractError {
            line: self.line,
            column: self.column,
            message: message.into(),
        }
    }
}

/// A static attribute, with HTML entities decoded.
#[derive(Clone, Debug)]
pub struct Attribute {
    pub name: String,
    pub value: String,
    pub origin: Origin,
}

/// Complete static structure in preorder. Parents always precede their children.
#[derive(Clone, Debug)]
pub struct Node {
    pub parent: Option<usize>,
    pub kind: NodeKind,
    pub origin: Origin,
}

/// Backend-local anchors are sparse and scoped to a mounted template instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Element(usize),
    Text(usize),
    Mount(usize),
}

#[derive(Clone, Debug)]
pub enum NodeKind {
    Element {
        tag: String,
        attributes: Vec<Attribute>,
        anchor: Option<usize>,
    },
    Text {
        value: String,
        anchor: Option<usize>,
    },
    /// A child region; no browser marker parsing is required downstream.
    Mount {
        anchor: usize,
    },
    Comment(String),
}

#[derive(Clone, Debug)]
pub struct Template {
    pub id: usize,
    pub nodes: Vec<Node>,
    pub origin: Origin,
}

/// Explicitly opt in to each operation. Unimplemented features fail before emission.
#[derive(Clone, Copy, Debug)]
pub enum Capability<'a> {
    /// Application entry generation, as distinct from reusable components.
    App,
    Text,
    Attribute(&'a str),
    Property(&'a str),
    Boolean(&'a str),
    Value,
    Checked,
    Class(&'a str),
    Event(&'a str),
    Bind(Control),
    Branch,
    Keyed,
    Component,
    Children,
    Router,
    Async,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Text,
    Checkbox,
    Radio,
    Select,
    SelectMultiple,
}

/// Paths supplied by the external build helper, never inferred from architecture.
/// `Scope` implements `fusor::render::Scope`. `Children` supplies
/// `default`, `new`, `take`, `with_named`, `named` and `Clone`.
/// `with_named` accepts `(static name, Children)` pairs; `named` selects one
/// fragment, returning empty children for omitted names. Error conversion is a generic function.
pub struct Runtime {
    pub scope: Path,
    pub error: Path,
    pub children: Path,
    pub convert_error: Path,
    /// Opt in to coherent installation.
    pub coherent_frame: Option<Path>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationMode {
    Reactive,
    Coherent,
}

/// Fully lowered operations. Captures and closure lifetimes belong to fusor.
/// Emit statements using `__fusor_scope` or `__fusor_frame` according to `mode`.
pub struct Operation {
    pub anchor: Anchor,
    pub origin: Origin,
    pub kind: OperationKind,
    pub mode: OperationMode,
}

/// A route's captured factory. Parameters and lexical aliases are lowered by fusor.
#[derive(Clone)]
pub struct Route {
    pub pattern: Option<String>,
    /// `Fn(&OwnerHandle, &fusor_router::pattern::Match) -> Result<Scope, Error>`.
    pub prepare: TokenStream,
}

#[derive(Clone)]
pub enum OperationKind {
    Text {
        read: TokenStream,
    },
    Attribute {
        name: String,
        read: TokenStream,
    },
    Property {
        name: String,
        read: TokenStream,
    },
    Boolean {
        name: String,
        read: TokenStream,
    },
    Value {
        read: TokenStream,
    },
    Checked {
        read: TokenStream,
    },
    Class {
        name: String,
        read: TokenStream,
    },
    Event {
        name: String,
        handler: TokenStream,
    },
    /// `value` is an already cloned binding value; `choice` is a String reader.
    Bind {
        control: Control,
        value: TokenStream,
        choice: Option<TokenStream>,
    },
    /// `read: Fn() -> (usize, T)`; `prepare: Fn(usize, Signal<T>, &OwnerHandle)`.
    Branch {
        read: TokenStream,
        prepare: TokenStream,
    },
    /// `read: Fn() -> Vec<T>`; `key: Fn(&T) -> K`; `prepare: Fn(Signal<T>, &OwnerHandle)`.
    Keyed {
        read: TokenStream,
        key: TokenStream,
        prepare: TokenStream,
    },
    /// `identity: Fn() -> Option<K>`; `make: Fn(OwnerHandle) -> Result<T, Error>`.
    Component {
        ty: TokenStream,
        identity: TokenStream,
        make: TokenStream,
        children: TokenStream,
    },
    Children {
        children: TokenStream,
    },
    Router {
        routes: Vec<Route>,
    },
    /// Attach a boundary. `render` is `Fn(&mut Frame<'_>) -> Result<(), String>`.
    Async {
        boundary: TokenStream,
        render: TokenStream,
    },
}

/// A declared component implementation or an application entry. `body` prepares
/// a Scope using locals `parent: Option<&OwnerHandle>` and `make` (FnOnce).
/// It uses `fusor::render::construct`, without committing the owner. Ordinary
/// scopes preserve immediate effects; explicitly prepared scopes defer them.
/// The renderer validates/publishes the result before activation.
pub struct ComponentCode {
    pub ty: TokenStream,
    pub body: TokenStream,
    pub app_state: Option<TokenStream>,
}

/// Bounded backend v2. Unsupported hydration/JS/server/opaque-content
/// operations are rejected by this version even if a backend would accept them.
pub trait Backend {
    /// Declarations shared by every component in this generated source file.
    fn file(&self) -> TokenStream {
        TokenStream::new()
    }
    fn version(&self) -> u32;
    fn name(&self) -> &str;
    fn runtime(&self) -> Runtime;
    fn supports(&self, capability: Capability<'_>) -> bool;
    /// Optional target-sensitive checks, for example accepting click on buttons.
    /// Called only after `supports` accepts the operation.
    fn validate_binding(
        &self,
        _template: &Template,
        _capability: Capability<'_>,
        _anchor: Anchor,
        _origin: &Origin,
    ) -> Result<(), ExtractError> {
        Ok(())
    }
    /// Validate all static elements and attributes, including unbound ones.
    fn validate(&self, template: &Template) -> Result<(), ExtractError>;
    /// Expression returning `Result<Scope, Error>`, with `parent` in lexical scope.
    fn mount(&self, template: &Template) -> TokenStream;
    fn operation(&self, operation: Operation) -> TokenStream;
    /// Emit the renderer's own mounting trait implementation or entry function.
    fn component(&self, component: ComponentCode) -> TokenStream;
}

/// Rust plus validated authored line mappings; serialize both into the build output.
pub struct GeneratedSource {
    pub rust: String,
    pub source_map: SourceMap,
}

/// Compile ordinary template HTML, independently of browser delivery metadata.
/// Use ordinary Rust modules and `template!(backend = "name", "path.html")`.
/// Inline/external Rust scripts are deliberately outside this bounded entry point.
pub fn generate(source: &str, backend: &dyn Backend) -> Result<GeneratedSource, ExtractError> {
    crate::bindings::generate_backend(source, backend)
}
