//! Adapt the supported public compiler callbacks to the internal emission contract.
use super::super::{ir, parse, tags};
use super::*;
use crate::{
    ExtractError,
    backend::{
        Attribute, Backend, Capability, GeneratedSource, Node, NodeKind, Operation, Origin,
        Template, VERSION,
    },
    error, html,
};
use fusor::template::{self, MountMarker, TextMarker};
use html5gum::{StartTag, Token};
use std::collections::BTreeMap;

struct ExternalBackend<'a> {
    backend: &'a dyn Backend,
    templates: &'a [Template],
    source: &'a str,
}

impl CompilerBackend for ExternalBackend<'_> {
    fn runtime(&self) -> Runtime {
        self.backend.runtime()
    }

    fn component(&self, component: &Component, ctx: Ctx) -> TokenStream {
        let local_clones = clone_locals(&component.async_locals);
        let incoming = incoming_children(component, ctx);
        let expose_capture = component.capture().map(|_| {
            quote! { #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = ::std::rc::Rc::clone(state.as_ref()); }
        });
        let mount = self.backend.mount(&self.templates[component.id.index()]);
        let bindings = self.install(component, ctx);
        let body = quote! {
            #local_clones
            #incoming
            let mut __fusor_scope = (#mount)?;
            #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = ::fusor::render::construct(&mut __fusor_scope, make)?;
            #expose_capture
            #bindings
            ::std::result::Result::Ok(__fusor_scope)
        };
        if component.capture().is_some() || component.inline() {
            captured(&body, ctx)
        } else {
            let ty = &component.ty;
            self.backend.component(crate::backend::ComponentCode {
                ty: quote! { #ty },
                body,
                app_state: component.app().map(|state| quote! { #state }),
            })
        }
    }

    fn binding(&self, binding: &Binding, ctx: Ctx<'_>, locals: &[Rust]) -> TokenStream {
        let span = binding.span();
        if matches!(binding, Binding::Region { .. }) {
            return self.region(binding, ctx, locals);
        }
        let captures = captures(span, ctx, locals);
        let read = |value: &Rust| quote_spanned! {span=> move || { #value } };
        let kind = match binding {
            Binding::Text { value, .. } => OperationKind::Text {
                read: quote_spanned! {span=> move || ::std::string::ToString::to_string(&(#value)) },
            },
            Binding::Attribute { name, value, .. } => {
                let value = emit::string(value);
                OperationKind::Attribute {
                    name: name.clone(),
                    read: quote_spanned! {span=> move || #value },
                }
            }
            Binding::Property { name, value, .. } => OperationKind::Property {
                name: name.clone(),
                read: read(value),
            },
            Binding::Boolean { name, value, .. } => OperationKind::Boolean {
                name: name.clone(),
                read: read(value),
            },
            Binding::Value { value, .. } => {
                let value = emit::string(value);
                OperationKind::Value {
                    read: quote_spanned! {span=> move || #value },
                }
            }
            Binding::Checked { value, .. } => OperationKind::Checked { read: read(value) },
            Binding::Class { name, value, .. } => OperationKind::Class {
                name: name.clone(),
                read: read(value),
            },
            Binding::Event { name, handler, .. } => OperationKind::Event {
                name: name.clone(),
                handler: quote_spanned! {span=> move |#[allow(unused_variables, reason = "handlers may ignore the event")] event| { #handler } },
            },
            Binding::Children { name, .. } => {
                let children = selected_children(name);
                OperationKind::Children {
                    children: quote! { (#children).clone() },
                }
            }
            Binding::Bind { control, value, .. } => {
                return self.bound_control(binding, (control, value), ctx, locals);
            }
            _ => unreachable!("validated structural bindings use common factories"),
        };
        let operation = self.operation(binding, kind, ctx.mode);
        quote_spanned! {span=> { #captures #operation }}
    }

    fn operation(
        &self,
        binding: &Binding,
        kind: OperationKind,
        mode: OperationMode,
    ) -> TokenStream {
        self.backend.operation(Operation {
            anchor: binding_anchor(binding),
            origin: origin(self.source, binding.origin().offset),
            kind,
            mode,
        })
    }

    fn content(&self, _factory: TokenStream) -> TokenStream {
        unreachable!("projected content is rejected by external backend validation")
    }
}

impl ExternalBackend<'_> {
    fn region(&self, binding: &Binding, ctx: Ctx, locals: &[Rust]) -> TokenStream {
        let Binding::Region {
            kind,
            value,
            bindings,
            ..
        } = binding
        else {
            unreachable!("async region binding")
        };
        let span = binding.span();
        if ctx.mode == OperationMode::Coherent {
            return await_binding(binding, ctx, locals, |item, ctx, locals| {
                self.coherent_binding(item, ctx, locals)
            });
        }
        let captures = captures(span, ctx, locals);
        let coherent = Ctx {
            mode: OperationMode::Coherent,
            ..ctx
        };
        let (boundary, body) = match kind {
            RegionKind::Boundary => (
                quote! { (#value).clone() },
                bindings
                    .iter()
                    .map(|item| self.coherent_binding(item, coherent, locals))
                    .collect(),
            ),
            RegionKind::Await { alias: Some(_) } => (
                quote! { ::fusor::coherence::AsyncBoundary::coherent() },
                self.coherent_binding(binding, coherent, locals),
            ),
            RegionKind::Await { alias: None } => unreachable!("validated await ancestor"),
        };
        let render = coherent_renderer(body, ctx);
        let operation =
            self.operation(binding, OperationKind::Async { boundary, render }, ctx.mode);
        quote_spanned! {span=> { #captures #operation }}
    }

    fn bound_control(
        &self,
        binding: &Binding,
        control_value: (&Control, &Rust),
        ctx: Ctx,
        locals: &[Rust],
    ) -> TokenStream {
        let (control, value) = control_value;
        let span = binding.span();
        let captures = captures(span, ctx, locals);
        let choice = control.choice().map(|value| {
            let value = emit::string(value);
            quote_spanned! {span=> move || #value }
        });
        let operation = self.operation(
            binding,
            OperationKind::Bind {
                control: control_kind(control),
                value: quote! { __fusor_bound },
                choice,
            },
            ctx.mode,
        );
        quote_spanned! {span=> {
            #captures
            {
                let __fusor_bound = ::std::clone::Clone::clone(&(#value));
                #operation
            }
        } }
    }

    fn coherent_binding(&self, item: &Binding, ctx: Ctx, locals: &[Rust]) -> TokenStream {
        if let Some(message) = coherent_rejection(item) {
            return quote_spanned! {item.span()=> __fusor_frame.reject(#message)?; };
        }
        binding(item, ctx, locals)
    }

    fn install(&self, component: &Component, ctx: Ctx) -> TokenStream {
        let ordinary = Ctx {
            mode: OperationMode::Reactive,
            ..ctx
        };
        let locals = &component.async_locals;
        if ctx.runtime.coherent_frame.is_none() {
            return order_bindings(&component.bindings)
                .into_iter()
                .map(|item| binding(item, ordinary, locals))
                .collect();
        }
        let mut code = BindingCode::default();
        for item in order_bindings(&component.bindings) {
            let next = self.install_binding(item, ctx, locals);
            code.shared.extend(next.shared);
            code.ordinary.extend(next.ordinary);
            code.coherent.extend(next.coherent);
        }
        let BindingCode {
            shared,
            ordinary: reactive,
            coherent: staged,
        } = code;
        let renderer = coherent_renderer(staged, ctx);
        let ordinary = (!reactive.is_empty()).then(|| quote! { else { #reactive } });
        quote! {
            #shared
            if __fusor_scope.is_coherent() {
                __fusor_scope.set_coherent_renderer(#renderer);
            } #ordinary
        }
    }
    fn install_binding(&self, item: &Binding, ctx: Ctx, locals: &[Rust]) -> BindingCode {
        let ordinary = Ctx {
            mode: OperationMode::Reactive,
            ..ctx
        };
        let coherent = Ctx {
            mode: OperationMode::Coherent,
            ..ctx
        };
        let mut code = BindingCode::default();
        let supplied = supplied_children(item, ctx);
        let structural = match item {
            Binding::Branch { .. } => Some(branch(item, ctx, locals)),
            Binding::ForEach { .. } => Some(list(item, ctx, locals)),
            _ => None,
        };
        if let Some((setup, operation)) = structural {
            code.shared.extend(setup);
            code.ordinary
                .extend(self.operation(item, operation.clone(), OperationMode::Reactive));
            let borrowed = borrowed_structure(operation);
            code.coherent
                .extend(self.operation(item, borrowed, OperationMode::Coherent));
        } else if let Some((point, child)) = supplied {
            let (make, setup) = shared_children_factory(point, child, ctx);
            code.shared.extend(setup);
            code.ordinary
                .extend(invocation(item, quote! { #make() }, ordinary, locals));
            code.coherent
                .extend(invocation(item, quote! { #make() }, coherent, locals));
        } else if let Binding::Region {
            node,
            kind: RegionKind::Await { alias: Some(_) },
            ..
        } = item
        {
            let name = indexed("await_render", node.index());
            let captures = captures(item.span(), ctx, locals);
            let renderer = coherent_renderer(self.coherent_binding(item, coherent, locals), ctx);
            code.shared
                .extend(quote! { let #name = { #captures #renderer }; });
            code.ordinary.extend(self.operation(
                item,
                OperationKind::Async {
                    boundary: quote! { ::fusor::coherence::AsyncBoundary::coherent() },
                    render: quote! { #name },
                },
                OperationMode::Reactive,
            ));
            code.coherent
                .extend(quote_spanned! {item.span()=> #name(__fusor_frame)?; });
        } else {
            code.ordinary.extend(binding(item, ordinary, locals));
            code.coherent
                .extend(self.coherent_binding(item, coherent, locals));
        }
        code
    }
}

pub(crate) fn generate(
    source: &str,
    backend: &dyn Backend,
) -> Result<GeneratedSource, ExtractError> {
    if backend.version() != VERSION {
        return Err(error(
            source,
            0,
            format!(
                "backend {} requires compiler contract {}, supported version is {VERSION}",
                backend.name(),
                backend.version()
            ),
        ));
    }
    let authored = authored_tags(source)?;
    let plan = parse::parse(source, &[], 0)?;
    validate_source_coverage(source, &plan.components)?;
    let templates = plan
        .components
        .iter()
        .map(|component| {
            let tree = structure(source, component, &authored);
            validate(source, component, &tree, backend)?;
            backend.validate(&tree)?;
            Ok(tree)
        })
        .collect::<Result<Vec<_>, ExtractError>>()?;
    let mut rust = String::new();
    let locations = super::generate_using(
        source,
        &plan.components,
        &mut rust,
        &ExternalBackend {
            backend,
            templates: &templates,
            source,
        },
        false,
    );
    rust.push_str(&backend.file().to_string());
    Ok(GeneratedSource {
        rust,
        source_map: crate::SourceMap::new(locations).expect("ordered compiler locations"),
    })
}

fn authored_tags(source: &str) -> Result<BTreeMap<usize, StartTag<usize>>, ExtractError> {
    // Source linkage is the caller's ordinary Rust module, never web script discovery.
    let mut authored = BTreeMap::new();
    for token in html::tokens(source) {
        if let Token::StartTag(tag) = token {
            for (name, value) in &tag.attributes {
                if name.as_ref() == b"hydrate" || name.starts_with(b"hydrate:") {
                    return Err(error(
                        source,
                        value.span.start,
                        "hydration is unsupported by external backends",
                    ));
                }
            }
            if tag.name.as_ref() == b"script" {
                return Err(error(
                    source,
                    tag.span.start,
                    "external backend templates use ordinary Rust modules; script integration is unsupported",
                ));
            }
            authored.insert(tag.span.start, tag);
        }
    }
    Ok(authored)
}

// The facade delivers component trees, not a browser document shell. Refuse
// outside markup so head styles, links and static siblings cannot disappear.
fn validate_source_coverage(source: &str, components: &[Component]) -> Result<(), ExtractError> {
    if components.is_empty() {
        return Err(error(
            source,
            0,
            "backend templates require a rust:component declaration or App",
        ));
    }
    for token in html::tokens(source) {
        let offset = match token {
            Token::StartTag(tag) => tag.span.start,
            Token::EndTag(tag) => tag.span.start,
            Token::String(text) if !text.iter().all(u8::is_ascii_whitespace) => {
                text.span.start
                    + source[text.span.start..text.span.end]
                        .bytes()
                        .take_while(u8::is_ascii_whitespace)
                        .count()
            }
            Token::Doctype(doctype) => doctype.span.start,
            _ => continue,
        };
        if !components
            .iter()
            .any(|component| component.range.contains(&offset))
        {
            return Err(error(
                source,
                offset,
                "external compilation requires markup inside rust:component or App; document shells and external assets belong to the build helper",
            ));
        }
    }
    Ok(())
}

fn origin(source: &str, offset: usize) -> Origin {
    let (line, column) = crate::location(source, offset);
    Origin {
        offset,
        line,
        column,
    }
}

fn validate(
    source: &str,
    component: &Component,
    tree: &Template,
    backend: &dyn Backend,
) -> Result<(), ExtractError> {
    if component.app().is_some() && !backend.supports(Capability::App) {
        return Err(error(
            source,
            component.range.start,
            format!("backend {} does not support App", backend.name()),
        ));
    }
    if component.render != RenderTarget::Browser {
        return Err(error(
            source,
            component.range.start,
            "rust:render is a browser/server delivery contract; unsupported by external backends",
        ));
    }
    if component.javascript.is_some() {
        return Err(error(
            source,
            component.range.start,
            "JavaScript component modules are unsupported by external backends",
        ));
    }
    for binding in Binding::walk(&component.bindings) {
        let capability = capability(source, binding)?;
        if matches!(capability, Capability::Async) && backend.runtime().coherent_frame.is_none() {
            return Err(error(
                source,
                binding.origin().offset,
                format!(
                    "backend {} does not provide coherent Async/Await frames",
                    backend.name()
                ),
            ));
        }
        if !backend.supports(capability) {
            return Err(error(
                source,
                binding.origin().offset,
                format!("backend {} does not support {capability:?}", backend.name()),
            ));
        }
        backend.validate_binding(
            tree,
            capability,
            binding_anchor(binding),
            &origin(source, binding.origin().offset),
        )?;
    }
    validate_regions(source, &component.bindings, false)?;
    Ok(())
}

fn capability<'a>(source: &str, binding: &'a Binding) -> Result<Capability<'a>, ExtractError> {
    Ok(match binding {
        Binding::Text { .. } => Capability::Text,
        Binding::Attribute { name, .. } => Capability::Attribute(name),
        Binding::Property { name, .. } => Capability::Property(name),
        Binding::Boolean { name, .. } => Capability::Boolean(name),
        Binding::Value { .. } => Capability::Value,
        Binding::Checked { .. } => Capability::Checked,
        Binding::Class { name, .. } => Capability::Class(name),
        Binding::Event { name, .. } => Capability::Event(name),
        Binding::Bind { control, .. } => Capability::Bind(control_kind(control)),
        Binding::Branch { .. } => Capability::Branch,
        Binding::ForEach { .. } => Capability::Keyed,
        Binding::Invocation { inputs, .. } if !has_content(inputs) => Capability::Component,
        Binding::Children { .. } => Capability::Children,
        Binding::Router { .. } => Capability::Router,
        Binding::Region { .. } => Capability::Async,
        _ => {
            return Err(error(
                source,
                binding.origin().offset,
                "external backends do not support hydration or opaque/projected content",
            ));
        }
    })
}

fn validate_regions(
    source: &str,
    bindings: &[Binding],
    coherent: bool,
) -> Result<(), ExtractError> {
    for binding in bindings {
        if coherent {
            if let Some(message) = coherent_rejection(binding) {
                return Err(error(source, binding.origin().offset, message));
            }
        }
        if let Binding::Region { kind, bindings, .. } = binding {
            if !coherent && matches!(kind, RegionKind::Await { alias: None }) {
                return Err(error(
                    source,
                    binding.origin().offset,
                    "rust:await requires a coherent boundary ancestor; use Await for independent loading",
                ));
            }
            validate_regions(source, bindings, true)?;
        }
    }
    Ok(())
}

fn control_kind(control: &ir::Control) -> crate::backend::Control {
    match control {
        ir::Control::Text => crate::backend::Control::Text,
        ir::Control::Select => crate::backend::Control::Select,
        ir::Control::SelectMultiple => crate::backend::Control::SelectMultiple,
        ir::Control::Checkbox(_) => crate::backend::Control::Checkbox,
        ir::Control::Radio(_) => crate::backend::Control::Radio,
    }
}

fn structure(
    source: &str,
    component: &Component,
    authored: &BTreeMap<usize, StartTag<usize>>,
) -> Template {
    let mut tree = Tree::default();
    let at = |offset: usize| origin(source, component.html_origins[offset]);
    for token in html::tokens(&component.html) {
        let parent = tree.stack.last().and_then(|(_, index)| *index);
        match token {
            Token::StartTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name).into_owned();
                // Only the outer compiler template wrapper is delivery metadata.
                if tree.stack.is_empty()
                    && name == "template"
                    && html::attribute(&tag, template::COMPONENT_ATTRIBUTE.as_bytes()).is_some()
                {
                    tree.stack.push((name, None));
                    continue;
                }
                let attributes = node_attributes(source, component, authored, &tag);
                tree.element(&tag, name, attributes, at(tag.span.start));
            }
            Token::EndTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name);
                if let Some(index) = tree.stack.iter().rposition(|(open, _)| open == &name) {
                    tree.stack.truncate(index);
                }
            }
            Token::String(text) => tree.nodes.push(Node {
                parent,
                kind: NodeKind::Text {
                    value: String::from_utf8_lossy(&text).into_owned(),
                    anchor: None,
                },
                origin: at(text.span.start),
            }),
            Token::Comment(comment) => {
                let value = String::from_utf8_lossy(&comment);
                let Some(kind) = comment_kind(value) else {
                    continue;
                };
                tree.nodes.push(Node {
                    parent,
                    kind,
                    origin: at(comment.span.start),
                });
            }
            _ => {}
        }
    }
    Template {
        id: component.id.index(),
        nodes: tree.nodes,
        origin: origin(source, component.range.start),
    }
}

fn binding_anchor(binding: &Binding) -> crate::backend::Anchor {
    match binding.anchor() {
        ir::Anchor::Element(id) => crate::backend::Anchor::Element(id.index()),
        ir::Anchor::Text(id) => crate::backend::Anchor::Text(id.index()),
        ir::Anchor::Mount(id) => crate::backend::Anchor::Mount(id.index()),
    }
}

fn node_attributes(
    source: &str,
    component: &Component,
    authored: &BTreeMap<usize, StartTag<usize>>,
    tag: &StartTag<usize>,
) -> Vec<Attribute> {
    let at = |offset: usize| origin(source, component.html_origins[offset]);
    let original = authored.get(&at(tag.span.start).offset);
    tag.attributes
        .iter()
        .filter(|(name, _)| !template::reserved_attribute(&String::from_utf8_lossy(name)))
        .map(|(name, value)| Attribute {
            name: String::from_utf8_lossy(name).into_owned(),
            value: String::from_utf8_lossy(value).into_owned(),
            origin: original
                .and_then(|tag| tag.attributes.get(name.as_ref()))
                .map_or_else(
                    || at(value.span.start),
                    |value| origin(source, value.span.start),
                ),
        })
        .collect()
}

fn comment_kind(value: std::borrow::Cow<'_, str>) -> Option<NodeKind> {
    Some(
        match (MountMarker::parse(&value), TextMarker::parse(&value)) {
            (Ok(Some(MountMarker::Start(id))), _) => NodeKind::Mount { anchor: id.index() },
            (_, Ok(Some(TextMarker::Start(id)))) => NodeKind::Text {
                value: String::new(),
                anchor: Some(id.index()),
            },
            (Ok(Some(MountMarker::End(_))), _) | (_, Ok(Some(TextMarker::End(_)))) => {
                return None;
            }
            _ => NodeKind::Comment(value.into_owned()),
        },
    )
}

fn borrowed_structure(operation: OperationKind) -> OperationKind {
    match operation {
        OperationKind::Branch { read, prepare } => OperationKind::Branch {
            read: quote! { &#read },
            prepare: quote! { &#prepare },
        },
        OperationKind::Keyed { read, key, prepare } => OperationKind::Keyed {
            read: quote! { &#read },
            key: quote! { &#key },
            prepare: quote! { &#prepare },
        },
        _ => unreachable!("structural operation"),
    }
}

fn supplied_children<'a>(
    item: &'a Binding,
    ctx: Ctx,
) -> Option<(fusor::template::MountId, &'a [ChildFragment])> {
    match item {
        Binding::Invocation {
            point,
            children,
            inputs,
            ..
        } if children
            .iter()
            .any(|child| !ctx.components[child.body].empty)
            && !has_content(inputs) =>
        {
            Some((*point, children))
        }
        _ => None,
    }
}

#[derive(Default)]
struct Tree {
    nodes: Vec<Node>,
    stack: Vec<(String, Option<usize>)>,
}

impl Tree {
    fn element(
        &mut self,
        tag: &StartTag<usize>,
        name: String,
        attributes: Vec<Attribute>,
        origin: Origin,
    ) {
        let parent = self.stack.last().and_then(|(_, index)| *index);
        let anchor = html::attribute(tag, template::ELEMENT_ATTRIBUTE.as_bytes())
            .map(|id| id.parse::<usize>().expect("compiler id"));
        let text = html::attribute(tag, template::TEXT_ELEMENT_ATTRIBUTE.as_bytes())
            .map(|id| id.parse::<usize>().expect("compiler id"));
        let index = self.nodes.len();
        self.nodes.push(Node {
            parent,
            kind: NodeKind::Element {
                tag: name.clone(),
                attributes,
                anchor,
            },
            origin: origin.clone(),
        });
        if let Some(anchor) = text {
            self.nodes.push(Node {
                parent: Some(index),
                kind: NodeKind::Text {
                    value: String::new(),
                    anchor: Some(anchor),
                },
                origin: origin.clone(),
            });
        }
        if !tag.self_closing && !tags::void_element(&name) {
            self.stack.push((name, Some(index)));
        }
    }
}
