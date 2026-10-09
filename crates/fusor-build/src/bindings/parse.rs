//! HTML traversal and validation. This module does not generate Rust code.
mod component_modules;
mod materialize;
mod native_attributes;
mod validate;

use component_modules::PendingModule;

use super::interpolation::interpolations;
use super::ir::*;
use super::markup;
use super::tag_input::TagInput;
use super::tags::{BuiltIn, foreign_element, table_structure, text_only_element, void_element};
use super::tokens::Rust;
use crate::{ExtractError, RustBlock, error};
use fusor::template::{self, ChildPolicy, ComponentId, ElementId, MountId, RootKind, TextId};
use html5gum::{EndTag, HtmlString, Span, Spanned, StartTag, Token};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

// Messages reported from more than one place.
const MIXED_CONTENT: &str = "do not mix named content and ordinary children in one invocation";
const OWNED_EMPTY: &str = "an owned host or hydrated component must be empty; its registered renderer supplies its contents";
const CONTROL_CHILDREN: &str = "Match accepts only Case children; Else must be last in If";
pub(super) const HYDRATE_SERVER: &str = "hydrate requires a server-rendered component";

// Async wrappers collect a single native root; a second root stays invalid
// until closing, preserving the authored closing-tag diagnostic.
enum AsyncRoot {
    Missing,
    One(ElementId),
    Multiple,
}

/// An `Async` or `Await` tag; its bindings start at `start` in the owner.
struct AsyncFrame {
    start: usize,
    value: Rust,
    kind: RegionKind,
    root: AsyncRoot,
}

/// A `rust:async` or `rust:await` element; its bindings start at `start`.
struct Region {
    start: usize,
    node: ElementId,
    value: Rust,
    kind: RegionKind,
}

#[derive(Clone, Copy)]
struct BranchRef {
    owner: usize,
    binding: usize,
}

/// A case of a Branch: its pattern, the names it binds, and the lexical locals
/// and route aliases its body captures.
struct NewCase {
    pattern: Rust,
    names: Vec<Rust>,
    async_locals: Vec<Rust>,
    route_locals: Vec<Rust>,
}

/// Open a body for `case` and add the case to its Branch. The body renders in
/// the Branch owner's place and reads the Branch's Await snapshots.
fn push_case(
    components: &mut Vec<Component>,
    first_component: usize,
    branch: BranchRef,
    offset: usize,
    case: NewCase,
) -> usize {
    let owner = &components[branch.owner];
    let Binding::Branch { snapshots, .. } = &owner.bindings[branch.binding] else {
        unreachable!("branch binding")
    };
    let index = components.len();
    let id = ComponentId::new(first_component + index);
    let body = Component {
        row_locals: owner.row_locals.clone(),
        async_locals: case.async_locals,
        route_locals: case.route_locals,
        snapshot_locals: snapshots.iter().map(|(name, _)| name.clone()).collect(),
        ..Component::new(
            id,
            Rust::ident(&format!("__FusorBranch{}", id.index()), offset),
            ComponentShape::Fragment(owner.ty.clone()),
            owner.render,
            offset..offset,
        )
    };
    components.push(body);
    let Binding::Branch { cases, .. } = &mut components[branch.owner].bindings[branch.binding]
    else {
        unreachable!("branch binding")
    };
    cases.push(CaseBranch {
        pattern: case.pattern,
        names: case.names,
        body: index,
    });
    index
}

/// Whether an option opening here belongs to a bound select.
fn in_bound_select(stack: &[Frame]) -> bool {
    stack
        .iter()
        .rev()
        .find(|frame| frame.name != "optgroup")
        .is_some_and(|frame| frame.bound() == Bound::Options)
}

/// If and Match need an ordinary container: the HTML parser relocates or
/// reinterprets content in tables, selects, text areas and foreign content.
fn branch_allowed(stack: &[Frame], parent: Option<&Frame>) -> bool {
    !parent.is_some_and(|frame| {
        matches!(frame.kind, FrameKind::Router { .. }) || frame.requires_native_root()
    }) && !stack.iter().any(|frame| {
        foreign_element(&frame.name)
            || table_structure(&frame.name)
            || matches!(frame.name.as_str(), "select" | "option" | "textarea")
    })
}

/// Each enclosing Await value, and how a case body reads it: snapshot locals
/// are signals, the others are cloned.
fn branch_snapshots(stack: &[Frame], snapshot_locals: &[Rust]) -> Vec<(Rust, Rust)> {
    stack
        .iter()
        .filter_map(|frame| match &frame.kind {
            FrameKind::Async(region) => region.kind.alias().cloned(),
            _ => None,
        })
        .map(|alias| {
            let method = if snapshot_locals
                .iter()
                .any(|local| local.same_tokens(&alias))
            {
                "get"
            } else {
                "clone"
            };
            let method = proc_macro2::Ident::new(method, proc_macro2::Span::call_site());
            let read = alias.derived(quote::quote! { #alias.#method() });
            (alias, read)
        })
        .collect()
}

/// The Branch a Case (in Match) or Else (in If) belongs to.
fn case_parent(parent: Option<&Frame>, builtin: Option<BuiltIn>) -> Option<BranchRef> {
    match (parent.map(|frame| &frame.kind), builtin) {
        (Some(FrameKind::Control(ControlFrame::Match { branch })), Some(BuiltIn::Case))
        | (Some(FrameKind::Control(ControlFrame::If { branch, .. })), Some(BuiltIn::Else)) => {
            Some(*branch)
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum IfPhase {
    Then,
    Else,
}

enum ControlFrame {
    If { branch: BranchRef, phase: IfPhase },
    Else,
    Match { branch: BranchRef },
    Case { aliases: Vec<Rust> },
}

impl Frame {
    /// A built-in or component closing tag must repeat its opening spelling.
    /// Returns that spelling and what replaces the closing tag.
    fn closing(&self) -> Option<(&str, &'static str)> {
        let builtin = || {
            BuiltIn::classify(&self.name)
                .expect("built-in frame")
                .spelling()
        };
        Some(match &self.kind {
            FrameKind::Element(_) => return None,
            FrameKind::Invocation(invocation) => (invocation.authored.as_str(), ""),
            FrameKind::Hydrated { authored } => (authored.as_str(), "</div>"),
            FrameKind::ForEach => (builtin(), "</template>"),
            FrameKind::Control(_)
            | FrameKind::Async(_)
            | FrameKind::App
            | FrameKind::Children
            | FrameKind::Router { .. }
            | FrameKind::Route { .. } => (builtin(), ""),
        })
    }
}

// Tag identity, lexical owner and the element cursor are traversal metadata.
// Feature payloads are exclusive; independent native-element properties coexist.
struct Frame {
    name: String,
    owner: Option<usize>,
    node: ElementId,
    kind: FrameKind,
}

enum FrameKind {
    Element(ElementFrame),
    Control(ControlFrame),
    Async(AsyncFrame),
    App,
    ForEach,
    Children,
    Router { binding: usize },
    Route { alias: Option<Rust> },
    Invocation(InvocationFrame),
    Hydrated { authored: String },
}

struct ElementFrame {
    text_host: Option<TextHost>,
    inert: bool,
    owns_children: bool,
    bound: Bound,
    component_root: bool,
    region: Option<Region>,
}

/// What `bind` requires of an element's contents.
#[derive(Clone, Copy, PartialEq)]
enum Bound {
    Free,
    /// A bound textarea, whose value the binding owns.
    Empty,
    /// A bound select, whose options the binding chooses.
    Options,
    /// An option of a bound select without a `value`: its static text is its value.
    StaticText,
}

struct InvocationFrame {
    binding: usize,
    authored: String,
    caller: usize,
}

impl Frame {
    fn owned(name: String, owner: usize, node: ElementId, kind: FrameKind) -> Self {
        Self {
            name,
            owner: Some(owner),
            node,
            kind,
        }
    }

    fn inert(&self) -> bool {
        matches!(
            self.kind,
            FrameKind::Element(ElementFrame { inert: true, .. })
        )
    }

    fn owns_children(&self) -> bool {
        matches!(
            self.kind,
            FrameKind::Children
                | FrameKind::Hydrated { .. }
                | FrameKind::Element(ElementFrame {
                    owns_children: true,
                    ..
                })
        )
    }

    fn is_template_or_app(&self) -> bool {
        matches!(self.kind, FrameKind::App)
            || (self.name == "template"
                && matches!(
                    self.kind,
                    FrameKind::Element(ElementFrame {
                        component_root: true,
                        ..
                    })
                ))
    }

    fn requires_native_root(&self) -> bool {
        self.is_template_or_app() || matches!(self.kind, FrameKind::ForEach)
    }

    /// Why non-whitespace text cannot appear directly inside this frame.
    fn text_rejection(&self, named_content: bool) -> Option<&'static str> {
        match &self.kind {
            FrameKind::Control(
                ControlFrame::Match { .. }
                | ControlFrame::If {
                    phase: IfPhase::Else,
                    ..
                },
            ) => Some(CONTROL_CHILDREN),
            FrameKind::Router { .. } => Some("Router accepts Route children, not text"),
            FrameKind::Invocation(_) if named_content => Some(MIXED_CONTENT),
            FrameKind::Element(ElementFrame {
                bound: Bound::Empty,
                ..
            }) => Some(
                "bind owns the textarea value; leave its contents empty and initialize the value in Rust",
            ),
            _ if self.owns_children() => Some(OWNED_EMPTY),
            _ => None,
        }
    }

    fn bound(&self) -> Bound {
        match &self.kind {
            FrameKind::Element(element) => element.bound,
            _ => Bound::Free,
        }
    }

    fn rendered_owner(&self) -> Option<usize> {
        self.owner
            .filter(|_| !self.inert() && !self.owns_children())
    }

    fn in_coherent_region(&self) -> bool {
        matches!(
            self.kind,
            FrameKind::Async(_)
                | FrameKind::Element(ElementFrame {
                    region: Some(_),
                    ..
                })
        )
    }

    fn lexical_aliases(&self, include_await: bool) -> &[Rust] {
        match &self.kind {
            FrameKind::Control(ControlFrame::Case { aliases }) => aliases,
            FrameKind::Route { alias } => alias.as_slice(),
            FrameKind::Async(region) if include_await => {
                region.kind.alias().map_or(&[], std::slice::from_ref)
            }
            _ => &[],
        }
    }
}

// What element_owner decided about a native element.
#[derive(Clone, Copy)]
struct ElementOwner {
    owner: Option<usize>,
    inert: bool,
    component_id: Option<ComponentId>,
    rust_script: bool,
}

fn replace_tag(span: &Span<usize>, replacement: impl Into<String>) -> Edit {
    Edit {
        range: span.start..span.end,
        replacement: replacement.into(),
    }
}

struct ElementRegion {
    id: ElementId,
    region: Option<Region>,
}

struct ContentOwner {
    fragment: usize,
    invocation: (usize, usize),
}

struct TextHost {
    opening: Range<usize>,
    opening_edit: Option<usize>,
    element: Option<ElementId>,
}

// Per-tag context: the tag's spelling, its nearest non-Async parent (an index into
// the frame stack) and the lexical captures visible at this position.
struct TagContext<'a> {
    name: String,
    authored_name: &'a str,
    parent: Option<usize>,
    builtin: Option<BuiltIn>,
    lexicals: Lexicals,
}

// Route parameters and Case handles are ordinary lexical captures; Await
// snapshots need refreshable retained storage.
struct Lexicals {
    snapshot_locals: Vec<Rust>,
    route_locals: Vec<Rust>,
    async_locals: Vec<Rust>,
}

impl Lexicals {
    fn case(
        &self,
        source: &str,
        tag: &StartTag<usize>,
        builtin: BuiltIn,
        rows: &[(Rust, Rust)],
    ) -> Result<NewCase, ExtractError> {
        let input = TagInput::new(source, tag, builtin.spelling());
        let (pattern, names) = if builtin == BuiltIn::Case {
            super::control::pattern(&input)?
        } else {
            input.accepts(&[], "no attributes")?;
            (
                Rust::synthetic(quote::quote! { false }, tag.span.start),
                Vec::new(),
            )
        };
        for alias in &names {
            if self.shadows(alias, rows) {
                return Err(error(
                    source,
                    alias.offset,
                    "Case binding cannot shadow an enclosing local",
                ));
            }
        }
        Ok(NewCase {
            pattern,
            async_locals: self.async_locals.iter().chain(&names).cloned().collect(),
            route_locals: self.route_locals.iter().chain(&names).cloned().collect(),
            names,
        })
    }

    fn with_route(mut self, alias: Option<&Rust>) -> Self {
        if let Some(alias) = alias {
            self.async_locals.push(alias.clone());
            self.route_locals.push(alias.clone());
        }
        self
    }

    fn shadows(&self, name: &Rust, rows: &[(Rust, Rust)]) -> bool {
        self.async_locals
            .iter()
            .chain(rows.iter().flat_map(|(item, index)| [item, index]))
            .any(|other| other.same_tokens(name))
    }

    /// A component opened here captures every lexical local visible at its tag.
    fn open(&self, component: Component) -> Component {
        Component {
            async_locals: self.async_locals.clone(),
            route_locals: self.route_locals.clone(),
            snapshot_locals: self.snapshot_locals.clone(),
            ..component
        }
    }
}

impl<'a> TagContext<'a> {
    fn new(
        source: &'a str,
        tag: &StartTag<usize>,
        stack: &[Frame],
        components: &[Component],
    ) -> Self {
        let parent = stack
            .iter()
            .rposition(|frame| !matches!(frame.kind, FrameKind::Async(_)));
        let snapshot_locals = parent
            .and_then(|index| stack[index].owner)
            .map(|owner| components[owner].snapshot_locals.clone())
            .unwrap_or_default();
        let route_locals = stack
            .iter()
            .flat_map(|frame| frame.lexical_aliases(false))
            .cloned()
            .collect();
        let async_locals = stack
            .iter()
            .flat_map(|frame| frame.lexical_aliases(true))
            .cloned()
            .collect();
        let name = String::from_utf8_lossy(&tag.name).into_owned();
        Self {
            builtin: BuiltIn::classify(&name),
            name,
            authored_name: super::tags::name(source, tag.span.start),
            parent,
            lexicals: Lexicals {
                snapshot_locals,
                route_locals,
                async_locals,
            },
        }
    }

    /// Whether a built-in is written with its exact spelling.
    fn spelled(&self) -> bool {
        self.builtin
            .is_some_and(|builtin| self.authored_name == builtin.spelling())
    }

    fn parent<'s>(&self, stack: &'s [Frame]) -> Option<&'s Frame> {
        self.parent.map(|index| &stack[index])
    }
}

struct Parser<'a> {
    source: &'a str,
    blocks: &'a [RustBlock],
    first_component: usize,
    foreach_hosts: BTreeSet<usize>,
    stack: Vec<Frame>,
    components: Vec<Component>,
    named_content: BTreeSet<usize>,
    component_types: BTreeSet<String>,
    template_roots: BTreeMap<usize, usize>,
    edits: Vec<Edit>,
    text_edits: BTreeMap<(usize, usize), (TextId, usize)>,
    node: usize,
    slot: usize,
    mount: usize,
    pending_module: Option<(usize, PendingModule)>,
}

pub(super) fn parse(
    source: &str,
    blocks: &[RustBlock],
    first_component: usize,
) -> Result<Plan, ExtractError> {
    let mut parser = Parser {
        source,
        blocks,
        first_component,
        foreach_hosts: super::foreach::hosts(source)?,
        stack: Vec::new(),
        components: Vec::new(),
        named_content: BTreeSet::new(),
        component_types: BTreeSet::new(),
        template_roots: BTreeMap::new(),
        edits: Vec::new(),
        text_edits: BTreeMap::new(),
        node: 0,
        slot: 0,
        mount: 0,
        pending_module: None,
    };
    for token in crate::html::tokens(source) {
        parser.token(token)?;
    }
    parser.finish()
}

impl Parser<'_> {
    fn token(&mut self, token: Token<usize>) -> Result<(), ExtractError> {
        if self.pending_module.is_some() {
            if let Token::EndTag(tag) = &token {
                if &*tag.name == b"script" {
                    let (owner, pending) = self
                        .pending_module
                        .take()
                        .expect("a pending script consumes its closing tag");
                    let (module, edit) =
                        pending.finish(self.source, tag.span.start..tag.span.end)?;
                    self.edits.push(edit);
                    self.components[owner].javascript = Some(module);
                }
            }
            return Ok(());
        }
        match token {
            Token::StartTag(tag) => self.start_tag(tag),
            Token::EndTag(tag) => self.end_tag(tag),
            Token::String(text) => self.text(text),

            Token::Comment(comment)
                if template::reserved_comment(&String::from_utf8_lossy(&comment)) =>
            {
                Err(error(
                    self.source,
                    comment.span.start,
                    "fusor: comment markers are reserved for the HTML compiler",
                ))
            }
            _ => Ok(()),
        }
    }

    fn start_tag(&mut self, tag: StartTag<usize>) -> Result<(), ExtractError> {
        let source = self.source;
        let cx = TagContext::new(source, &tag, &self.stack, &self.components);
        self.check_control_child(&tag, &cx)?;
        if self.start_module(&tag, &cx)? {
            return Ok(());
        }
        let builtin = cx.builtin;
        let spelled = cx.spelled();
        if matches!(builtin, Some(BuiltIn::Async | BuiltIn::Await)) {
            return self.start_async(tag, cx);
        }
        self.check_async_root_child(&tag, &cx)?;
        if builtin == Some(BuiltIn::App) {
            if !spelled {
                return Err(error(
                    source,
                    tag.span.start,
                    "the built-in application boundary is spelled App",
                ));
            }
            return self.start_app(tag, cx);
        }
        // foreach::hosts has already rejected every other spelling of ForEach.
        if builtin == Some(BuiltIn::ForEach) {
            return self.start_foreach(tag, cx);
        }
        self.check_named_content_child(&tag, &cx)?;
        match builtin {
            Some(BuiltIn::If | BuiltIn::Else | BuiltIn::Match | BuiltIn::Case) => {
                return self.start_control(tag, cx);
            }
            Some(BuiltIn::Children) if !spelled => {
                return Err(error(
                    source,
                    tag.span.start,
                    "the built-in is spelled Children",
                ));
            }
            Some(BuiltIn::Children) => return self.start_children(tag, cx),
            Some(BuiltIn::Router | BuiltIn::Route) => return self.start_router(tag, cx),
            _ => {}
        }
        if cx
            .parent(&self.stack)
            .is_some_and(|frame| matches!(frame.kind, FrameKind::Router { .. }))
        {
            return Err(error(
                source,
                tag.span.start,
                "Router accepts only direct Route children",
            ));
        }
        if tag.attributes.contains_key(b"hydrate".as_slice()) {
            return self.start_hydrated(tag, cx);
        }
        if super::tags::is_component(cx.authored_name) {
            return self.start_invocation(tag, cx);
        }
        self.start_element(tag, cx)
    }

    fn check_control_child(
        &self,
        tag: &StartTag<usize>,
        cx: &TagContext,
    ) -> Result<(), ExtractError> {
        let source = self.source;
        let parent = cx.parent(&self.stack);
        if parent.is_some_and(|frame| {
            (matches!(frame.kind, FrameKind::Control(ControlFrame::Match { .. }))
                && cx.builtin != Some(BuiltIn::Case))
                || matches!(
                    frame.kind,
                    FrameKind::Control(ControlFrame::If {
                        phase: IfPhase::Else,
                        ..
                    })
                )
        }) {
            return Err(error(source, tag.span.start, CONTROL_CHILDREN));
        }
        Ok(())
    }

    fn start_module(
        &mut self,
        tag: &StartTag<usize>,
        cx: &TagContext,
    ) -> Result<bool, ExtractError> {
        let source = self.source;
        let Self {
            stack,
            components,
            pending_module,
            ..
        } = self;
        let parent = cx.parent(stack);
        let name = &cx.name;
        if name == "script"
            && tag
                .attributes
                .get(b"type".as_slice())
                .is_some_and(|value| value.as_ref().trim_ascii().eq_ignore_ascii_case(b"module"))
        {
            if let Some(owner) = parent.and_then(|frame| frame.owner) {
                let component = &components[owner];
                if !parent.is_some_and(|frame| frame.is_template_or_app())
                    || component.capture().is_some()
                    || component.inline()
                    || component.render != RenderTarget::Browser
                    || stack.iter().any(|frame| frame.in_coherent_region())
                {
                    return Err(error(
                        source,
                        tag.span.start,
                        "component modules must be direct children of a browser template or App, outside Async, named content and server/island delivery",
                    ));
                }
                *pending_module = Some((owner, PendingModule::begin(source, tag, component)?));
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn start_async(&mut self, tag: StartTag<usize>, cx: TagContext) -> Result<(), ExtractError> {
        let source = self.source;
        let spelled = cx.spelled();
        let parent = cx.parent(&self.stack);
        let awaiting = cx.builtin == Some(BuiltIn::Await);
        if !spelled {
            return Err(error(
                source,
                tag.span.start,
                "async built-ins are spelled Async and Await",
            ));
        }
        let owner = parent.and_then(Frame::rendered_owner).ok_or_else(|| {
            error(
                source,
                tag.span.start,
                "Async and Await require rendered component HTML",
            )
        })?;
        if self.components[owner].render != RenderTarget::Browser {
            return Err(error(
                source,
                tag.span.start,
                "Async and Await require browser templates; use an island for server-rendered async views",
            ));
        }
        if !awaiting && self.stack.iter().any(|frame| frame.in_coherent_region()) {
            return Err(error(
                source,
                tag.span.start,
                "nested Async boundaries are unsupported; Await automatically uses its enclosing boundary",
            ));
        }
        let input = TagInput::new(source, &tag, cx.builtin.expect("async built-in").spelling());
        let (value, kind) = super::async_tags::inputs(&input, awaiting)?;
        if kind.alias().is_some_and(|alias| {
            cx.lexicals
                .shadows(alias, &self.components[owner].row_locals)
        }) {
            return Err(error(
                source,
                tag.span.start,
                "choose a distinct Await name; it cannot shadow an enclosing Await or ForEach binding",
            ));
        }
        self.edits.push(replace_tag(&tag.span, String::new()));
        self.stack.push(Frame::owned(
            cx.name,
            owner,
            ElementId::new(self.node),
            FrameKind::Async(AsyncFrame {
                start: self.components[owner].bindings.len(),
                value,
                kind,
                root: AsyncRoot::Missing,
            }),
        ));
        Ok(())
    }

    fn check_async_root_child(
        &self,
        tag: &StartTag<usize>,
        cx: &TagContext,
    ) -> Result<(), ExtractError> {
        let source = self.source;
        let stack = &self.stack;
        let name = &cx.name;
        let authored_name = cx.authored_name;
        if stack
            .last()
            .is_some_and(|frame| matches!(frame.kind, FrameKind::Async(_)))
            && (super::tags::is_component(authored_name)
                || matches!(name.as_str(), "template" | "script" | "style" | "select"))
        {
            return Err(error(
                source,
                tag.span.start,
                "Async and Await require one native rendered HTML root; place components inside that root",
            ));
        }
        Ok(())
    }

    fn start_app(&mut self, tag: StartTag<usize>, cx: TagContext) -> Result<(), ExtractError> {
        let source = self.source;
        let first_component = self.first_component;
        let Self {
            stack,
            components,
            template_roots,
            edits,
            node,
            ..
        } = self;
        let TagContext {
            name,
            parent,
            lexicals,
            ..
        } = cx;
        let parent = parent.map(|index| &stack[index]);
        if parent.is_some_and(|frame| {
            frame.owner.is_some() || frame.inert() || matches!(frame.kind, FrameKind::Invocation(_))
        }) || stack.iter().any(|frame| {
            foreign_element(&frame.name) || matches!(frame.name.as_str(), "select" | "template")
        }) {
            return Err(error(
                source,
                tag.span.start,
                "App must be a top-level application boundary, outside components and inert or foreign HTML",
            ));
        }
        let input = TagInput::new(source, &tag, BuiltIn::App.spelling());
        input.closed()?;
        input.accepts(
            &["state"],
            "only state=\"{{ Rust expression }}\"; put HTML attributes on its native root",
        )?;
        let state = input.expression("state")?;
        let id = ComponentId::new(first_component + components.len());
        let owner = components.len();
        components.push(lexicals.open(Component::new(
            id,
            Rust::ident("__FusorApp", tag.span.start),
            ComponentShape::App(state),
            RenderTarget::Browser,
            tag.span.start..tag.span.end,
        )));
        template_roots.insert(owner, 0);
        edits.push(replace_tag(&tag.span, String::new()));
        stack.push(Frame::owned(
            name,
            owner,
            ElementId::new(*node),
            FrameKind::App,
        ));
        Ok(())
    }

    fn foreach_parent(
        &self,
        tag: &StartTag<usize>,
        cx: &TagContext,
    ) -> Result<(usize, ElementId), ExtractError> {
        let source = self.source;
        let parent = cx.parent(&self.stack);
        let parent = parent.ok_or_else(|| {
            error(
                source,
                tag.span.start,
                "ForEach requires a native HTML list container",
            )
        })?;
        let caller = parent.rendered_owner().ok_or_else(|| {
            error(
                source,
                tag.span.start,
                "ForEach requires rendered HTML inside a component",
            )
        })?;
        if parent.is_template_or_app() {
            return Err(error(
                source,
                tag.span.start,
                "put ForEach inside a native HTML container",
            ));
        }
        Ok((caller, parent.node))
    }

    fn start_foreach(&mut self, tag: StartTag<usize>, cx: TagContext) -> Result<(), ExtractError> {
        let source = self.source;
        let first_component = self.first_component;
        let (caller, node) = self.foreach_parent(&tag, &cx)?;
        let (items, key, item, index) =
            super::foreach::inputs(&TagInput::new(source, &tag, BuiltIn::ForEach.spelling()))?;
        if cx
            .lexicals
            .async_locals
            .iter()
            .any(|alias| alias.same_tokens(&item) || alias.same_tokens(&index))
        {
            return Err(error(
                source,
                tag.span.start,
                "ForEach names cannot shadow an enclosing Await binding",
            ));
        }

        let id = ComponentId::new(first_component + self.components.len());
        let body = self.components.len();
        let mut row_locals = self.components[caller].row_locals.clone();
        row_locals.push((item, index));
        let render = self.components[caller].render;
        self.components[caller].bindings.push(Binding::ForEach {
            node,
            items,
            key,
            body,
        });
        self.components.push(Component {
            row_locals,
            ..cx.lexicals.open(Component::new(
                id,
                Rust::ident(&format!("__FusorForEach{}", id.index()), tag.span.start),
                ComponentShape::Row,
                render,
                tag.span.start..tag.span.end,
            ))
        });
        self.template_roots.insert(body, 0);
        self.edits
            .push(replace_tag(&tag.span, markup::template_open(id)));
        self.stack.push(Frame::owned(
            "foreach".into(),
            body,
            node,
            FrameKind::ForEach,
        ));
        Ok(())
    }

    fn check_named_content_child(
        &self,
        tag: &StartTag<usize>,
        cx: &TagContext,
    ) -> Result<(), ExtractError> {
        let source = self.source;
        let named_content = &self.named_content;
        let parent = cx.parent(&self.stack);
        let name = &cx.name;
        let projected = tag.attributes.get(b"rust:content".as_slice());
        let caller = parent.and_then(|frame| match &frame.kind {
            FrameKind::Invocation(invocation) => Some(invocation),
            _ => None,
        });
        if projected.is_some() && (caller.is_none() || name != "template") {
            return Err(error(
                source,
                tag.span.start,
                "named content must be declared directly inside a component tag",
            ));
        }
        if projected.is_none()
            && parent.is_some_and(|frame| {
                matches!(frame.kind, FrameKind::Invocation(_))
                    && frame
                        .owner
                        .is_some_and(|owner| named_content.contains(&owner))
            })
        {
            return Err(error(source, tag.span.start, MIXED_CONTENT));
        }
        Ok(())
    }

    fn start_control(&mut self, tag: StartTag<usize>, cx: TagContext) -> Result<(), ExtractError> {
        let source = self.source;
        if !cx.spelled() || tag.self_closing {
            return Err(error(
                source,
                tag.span.start,
                "If, Else, Match and Case require exact spelling and explicit closing tags",
            ));
        }
        let owner = cx
            .parent(&self.stack)
            .and_then(Frame::rendered_owner)
            .ok_or_else(|| {
                error(
                    source,
                    tag.span.start,
                    "control flow requires rendered component HTML",
                )
            })?;
        let frame = if matches!(cx.builtin, Some(BuiltIn::If | BuiltIn::Match)) {
            self.open_branch(tag, cx, owner)?
        } else {
            self.open_case(tag, cx)?
        };
        self.stack.push(frame);
        Ok(())
    }

    /// `If` and `Match` add a Branch binding with a mount point in the caller.
    fn open_branch(
        &mut self,
        tag: StartTag<usize>,
        cx: TagContext,
        owner: usize,
    ) -> Result<Frame, ExtractError> {
        let source = self.source;
        let first_component = self.first_component;
        let parent = cx.parent(&self.stack);
        if !branch_allowed(&self.stack, parent) {
            return Err(error(
                source,
                tag.span.start,
                "If and Match belong inside an ordinary native HTML container, outside table/select/SVG/MathML parsing contexts",
            ));
        }
        let value = super::control::value(source, &tag, cx.builtin.expect("control built-in"))?;
        let point = MountId::new(self.mount);
        self.mount += 1;
        let binding = self.components[owner].bindings.len();
        let snapshots = branch_snapshots(&self.stack, &cx.lexicals.snapshot_locals);
        self.components[owner].bindings.push(Binding::Branch {
            point,
            value,
            cases: Vec::new(),
            snapshots,
        });
        let branch = BranchRef { owner, binding };
        // If's own content is its first case; Match's cases are its children.
        let (frame_owner, control) = if cx.builtin == Some(BuiltIn::If) {
            let case = NewCase {
                pattern: Rust::synthetic(quote::quote! { true }, tag.span.start),
                names: Vec::new(),
                async_locals: cx.lexicals.async_locals.clone(),
                route_locals: cx.lexicals.route_locals.clone(),
            };
            let body = push_case(
                &mut self.components,
                first_component,
                branch,
                tag.span.end,
                case,
            );
            let control = ControlFrame::If {
                branch,
                phase: IfPhase::Then,
            };
            (body, control)
        } else {
            (owner, ControlFrame::Match { branch })
        };
        self.edits
            .push(replace_tag(&tag.span, markup::mount_point(point)));
        Ok(Frame::owned(
            cx.name,
            frame_owner,
            ElementId::new(self.node),
            FrameKind::Control(control),
        ))
    }

    /// `Case` and `Else` add a case body to the Branch their parent opened.
    fn open_case(&mut self, tag: StartTag<usize>, cx: TagContext) -> Result<Frame, ExtractError> {
        let source = self.source;
        let first_component = self.first_component;
        let parent = cx.parent(&self.stack);
        let branch = case_parent(parent, cx.builtin).ok_or_else(|| {
            error(
                source,
                tag.span.start,
                "Case must be a direct child of Match; Else must be the final direct child of If",
            )
        })?;
        let caller = branch.owner;
        let case = cx.lexicals.case(
            source,
            &tag,
            cx.builtin.expect("control built-in"),
            &self.components[caller].row_locals,
        )?;
        let names = case.names.clone();
        let frame_owner = push_case(
            &mut self.components,
            first_component,
            branch,
            tag.span.end,
            case,
        );
        let control = if cx.builtin == Some(BuiltIn::Case) {
            ControlFrame::Case { aliases: names }
        } else {
            ControlFrame::Else
        };
        if cx.builtin == Some(BuiltIn::Else) {
            self.start_else(tag.span.start);
        }
        self.edits.push(replace_tag(&tag.span, String::new()));
        Ok(Frame::owned(
            cx.name,
            frame_owner,
            ElementId::new(self.node),
            FrameKind::Control(control),
        ))
    }

    fn start_else(&mut self, offset: usize) {
        let parent = self.stack.last_mut().expect("If frame");
        self.components[parent.owner.expect("an If frame owns its then fragment")]
            .range
            .end = offset;
        let FrameKind::Control(ControlFrame::If { phase, .. }) = &mut parent.kind else {
            unreachable!("validated If parent")
        };
        *phase = IfPhase::Else;
    }

    fn start_children(&mut self, tag: StartTag<usize>, cx: TagContext) -> Result<(), ExtractError> {
        let source = self.source;
        let Self {
            stack,
            components,
            edits,
            node,
            mount,
            ..
        } = self;
        let TagContext { name, parent, .. } = cx;
        let parent = parent.map(|index| &stack[index]);
        let owner = parent.and_then(Frame::rendered_owner).ok_or_else(|| {
            error(
                source,
                tag.span.start,
                "Children requires a reusable component template",
            )
        })?;
        if tag.self_closing || !tag.attributes.is_empty() {
            return Err(error(
                source,
                tag.span.start,
                "write <Children></Children> without attributes",
            ));
        }
        if parent.is_some_and(|frame| frame.requires_native_root()) {
            return Err(error(
                source,
                tag.span.start,
                "Children belongs inside the component's native HTML root",
            ));
        }
        if components[owner].app().is_some() || !components[owner].row_locals.is_empty() {
            return Err(error(
                source,
                tag.span.start,
                "Children requires a reusable component and cannot be repeated inside ForEach",
            ));
        }
        let point = MountId::new(*mount);
        *mount += 1;
        components[owner].bindings.push(Binding::Children {
            point,
            origin: Rust::ident("Children", tag.span.start),
        });
        edits.push(replace_tag(&tag.span, markup::mount_point(point)));
        stack.push(Frame::owned(
            name,
            owner,
            ElementId::new(*node),
            FrameKind::Children,
        ));
        Ok(())
    }

    fn start_router(&mut self, tag: StartTag<usize>, cx: TagContext) -> Result<(), ExtractError> {
        let source = self.source;
        if !cx.spelled() || tag.self_closing {
            return Err(error(
                source,
                tag.span.start,
                "Router and Route require exact spelling and explicit closing tags",
            ));
        }
        let owner = cx
            .parent(&self.stack)
            .and_then(|frame| frame.owner)
            .ok_or_else(|| {
                error(
                    source,
                    tag.span.start,
                    "Router and Route require component HTML",
                )
            })?;
        if self.components[owner].render != RenderTarget::Browser
            || self.stack.iter().any(|frame| frame.in_coherent_region())
        {
            return Err(error(
                source,
                tag.span.start,
                "Router requires browser HTML outside coherent Async regions",
            ));
        }
        if cx.builtin == Some(BuiltIn::Router) {
            self.open_router(tag, cx, owner)
        } else {
            self.open_route(tag, cx, owner)
        }
    }

    fn open_router(
        &mut self,
        tag: StartTag<usize>,
        cx: TagContext,
        owner: usize,
    ) -> Result<(), ExtractError> {
        let source = self.source;
        let Self {
            stack,
            components,
            edits,
            node,
            mount,
            ..
        } = self;
        let TagContext { name, parent, .. } = cx;
        let parent = parent.map(|index| &stack[index]);
        if !tag.attributes.is_empty()
            || stack
                .iter()
                .any(|frame| foreign_element(&frame.name) || frame.name == "select")
            || parent.is_some_and(|frame| {
                matches!(frame.kind, FrameKind::Router { .. })
                    || frame.owns_children()
                    || frame.is_template_or_app()
            })
        {
            return Err(error(
                source,
                tag.span.start,
                "Router takes Route children and belongs inside a native HTML root",
            ));
        }
        let point = MountId::new(*mount);
        *mount += 1;
        let binding = components[owner].bindings.len();
        components[owner].bindings.push(Binding::Router {
            point,
            origin: Rust::ident("Router", tag.span.start),
            routes: Vec::new(),
        });
        edits.push(replace_tag(&tag.span, markup::mount_point(point)));
        stack.push(Frame::owned(
            name,
            owner,
            ElementId::new(*node),
            FrameKind::Router { binding },
        ));
        Ok(())
    }

    fn open_route(
        &mut self,
        tag: StartTag<usize>,
        cx: TagContext,
        owner: usize,
    ) -> Result<(), ExtractError> {
        let source = self.source;
        let first_component = self.first_component;
        let parent = cx.parent(&self.stack);
        let Some(FrameKind::Router { binding }) = parent.map(|frame| &frame.kind) else {
            return Err(error(
                source,
                tag.span.start,
                "Route must be a direct child of Router",
            ));
        };
        let binding = *binding;
        let super::router_tags::Declaration { path, alias, names } =
            super::router_tags::route(&TagInput::new(source, &tag, BuiltIn::Route.spelling()))?;
        let Binding::Router { routes, .. } = &self.components[owner].bindings[binding] else {
            unreachable!()
        };
        super::router_tags::validate(source, tag.span.start, routes, path.as_deref())?;
        if alias.as_ref().is_some_and(|alias| {
            cx.lexicals
                .shadows(alias, &self.components[owner].row_locals)
        }) {
            return Err(error(
                source,
                tag.span.start,
                "Route binding cannot shadow an enclosing local",
            ));
        }
        let index = self.components.len();
        let id = ComponentId::new(first_component + index);
        self.components
            .push(cx.lexicals.with_route(alias.as_ref()).open(Component {
                row_locals: self.components[owner].row_locals.clone(),
                ..Component::new(
                    id,
                    Rust::ident(&format!("__FusorRoute{}", id.index()), tag.span.start),
                    ComponentShape::Fragment(self.components[owner].ty.clone()),
                    RenderTarget::Browser,
                    tag.span.end..tag.span.end,
                )
            }));
        let Binding::Router { routes, .. } = &mut self.components[owner].bindings[binding] else {
            unreachable!()
        };
        routes.push(RouteBranch {
            path,
            params: alias.clone(),
            names,
            body: index,
        });
        self.edits.push(replace_tag(&tag.span, String::new()));
        self.stack.push(Frame::owned(
            cx.name,
            index,
            ElementId::new(self.node),
            FrameKind::Route { alias },
        ));
        Ok(())
    }

    fn start_hydrated(&mut self, tag: StartTag<usize>, cx: TagContext) -> Result<(), ExtractError> {
        let source = self.source;
        let Self {
            stack,
            components,
            edits,
            node,
            ..
        } = self;
        let TagContext {
            name,
            authored_name,
            parent,
            ..
        } = cx;
        let parent = parent.map(|index| &stack[index]);
        let owner = parent.and_then(Frame::rendered_owner).ok_or_else(|| {
            error(
                source,
                tag.span.start,
                "hydrate requires rendered HTML inside a server component",
            )
        })?;
        if components[owner].render != RenderTarget::Server {
            return Err(error(source, tag.span.start, HYDRATE_SERVER));
        }
        if parent.is_some_and(|frame| frame.requires_native_root())
            || stack.iter().any(|frame| foreign_element(&frame.name))
            || parent.is_some_and(|frame| {
                table_structure(&frame.name)
                    || matches!(
                        frame.name.as_str(),
                        "select" | "optgroup" | "p" | "head" | "html"
                    )
            })
        {
            return Err(error(
                source,
                tag.span.start,
                "a hydrated component renders a div boundary; place it inside a native HTML container that accepts div children",
            ));
        }
        let element = ElementId::new(*node);
        let replacement = super::hydration::lower(source, &tag, element, &mut components[owner])?;
        *node += 1;
        edits.push(replace_tag(&tag.span, replacement));
        stack.push(Frame::owned(
            name,
            owner,
            element,
            FrameKind::Hydrated {
                authored: authored_name.to_owned(),
            },
        ));
        Ok(())
    }

    fn invocation_owner(
        &mut self,
        tag: &StartTag<usize>,
        cx: &TagContext,
    ) -> Result<usize, ExtractError> {
        let source = self.source;
        let parent = cx.parent(&self.stack);
        let owner = parent.and_then(Frame::rendered_owner).ok_or_else(|| {
            error(
                source,
                tag.span.start,
                "component tags require rendered HTML inside a rust:component",
            )
        })?;
        if parent.is_some_and(|frame| frame.is_template_or_app()) {
            return Err(error(
                source,
                tag.span.start,
                "a template requires a native HTML root; component tags belong inside that root",
            ));
        }
        if parent.is_some_and(|frame| matches!(frame.kind, FrameKind::ForEach)) {
            *self.template_roots.entry(owner).or_insert(0) += 1;
        }
        if tag.self_closing {
            return Err(error(
                source,
                tag.span.start,
                "component tags need an explicit closing tag",
            ));
        }
        Ok(owner)
    }

    fn start_invocation(
        &mut self,
        tag: StartTag<usize>,
        cx: TagContext,
    ) -> Result<(), ExtractError> {
        let source = self.source;
        let first_component = self.first_component;
        let owner = self.invocation_owner(&tag, &cx)?;
        let parent = cx.parent(&self.stack);
        let point = MountId::new(self.mount);
        self.mount += 1;
        let binding = self.components[owner].bindings.len();
        let mut invocation = super::tags::invocation(source, &tag, point, false)?;
        if parent.is_some_and(|frame| matches!(frame.kind, FrameKind::ForEach))
            && matches!(
                &invocation,
                Binding::Invocation {
                    condition: Some(_),
                    ..
                } | Binding::Invocation { key: Some(_), .. }
            )
        {
            return Err(error(
                source,
                tag.span.start,
                "ForEach owns row identity; put conditional or separately keyed components inside a native row element",
            ));
        }
        let fragment_index = self.components.len();
        let id = ComponentId::new(first_component + fragment_index);
        let capture = self.components[owner].ty.clone();
        let render = self.components[owner].render;
        let row_locals = self.components[owner].row_locals.clone();
        self.components.push(Component {
            row_locals,
            ..cx.lexicals.open(Component::new(
                id,
                Rust::ident(&format!("__FusorChildren{}", id.index()), tag.span.start),
                ComponentShape::Fragment(capture),
                render,
                tag.span.end..tag.span.end,
            ))
        });
        if let Binding::Invocation { children, .. } = &mut invocation {
            *children = Some(fragment_index);
        }
        self.components[owner].bindings.push(invocation);
        self.edits
            .push(replace_tag(&tag.span, markup::mount_point(point)));
        self.stack.push(Frame::owned(
            cx.name,
            fragment_index,
            ElementId::new(self.node),
            FrameKind::Invocation(InvocationFrame {
                binding,
                authored: cx.authored_name.to_owned(),
                caller: owner,
            }),
        ));
        Ok(())
    }

    fn start_element(&mut self, tag: StartTag<usize>, cx: TagContext) -> Result<(), ExtractError> {
        if cx.name == "option"
            && in_bound_select(&self.stack)
            && tag.attributes.contains_key(b"selected".as_slice())
        {
            return Err(error(
                self.source,
                tag.span.start,
                "bind chooses the selected options; remove the selected attribute",
            ));
        }
        let owner = self.element_owner(&tag, &cx)?;
        let region = self.element_region(&tag, &cx, &owner)?;
        let element_id = ElementId::new(self.node);
        self.lower_element(&tag, &cx, &owner)?;
        self.mark_async_roots(&tag, &cx.name, element_id)?;
        if !void_element(&cx.name) && !tag.self_closing {
            self.push_element(
                &tag,
                cx.name,
                owner,
                ElementRegion {
                    id: element_id,
                    region,
                },
            );
        }
        Ok(())
    }

    /// Which component an element belongs to, and whether it declares one.
    fn element_owner(
        &mut self,
        tag: &StartTag<usize>,
        cx: &TagContext,
    ) -> Result<ElementOwner, ExtractError> {
        let source = self.source;
        let ElementOwner {
            mut owner,
            mut inert,
            mut component_id,
            rust_script,
        } = self.inherit_element(tag, cx)?;
        let parent = cx.parent(&self.stack);
        let caller = parent.and_then(|frame| match &frame.kind {
            FrameKind::Invocation(invocation) => Some((invocation.caller, invocation.binding)),
            _ => None,
        });
        let projected = tag.attributes.get(b"rust:content".as_slice());
        let declaration = tag.attributes.get(b"rust:component".as_slice());
        if let Some(content) = projected {
            self.check_projected(tag, owner)?;
            let index = self.open_content(
                tag,
                cx,
                content,
                ContentOwner {
                    fragment: owner.expect("fragment owner"),
                    invocation: caller.expect("validated caller"),
                },
            )?;
            owner = Some(index);
            component_id = Some(self.components[index].id);
        } else if let Some(ty) = declaration {
            if owner.is_some() || inert {
                return Err(error(
                    source,
                    tag.span.start,
                    "declare components separately; compose them with component tags",
                ));
            }

            let index = self.declare_component(tag, cx, ty)?;
            owner = Some(index);
            component_id = Some(self.components[index].id);
        } else if cx.name == "template" {
            inert = true;
        }
        Ok(ElementOwner {
            owner,
            inert,
            component_id,
            rust_script,
        })
    }

    fn inherit_element(
        &mut self,
        tag: &StartTag<usize>,
        cx: &TagContext,
    ) -> Result<ElementOwner, ExtractError> {
        let source = self.source;
        let blocks = self.blocks;
        let name = &cx.name;
        let parent = cx.parent(&self.stack);
        if parent.is_some_and(|frame| frame.owns_children()) {
            return Err(error(source, tag.span.start, OWNED_EMPTY));
        }
        if let Some(parent) = parent.filter(|parent| parent.requires_native_root()) {
            if !blocks
                .iter()
                .any(|block| block.element.start == tag.span.start)
            {
                *self
                    .template_roots
                    .entry(parent.owner.expect("component root"))
                    .or_insert(0) += 1;
            }
        }
        let owner = parent.and_then(|frame| frame.owner);
        let inert = parent.is_some_and(|frame| frame.inert());
        // Rust script source metadata is consumed by the extractor, not
        // by the component binding language, including scripts in templates.
        let rust_script = blocks
            .iter()
            .any(|block| block.element.start == tag.span.start);
        if rust_script
            && owner.is_some_and(|index| {
                self.components[index].capture().is_some()
                    || self.components[index].inline()
                    || self.components[index].app().is_some()
            })
        {
            return Err(error(
                source,
                tag.span.start,
                "Rust scripts do not belong inside App, projected content, or ForEach rows; keep state in the associated Rust module",
            ));
        }
        let app_root = parent.is_some_and(|frame| matches!(frame.kind, FrameKind::App));
        if app_root
            && (rust_script
                || foreign_element(name)
                || matches!(name.as_str(), "template" | "script" | "style"))
        {
            return Err(error(
                source,
                tag.span.start,
                "App requires a native rendered HTML root; put Rust scripts outside App",
            ));
        }
        let component_id = app_root.then(|| self.components[owner.expect("App owner")].id);
        Ok(ElementOwner {
            owner,
            inert,
            component_id,
            rust_script,
        })
    }

    /// Named content is a browser feature of reusable components.
    fn check_projected(
        &self,
        tag: &StartTag<usize>,
        owner: Option<usize>,
    ) -> Result<(), ExtractError> {
        let source = self.source;
        let components = &self.components;
        if owner.is_some_and(|index| !components[index].row_locals.is_empty()) {
            return Err(error(
                source,
                tag.span.start,
                "named content inside ForEach is not supported yet; pass the row to a reusable component",
            ));
        }
        if components[owner.expect("content owner")].render != RenderTarget::Browser
            || self.stack.iter().any(|frame| frame.in_coherent_region())
        {
            return Err(error(
                source,
                tag.span.start,
                "projected content requires a browser template outside coherent regions; pass typed data inputs to shared or coherent components",
            ));
        }
        Ok(())
    }

    fn check_content_layout(
        &mut self,
        tag: &StartTag<usize>,
        fragment_index: usize,
    ) -> Result<(), ExtractError> {
        let source = self.source;
        let fragment = &mut self.components[fragment_index];
        if !self.named_content.contains(&fragment_index)
            && !crate::html::is_blank(&source[fragment.range.start..tag.span.start])
        {
            return Err(error(source, tag.span.start, MIXED_CONTENT));
        }
        self.named_content.insert(fragment_index);
        if tag.attributes.len() != 1 {
            return Err(error(
                source,
                tag.span.start,
                "a projected template only accepts rust:content; put HTML attributes on its root",
            ));
        }
        Ok(())
    }

    /// A `rust:content` template becomes a content component of its invocation.
    fn open_content(
        &mut self,
        tag: &StartTag<usize>,
        cx: &TagContext,
        content: &Spanned<HtmlString, usize>,
        owner: ContentOwner,
    ) -> Result<usize, ExtractError> {
        self.check_content_layout(tag, owner.fragment)?;
        let source = self.source;
        let first_component = self.first_component;
        let Self {
            components,
            template_roots,
            ..
        } = self;
        let TagContext { lexicals, .. } = cx;
        let field = super::tags::field(
            source,
            &String::from_utf8_lossy(content),
            crate::html::value_start(source, content.span.start),
        )?;
        let (caller_owner, invocation) = owner.invocation;
        let id = ComponentId::new(first_component + components.len());
        let ty = Rust::ident(&format!("__FusorContent{}", id.index()), tag.span.start);
        let capture = components[caller_owner]
            .capture()
            .unwrap_or(&components[caller_owner].ty)
            .clone();
        let content_index = components.len();
        let Binding::Invocation { inputs, .. } = &mut components[caller_owner].bindings[invocation]
        else {
            unreachable!()
        };
        if inputs.iter().any(|input| input.name.same_tokens(&field)) {
            return Err(error(
                source,
                content.span.start,
                "duplicate component input or content name",
            ));
        }
        inputs.push(Input {
            name: field,
            value: InputValue::Content {
                component: content_index,
                origin: ty.clone(),
            },
        });
        template_roots.insert(content_index, 0);
        components.push(lexicals.open(Component::new(
            id,
            ty,
            ComponentShape::Content(capture),
            RenderTarget::Browser,
            tag.span.start..tag.span.end,
        )));
        Ok(content_index)
    }

    fn declare_component(
        &mut self,
        tag: &StartTag<usize>,
        cx: &TagContext,
        ty: &Spanned<HtmlString, usize>,
    ) -> Result<usize, ExtractError> {
        let source = self.source;
        let first_component = self.first_component;
        let Self {
            components,
            component_types,
            template_roots,
            ..
        } = self;
        let TagContext { name, lexicals, .. } = cx;
        let ty = String::from_utf8_lossy(ty).trim().to_owned();
        if ty.is_empty() || !component_types.insert(ty.clone()) {
            return Err(error(
                source,
                tag.span.start,
                "rust:component needs a unique Rust type; use a template for repeated instances",
            ));
        }
        let index = components.len();
        let id = ComponentId::new(first_component + index);
        components.push(lexicals.open(Component::new(
            id,
            Rust::parse(source, &ty, tag.span.start)?,
            ComponentShape::Declared(if name == "template" {
                RootKind::Template
            } else {
                RootKind::Existing
            }),
            RenderTarget::Browser,
            tag.span.start..tag.span.end,
        )));
        if name == "template" {
            template_roots.entry(index).or_insert(0);
        }
        Ok(index)
    }

    /// Validate an element's attributes and open its rust:async/rust:await region.
    fn element_region(
        &self,
        tag: &StartTag<usize>,
        cx: &TagContext,
        element: &ElementOwner,
    ) -> Result<Option<Region>, ExtractError> {
        let source = self.source;
        let name = &cx.name;
        let projected = tag.attributes.get(b"rust:content".as_slice());
        let ElementOwner {
            owner,
            inert,
            component_id,
            rust_script,
        } = *element;
        if tag
            .attributes
            .keys()
            .any(|key| template::reserved_attribute(&String::from_utf8_lossy(key)))
        {
            return Err(error(
                source,
                tag.span.start,
                "data-fusor-* attributes are reserved for the HTML compiler",
            ));
        }
        let has_directive = !rust_script
            && tag
                .attributes
                .keys()
                .any(|key| native_attributes::is_directive(&String::from_utf8_lossy(key)));
        let has_interpolation = owner.is_some()
            && tag
                .attributes
                .values()
                .any(|value| value.windows(2).any(|bytes| bytes == b"{{"));
        let region = self.async_region(tag, name, owner)?;
        if (has_directive || has_interpolation) && (owner.is_none() || inert) {
            return Err(error(
                source,
                tag.span.start,
                "bindings require a rust:component and cannot live in an inert template",
            ));
        }
        if name == "template"
            && component_id.is_some()
            && projected.is_none()
            && (has_interpolation
                || tag.attributes.keys().any(|key| {
                    key.as_ref() != b"rust:component"
                        && key.as_ref() != b"rust:render"
                        && native_attributes::is_directive(&String::from_utf8_lossy(key))
                }))
        {
            return Err(error(
                source,
                tag.span.start,
                "put template bindings on the root element inside the template",
            ));
        }
        Ok(region)
    }

    /// `rust:async` and `rust:await` make an element a coherent region, which
    /// only holds display bindings.
    fn async_region(
        &self,
        tag: &StartTag<usize>,
        name: &str,
        owner: Option<usize>,
    ) -> Result<Option<Region>, ExtractError> {
        let source = self.source;
        let enclosing = self.stack.iter().any(|frame| frame.in_coherent_region());
        let async_attr = tag.attributes.get(b"rust:async".as_slice());
        let await_attr = tag.attributes.get(b"rust:await".as_slice());
        if async_attr.is_some() && (await_attr.is_some() || enclosing) {
            return Err(error(
                source,
                tag.span.start,
                "nested async boundaries and async/await on the same host are unsupported",
            ));
        }
        if (async_attr.is_some() || enclosing)
            && (name.contains('-')
                || tag
                    .attributes
                    .keys()
                    .any(|key| matches!(key.as_ref(), b"is" | b"bind" | b"rust:slot")))
        {
            return Err(error(
                source,
                tag.span.start,
                "rust:async regions support display bindings only; move editable controls, widgets, outlets, opaque content and islands outside",
            ));
        }
        let Some(value) = async_attr.or(await_attr) else {
            return Ok(None);
        };
        let index = owner
            .ok_or_else(|| error(source, tag.span.start, "async regions require a component"))?;
        let offset = crate::html::value_start(source, value.span.start);
        let value = Rust::parse(source, &String::from_utf8_lossy(value), offset)?;
        Ok(Some(Region {
            start: self.components[index].bindings.len(),
            node: ElementId::new(self.node),
            value,
            kind: if await_attr.is_some() {
                RegionKind::Await { alias: None }
            } else {
                RegionKind::Boundary
            },
        }))
    }

    fn lower_element(
        &mut self,
        tag: &StartTag<usize>,
        cx: &TagContext,
        element: &ElementOwner,
    ) -> Result<(), ExtractError> {
        let source = self.source;
        let Self {
            stack,
            components,
            edits,
            foreach_hosts,
            node,
            ..
        } = self;
        let name = &cx.name;
        let projected = tag.attributes.get(b"rust:content".as_slice());
        let ElementOwner {
            owner,
            inert,
            component_id,
            rust_script,
        } = *element;
        if let Some(index) = owner.filter(|_| !inert && !rust_script) {
            if foreign_element(name) {
                return Err(error(
                    source,
                    tag.span.start,
                    "component bindings are not supported inside SVG or MathML",
                ));
            }
            if tag.self_closing && !void_element(name) {
                return Err(error(
                    source,
                    tag.span.start,
                    "non-void component elements need explicit closing tags",
                ));
            }
            if projected.is_some() {
                edits.push(replace_tag(
                    &tag.span,
                    markup::template_open(component_id.expect("content component")),
                ));
            } else if let Some(replacement) = native_attributes::lower(
                source,
                tag,
                &mut components[index],
                native_attributes::ElementContext {
                    node: ElementId::new(*node),
                    component_id,
                    foreach_host: foreach_hosts.contains(&tag.span.start),
                    async_root: stack
                        .last()
                        .is_some_and(|frame| matches!(frame.kind, FrameKind::Async(_))),
                },
            )? {
                edits.push(replace_tag(&tag.span, replacement));
                *node += 1;
            }
        }
        Ok(())
    }

    fn mark_async_roots(
        &mut self,
        tag: &StartTag<usize>,
        name: &str,
        element_id: ElementId,
    ) -> Result<(), ExtractError> {
        let source = self.source;
        let stack = &mut self.stack;
        for frame in stack.iter_mut().rev() {
            let FrameKind::Async(region) = &mut frame.kind else {
                break;
            };
            if void_element(name) {
                return Err(error(
                    source,
                    tag.span.start,
                    "Async and Await need a non-void native root",
                ));
            }
            region.root = match region.root {
                AsyncRoot::Missing => AsyncRoot::One(element_id),
                AsyncRoot::One(_) | AsyncRoot::Multiple => AsyncRoot::Multiple,
            };
        }
        Ok(())
    }

    fn text_host(
        &self,
        tag: &StartTag<usize>,
        name: &str,
        element: ElementOwner,
        element_id: ElementId,
    ) -> Option<TextHost> {
        element
            .owner
            .filter(|index| {
                !element.inert
                    && !element.rust_script
                    && !name.contains('-')
                    // Leading newlines in pre and listing, and noscript's
                    // scripting-dependent parsing, keep them out of direct text.
                    && !text_only_element(name)
                    && !matches!(name, "template" | "pre" | "listing" | "noscript")
                    && !self.foreach_hosts.contains(&tag.span.start)
                    && !tag.attributes.contains_key(b"rust:slot".as_slice())
                    && !tag.attributes.contains_key(b"bind".as_slice())
                    && !tag.attributes.contains_key(b"is".as_slice())
                    && !self.components[*index].elements.last().is_some_and(|element| {
                        element.id == element_id && element.children == ChildPolicy::Managed
                    })
            })
            .map(|index| TextHost {
                opening: tag.span.start..tag.span.end,
                opening_edit: self
                    .edits
                    .last()
                    .filter(|edit| edit.range == (tag.span.start..tag.span.end))
                    .map(|_| self.edits.len() - 1),
                // An unbound frame's node is only the next available
                // ID. Record an actual descriptor before its children
                // can consume that ID.
                element: self.components[index]
                    .elements
                    .last()
                    .filter(|element| element.id == element_id)
                    .map(|element| element.id),
            })
    }

    fn push_element(
        &mut self,
        tag: &StartTag<usize>,
        name: String,
        element: ElementOwner,
        region: ElementRegion,
    ) {
        let ElementRegion {
            id: element_id,
            region,
        } = region;
        let ElementOwner {
            owner,
            inert,
            component_id,
            rust_script: _,
        } = element;
        let text_host = self.text_host(tag, &name, element, element_id);
        let has = |attribute: &[u8]| tag.attributes.contains_key(attribute);
        let bound = match name.as_str() {
            "textarea" if has(b"bind") => Bound::Empty,
            "select" if has(b"bind") => Bound::Options,
            "option" if in_bound_select(&self.stack) && !has(b"value") => Bound::StaticText,
            _ => Bound::Free,
        };
        self.stack.push(Frame {
            name,
            owner,
            node: element_id,
            kind: FrameKind::Element(ElementFrame {
                text_host,
                inert,
                owns_children: tag.attributes.contains_key(b"rust:slot".as_slice()),
                bound,
                component_root: component_id.is_some(),
                region,
            }),
        });
    }

    fn end_tag(&mut self, tag: EndTag<usize>) -> Result<(), ExtractError> {
        let source = self.source;
        let name = String::from_utf8_lossy(&tag.name);
        let Some(index) = self.stack.iter().rposition(|frame| frame.name == name) else {
            return Ok(());
        };
        if self.stack.last().is_some_and(|frame| frame.owner.is_some())
            && index + 1 != self.stack.len()
        {
            return Err(error(
                source,
                tag.span.start,
                "component HTML requires explicitly nested closing tags",
            ));
        }
        self.stack.truncate(index + 1);
        let frame = self.stack.pop().expect("matched closing tag");
        // Spelling and close edits belong to the selected role. Native
        // HTML keeps its existing case-insensitive closing behavior.
        if let Some((spelling, replacement)) = frame.closing() {
            if super::tags::name(source, tag.span.start) != spelling {
                return Err(error(
                    source,
                    tag.span.start,
                    format!("close {spelling} with </{spelling}>"),
                ));
            }
            self.edits.push(replace_tag(&tag.span, replacement));
        }
        self.close(frame, &tag)
    }

    fn close(&mut self, frame: Frame, tag: &EndTag<usize>) -> Result<(), ExtractError> {
        let owner = frame.owner;
        match frame.kind {
            FrameKind::Control(control) => self.close_control(control, owner, tag)?,
            FrameKind::Element(element) => self.close_element(element, frame.name, owner, tag),
            FrameKind::Async(region) => self.close_async(region, owner, tag)?,
            FrameKind::Invocation(_) => {
                let owner = owner.expect("children fragment");
                let fragment = &mut self.components[owner];
                fragment.range.end = if self.named_content.contains(&owner) {
                    fragment.range.start
                } else {
                    tag.span.start
                };
            }
            FrameKind::Route { .. } => {
                self.components[owner.expect("route body")].range.end = tag.span.start
            }
            FrameKind::App | FrameKind::ForEach => {
                self.components[owner.expect("component root")].range.end = tag.span.end
            }
            FrameKind::Router { .. } | FrameKind::Hydrated { .. } | FrameKind::Children => {}
        }
        Ok(())
    }

    fn close_control(
        &mut self,
        control: ControlFrame,
        owner: Option<usize>,
        tag: &EndTag<usize>,
    ) -> Result<(), ExtractError> {
        let source = self.source;
        let first_component = self.first_component;
        let components = &mut self.components;
        match control {
            ControlFrame::If {
                branch,
                phase: IfPhase::Then,
            } => {
                components[owner.expect("an If frame owns its then fragment")]
                    .range
                    .end = tag.span.start;
                // An If without Else renders nothing when its condition is false.
                let case = NewCase {
                    pattern: Rust::synthetic(quote::quote! { false }, tag.span.start),
                    names: Vec::new(),
                    async_locals: Vec::new(),
                    route_locals: Vec::new(),
                };
                push_case(components, first_component, branch, tag.span.start, case);
            }
            ControlFrame::Match { branch } => {
                let Binding::Branch { cases, .. } =
                    &components[branch.owner].bindings[branch.binding]
                else {
                    unreachable!()
                };
                if cases.is_empty() {
                    return Err(error(
                        source,
                        tag.span.start,
                        "Match requires at least one Case",
                    ));
                }
            }
            ControlFrame::Else | ControlFrame::Case { .. } => {
                components[owner.expect("Else and Case frames own their body fragments")]
                    .range
                    .end = tag.span.start
            }
            ControlFrame::If {
                phase: IfPhase::Else,
                ..
            } => {}
        }
        Ok(())
    }

    fn close_element(
        &mut self,
        element: ElementFrame,
        name: String,
        owner: Option<usize>,
        tag: &EndTag<usize>,
    ) {
        let source = self.source;
        let Self {
            components,
            edits,
            text_edits,
            ..
        } = self;
        if let Some(host) = element.text_host {
            if let Some((text_id, text_edit)) =
                text_edits.remove(&(host.opening.end, tag.span.start))
            {
                let component = &mut components[owner.expect("native text host")];
                let marker = format!(" {}=\"{text_id}\"", template::TEXT_ELEMENT_ATTRIBUTE);
                let opening = if let Some(index) = host.opening_edit {
                    &mut edits[index].replacement
                } else {
                    edits.push(Edit {
                        replacement: source[host.opening.clone()].to_owned(),
                        range: host.opening,
                    });
                    &mut edits
                        .last_mut()
                        .expect("the opening tag edit was just appended")
                        .replacement
                };
                opening.insert_str(opening.len() - 1, &marker);
                edits[text_edit].replacement.clear();
                // Exact authored contents exclude whitespace,
                // comments and siblings, so this is the last text
                // slot collected for this component.
                let last = component.texts.pop();
                debug_assert_eq!(last, Some(text_id));
                component.text_elements.push(TextElement {
                    id: text_id,
                    host: host.element,
                    tag: name,
                });
            }
        }

        if element.component_root {
            components[owner.expect("component root")].range.end = tag.span.end;
        }
        if let Some(region) = element.region {
            let component = &mut components[owner.expect("region component")];
            let bindings = component.bindings.split_off(region.start);
            component.bindings.push(Binding::Region {
                node: region.node,
                value: region.value,
                kind: region.kind,
                bindings,
            });
        }
    }

    fn close_async(
        &mut self,
        region: AsyncFrame,
        owner: Option<usize>,
        tag: &EndTag<usize>,
    ) -> Result<(), ExtractError> {
        let source = self.source;
        let components = &mut self.components;
        let AsyncRoot::One(node) = region.root else {
            return Err(error(
                source,
                tag.span.start,
                "Async and Await require exactly one native HTML root",
            ));
        };
        let component = &mut components[owner.expect("async owner")];
        let bindings = component.bindings.split_off(region.start);
        component.bindings.push(Binding::Region {
            node,
            value: region.value,
            kind: region.kind,
            bindings,
        });
        Ok(())
    }

    fn text(&mut self, text: Spanned<HtmlString, usize>) -> Result<(), ExtractError> {
        let source = self.source;
        let Some(frame) = self.stack.last() else {
            return Ok(());
        };
        let Some(index) = frame.owner else {
            return Ok(());
        };
        let offset = text.span.start;
        let blank = String::from_utf8_lossy(&text).trim().is_empty();
        if !blank {
            if let Some(message) = frame.text_rejection(self.named_content.contains(&index)) {
                return Err(error(source, offset, message));
            }
        }
        if matches!(frame.name.as_str(), "script" | "style") {
            return Ok(());
        }
        let raw = &source[text.span.start..text.span.end];
        if !blank && (matches!(frame.kind, FrameKind::Async(_)) || frame.requires_native_root()) {
            return Err(error(
                source,
                offset,
                "a component template must put its text inside a native root element",
            ));
        }
        if !raw.contains("{{") {
            return Ok(());
        }
        if frame.inert() || text_only_element(&frame.name) {
            return Err(error(
                source,
                offset,
                "text interpolation requires rendered HTML text; raw-text elements cannot contain bindings",
            ));
        }
        if frame.bound() == Bound::StaticText {
            return Err(error(
                source,
                offset,
                "an option in a bound select needs a value attribute when its text is interpolated",
            ));
        }
        self.text_bindings(index, raw, offset)
    }

    /// Each `{{ }}` in rendered text becomes a Text binding between two markers.
    fn text_bindings(&mut self, index: usize, raw: &str, start: usize) -> Result<(), ExtractError> {
        let source = self.source;
        let Self {
            components,
            edits,
            text_edits,
            slot,
            ..
        } = self;
        for part in interpolations(source, raw, start, true)? {
            let offset = start + part.range.start;
            let text_id = TextId::new(*slot);
            text_edits.insert((offset, start + part.range.end), (text_id, edits.len()));
            edits.push(Edit {
                range: offset..start + part.range.end,
                replacement: markup::text_slot(text_id),
            });
            components[index].bindings.push(Binding::Text {
                slot: text_id,
                value: Rust::authored(part.tokens, offset),
            });
            components[index].texts.push(text_id);
            *slot += 1;
        }
        Ok(())
    }

    fn finish(self) -> Result<Plan, ExtractError> {
        let Parser {
            source,
            blocks,
            stack,
            mut components,
            mut edits,
            template_roots,
            pending_module,
            ..
        } = self;
        if pending_module.is_some() {
            return Err(error(
                source,
                source.len(),
                "component module is missing its closing </script> tag",
            ));
        }
        if stack.iter().any(|frame| frame.owner.is_some()) {
            return Err(error(
                source,
                source.len(),
                "component HTML is missing a closing tag",
            ));
        }
        validate::roots(source, &mut components, &template_roots)?;
        validate::components(source, &components)?;
        let content_ranges: Vec<_> = components
            .iter()
            .filter(|component| component.capture().is_some() || component.inline())
            .map(|component| component.range.clone())
            .filter(|range| !range.is_empty())
            .collect();
        materialize::components(source, blocks, &edits, &content_ranges, &mut components);
        // Row analysis consumes authored bindings. Scope rewriting inserts lexical
        // aliases, so it must run only after the item-only proof has been recorded.
        super::foreach::mark_item_only_rows(&mut components);
        for component in &mut components {
            super::lexical::rewrite(component);
        }
        let templates = components
            .iter()
            .filter(|component| {
                (component.capture().is_some() || component.inline()) && !component.empty
            })
            .map(|component| component.html.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        materialize::remove_captured_edits(&mut edits, &content_ranges);
        Ok(Plan {
            edits,
            components,
            templates,
        })
    }
}
