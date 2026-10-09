//! Private compiler contract. Rust token trees retain their native spans.

use super::tokens::Rust;
use fusor::template::{ChildPolicy, ComponentId, ElementId, MountId, RootKind, TextId};
pub(super) use fusor_islands::{Activation, Prefetch};
use proc_macro2::Span;
use std::ops::Range;

pub(crate) struct Edit {
    pub range: Range<usize>,
    pub replacement: String,
}

pub(super) struct Plan {
    pub edits: Vec<Edit>,
    pub components: Vec<Component>,
    pub templates: String,
}

pub(super) struct Component {
    pub id: ComponentId,
    pub ty: Rust,
    pub shape: ComponentShape,
    pub empty: bool,
    pub row_locals: Vec<(Rust, Rust)>,
    // Proven before lexical aliases are inserted; only direct forwarding rows.
    pub item_only_row: bool,
    pub async_locals: Vec<Rust>,
    pub route_locals: Vec<Rust>,
    pub snapshot_locals: Vec<Rust>,
    pub elements: Vec<Element>,
    pub texts: Vec<TextId>,
    pub text_elements: Vec<TextElement>,
    pub bindings: Vec<Binding>,
    pub render: RenderTarget,
    pub range: Range<usize>,
    pub html: String,
    // Authored byte for each materialized byte, retained for backend diagnostics.
    pub html_origins: Vec<usize>,
    pub javascript: Option<crate::JavaScriptModule>,
}

/// What kind of component this is, which decides how it mounts and what it captures.
pub(super) enum ComponentShape {
    /// A `rust:component` declaration, mounted from a template or an existing root.
    Declared(RootKind),
    /// The `App` boundary, built by this state expression.
    App(Rust),
    /// A ForEach row, rendered inline by its list.
    Row,
    /// Children of a component tag, a Branch case or a Route body; captures its caller's state.
    Fragment(Rust),
    /// A `rust:content` template passed to a component; captures its caller's state.
    Content(Rust),
}

impl Component {
    pub fn new(
        id: ComponentId,
        ty: Rust,
        shape: ComponentShape,
        render: RenderTarget,
        range: Range<usize>,
    ) -> Self {
        Self {
            id,
            ty,
            shape,
            empty: false,
            row_locals: Vec::new(),
            item_only_row: false,
            async_locals: Vec::new(),
            route_locals: Vec::new(),
            snapshot_locals: Vec::new(),
            elements: Vec::new(),
            texts: Vec::new(),
            text_elements: Vec::new(),
            bindings: Vec::new(),
            render,
            range,
            html: String::new(),
            html_origins: Vec::new(),
            javascript: None,
        }
    }

    /// The state a fragment or content component borrows from its caller.
    pub fn capture(&self) -> Option<&Rust> {
        match &self.shape {
            ComponentShape::Fragment(capture) | ComponentShape::Content(capture) => Some(capture),
            ComponentShape::Declared(_) | ComponentShape::App(_) | ComponentShape::Row => None,
        }
    }

    pub fn inline(&self) -> bool {
        matches!(self.shape, ComponentShape::Row)
    }

    pub fn fragment(&self) -> bool {
        matches!(
            self.shape,
            ComponentShape::Fragment(_) | ComponentShape::Declared(RootKind::Fragment)
        )
    }

    pub fn kind(&self) -> RootKind {
        match self.shape {
            ComponentShape::Declared(kind) => kind,
            ComponentShape::App(_) => RootKind::Existing,
            ComponentShape::Row | ComponentShape::Fragment(_) | ComponentShape::Content(_) => {
                RootKind::Template
            }
        }
    }

    pub fn app(&self) -> Option<&Rust> {
        match &self.shape {
            ComponentShape::App(state) => Some(state),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RenderTarget {
    Browser,
    Server,
    Shared,
}

pub(super) struct Element {
    pub id: ElementId,
    pub tag: String,
    pub children: ChildPolicy,
}

pub(super) struct TextElement {
    pub id: TextId,
    pub host: Option<ElementId>,
    pub tag: String,
}

pub(super) enum StringPart {
    Literal(String),
    Expression(Rust),
}

pub(super) struct InterpolatedString(pub Vec<StringPart>);

impl InterpolatedString {
    pub fn literal(text: &str) -> Self {
        Self(vec![StringPart::Literal(text.to_owned())])
    }

    /// The text, when no part is an expression.
    pub fn as_literal(&self) -> Option<String> {
        self.0
            .iter()
            .map(|part| match part {
                StringPart::Literal(text) => Some(text.as_str()),
                StringPart::Expression(_) => None,
            })
            .collect()
    }

    /// The expression, when the value is exactly one interpolation.
    pub fn as_expression(&self) -> Option<&Rust> {
        match self.0.as_slice() {
            [StringPart::Expression(expression)] => Some(expression),
            [
                StringPart::Literal(before),
                StringPart::Expression(expression),
                StringPart::Literal(after),
            ] if before.is_empty() && after.is_empty() => Some(expression),
            _ => None,
        }
    }

    pub fn expressions(&self) -> impl Iterator<Item = &Rust> {
        self.0.iter().filter_map(|part| match part {
            StringPart::Expression(expression) => Some(expression),
            StringPart::Literal(_) => None,
        })
    }

    pub fn expressions_mut(&mut self) -> impl Iterator<Item = &mut Rust> {
        self.0.iter_mut().filter_map(|part| match part {
            StringPart::Expression(expression) => Some(expression),
            StringPart::Literal(_) => None,
        })
    }
}

/// What a `bind` attribute edits, chosen from the element's markup.
pub(super) enum Control {
    /// A text-like or numeric input, or a textarea: its `value` property.
    Text,
    Select,
    SelectMultiple,
    /// A checkbox and its `value`, which a list of values collects.
    Checkbox(InterpolatedString),
    /// A radio button and the `value` it chooses.
    Radio(InterpolatedString),
}

impl Control {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Select => "select",
            Self::SelectMultiple => "select_multiple",
            Self::Checkbox(_) => "checkbox",
            Self::Radio(_) => "radio",
        }
    }

    /// The `value` a checkbox or radio compares with the bound value.
    pub fn choice(&self) -> Option<&InterpolatedString> {
        match self {
            Self::Checkbox(choice) | Self::Radio(choice) => Some(choice),
            _ => None,
        }
    }

    pub fn choice_mut(&mut self) -> Option<&mut InterpolatedString> {
        match self {
            Self::Checkbox(choice) | Self::Radio(choice) => Some(choice),
            _ => None,
        }
    }
}

pub(super) struct Input {
    pub name: Rust,
    pub value: InputValue,
}

impl Input {
    fn value_fragments(&self) -> impl Iterator<Item = &Rust> {
        std::iter::once(&self.name).chain(self.value.value())
    }

    fn fragments(&self) -> [&Rust; 2] {
        let (InputValue::Expression(value)
        | InputValue::Literal(value)
        | InputValue::Content { origin: value, .. }) = &self.value;
        [&self.name, value]
    }
}

pub(super) enum InputValue {
    /// `name="{{ expression }}"`.
    Expression(Rust),
    /// `name="text"`: a string literal token.
    Literal(Rust),
    /// A `rust:content` template passed as this input.
    Content { component: usize, origin: Rust },
}

impl InputValue {
    /// The Rust this input evaluates, unless it is projected content.
    pub fn value(&self) -> Option<&Rust> {
        match self {
            Self::Expression(value) | Self::Literal(value) => Some(value),
            Self::Content { .. } => None,
        }
    }
}

/// What a coherent region does with its value.
pub(super) enum RegionKind {
    /// `Async` or `rust:async`: the value is the region's async boundary.
    Boundary,
    /// `Await` or `rust:await`: the value is read; `Await` names the result.
    Await { alias: Option<Rust> },
}

impl RegionKind {
    pub fn alias(&self) -> Option<&Rust> {
        match self {
            Self::Boundary => None,
            Self::Await { alias } => alias.as_ref(),
        }
    }
}

pub(super) struct RouteBranch {
    pub path: Option<String>,
    pub params: Option<Rust>,
    pub names: Vec<Rust>,
    pub body: usize,
}

pub(super) struct CaseBranch {
    pub pattern: Rust,
    pub names: Vec<Rust>,
    pub body: usize,
}

pub(super) struct ChildFragment {
    pub name: Option<String>,
    pub body: usize,
}

pub(super) enum Binding {
    Branch {
        point: MountId,
        value: Rust,
        cases: Vec<CaseBranch>,
        snapshots: Vec<(Rust, Rust)>,
    },
    Router {
        point: MountId,
        origin: Rust,
        routes: Vec<RouteBranch>,
    },
    ForEach {
        node: ElementId,
        items: Rust,
        key: Rust,
        body: usize,
    },
    Children {
        name: Option<String>,
        point: MountId,
        origin: Rust,
    },
    Invocation {
        point: MountId,
        ty: Rust,
        inputs: Vec<Input>,
        children: Vec<ChildFragment>,
        condition: Option<Rust>,
        key: Option<Rust>,
    },
    Island {
        node: ElementId,
        descriptor: Rust,
        /// The component's inputs, which become its serialized props.
        inputs: Vec<Input>,
        activation: Activation,
        prefetch: Prefetch,
    },
    Region {
        node: ElementId,
        value: Rust,
        kind: RegionKind,
        bindings: Vec<Binding>,
    },
    Text {
        slot: TextId,
        value: Rust,
    },
    Attribute {
        node: ElementId,
        name: String,
        value: InterpolatedString,
    },
    Property {
        node: ElementId,
        name: String,
        value: Rust,
    },
    Boolean {
        node: ElementId,
        name: String,
        value: Rust,
    },
    Value {
        node: ElementId,
        value: InterpolatedString,
    },
    Checked {
        node: ElementId,
        value: Rust,
    },
    Class {
        node: ElementId,
        name: String,
        value: Rust,
    },
    Event {
        node: ElementId,
        name: String,
        handler: Rust,
    },
    Bind {
        node: ElementId,
        control: Control,
        value: Rust,
    },
    Slot {
        node: ElementId,
        content: Rust,
        condition: Option<Rust>,
        key: Option<Rust>,
    },
}

/// Where a binding lives in its template.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Anchor {
    Element(ElementId),
    Mount(MountId),
    Text(TextId),
}

impl Binding {
    pub fn anchor(&self) -> Anchor {
        match self {
            Self::Text { slot, .. } => Anchor::Text(*slot),
            Self::Branch { point, .. }
            | Self::Router { point, .. }
            | Self::Children { point, .. }
            | Self::Invocation { point, .. } => Anchor::Mount(*point),
            Self::ForEach { node, .. }
            | Self::Island { node, .. }
            | Self::Region { node, .. }
            | Self::Attribute { node, .. }
            | Self::Property { node, .. }
            | Self::Boolean { node, .. }
            | Self::Value { node, .. }
            | Self::Checked { node, .. }
            | Self::Class { node, .. }
            | Self::Event { node, .. }
            | Self::Bind { node, .. }
            | Self::Slot { node, .. } => Anchor::Element(*node),
        }
    }

    /// The fragment that locates this binding in the HTML; code generated for the
    /// binding is spanned to it. It is always the first of `fragments()`.
    pub fn origin(&self) -> &Rust {
        match self {
            Self::Branch { value, .. }
            | Self::Region { value, .. }
            | Self::Text { value, .. }
            | Self::Property { value, .. }
            | Self::Boolean { value, .. }
            | Self::Checked { value, .. }
            | Self::Class { value, .. }
            | Self::Bind { value, .. } => value,
            Self::Children { origin, .. } | Self::Router { origin, .. } => origin,
            Self::ForEach { items, .. } => items,
            Self::Invocation { ty, .. } => ty,
            Self::Island { descriptor, .. } => descriptor,
            Self::Event { handler, .. } => handler,
            Self::Slot { content, .. } => content,
            Self::Attribute { value, .. } | Self::Value { value, .. } => value
                .expressions()
                .next()
                .expect("an interpolated attribute has an expression"),
        }
    }

    /// These bindings, each followed by the bindings inside it when it is a region.
    pub fn walk(bindings: &[Binding]) -> Vec<&Binding> {
        let mut all = Vec::new();
        for binding in bindings {
            all.push(binding);
            if let Self::Region { bindings, .. } = binding {
                all.extend(Self::walk(bindings));
            }
        }
        all
    }

    /// Call `visit` on each binding, then on the bindings inside it when it is a region.
    pub fn visit_mut(bindings: &mut [Binding], visit: &mut impl FnMut(&mut Binding)) {
        for binding in bindings {
            visit(binding);
            if let Self::Region { bindings, .. } = binding {
                Self::visit_mut(bindings, visit);
            }
        }
    }

    /// The components this binding renders: a tag's children and named content,
    /// each case or route body, and a list's row.
    pub fn components(&self) -> Vec<usize> {
        match self {
            Self::Invocation {
                children, inputs, ..
            } => children
                .iter()
                .map(|child| child.body)
                .chain(inputs.iter().filter_map(|input| match input.value {
                    InputValue::Content { component, .. } => Some(component),
                    _ => None,
                }))
                .collect(),
            Self::Branch { cases, .. } => cases.iter().map(|case| case.body).collect(),
            Self::Router { routes, .. } => routes.iter().map(|route| route.body).collect(),
            Self::ForEach { body, .. } => vec![*body],
            _ => Vec::new(),
        }
    }

    pub fn span(&self) -> Span {
        self.origin().span()
    }

    pub fn fragments(&self) -> Vec<&Rust> {
        match self {
            Self::Branch { value, cases, .. } => std::iter::once(value)
                .chain(cases.iter().map(|case| &case.pattern))
                .collect(),
            Self::Children { origin, .. } | Self::Router { origin, .. } => vec![origin],
            Self::ForEach { items, key, .. } => vec![items, key],
            Self::Invocation {
                ty,
                inputs,
                condition,
                key,
                ..
            } => std::iter::once(ty)
                .chain(inputs.iter().flat_map(Input::fragments))
                .chain(condition)
                .chain(key)
                .collect(),
            Self::Island {
                descriptor, inputs, ..
            } => std::iter::once(descriptor)
                .chain(inputs.iter().flat_map(Input::value_fragments))
                .collect(),
            Self::Region {
                value,
                kind,
                bindings,
                ..
            } => std::iter::once(value)
                .chain(kind.alias())
                .chain(bindings.iter().flat_map(Self::fragments))
                .collect(),
            Self::Text { value, .. }
            | Self::Property { value, .. }
            | Self::Boolean { value, .. }
            | Self::Checked { value, .. }
            | Self::Class { value, .. } => vec![value],
            Self::Bind { value, control, .. } => std::iter::once(value)
                .chain(
                    control
                        .choice()
                        .into_iter()
                        .flat_map(InterpolatedString::expressions),
                )
                .collect(),
            Self::Event { handler, .. } => vec![handler],
            Self::Slot {
                content: constructor,
                condition,
                key,
                ..
            } => std::iter::once(constructor)
                .chain(condition)
                .chain(key)
                .collect(),
            Self::Attribute { value, .. } | Self::Value { value, .. } => {
                value.expressions().collect()
            }
        }
    }
}
