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
            quote! { let state = ::std::rc::Rc::clone(state.as_ref()); }
        });
        let mount = self.backend.mount(&self.templates[component.id.index()]);
        let bindings = order_bindings(&component.bindings)
            .into_iter()
            .map(|item| binding(item, ctx, &component.async_locals));
        let body = quote! {
            #local_clones
            #incoming
            let mut __fusor_scope = (#mount)?;
            let state = make(__fusor_scope.owner())?;
            let state = __fusor_scope.retain_state(state);
            #expose_capture
            #(#bindings)*
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
                handler: quote_spanned! {span=> move |event| { #handler } },
            },
            Binding::Children { .. } => OperationKind::Children {
                children: quote! { __fusor_children.clone() },
            },
            Binding::Bind { control, value, .. } => {
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
                );
                return quote_spanned! {span=> {
                    #captures
                    {
                        let __fusor_bound = ::std::clone::Clone::clone(&(#value));
                        #operation
                    }
                } };
            }
            _ => unreachable!("validated structural bindings use common factories"),
        };
        let operation = self.operation(binding, kind);
        quote_spanned! {span=> { #captures #operation }}
    }

    fn operation(&self, binding: &Binding, kind: OperationKind) -> TokenStream {
        self.backend.operation(Operation {
            anchor: binding_anchor(binding),
            origin: origin(self.source, binding.origin().offset),
            kind,
        })
    }

    fn content(&self, _factory: TokenStream) -> TokenStream {
        unreachable!("projected content is rejected by external backend validation")
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
    // Source linkage is the caller's ordinary Rust module, never web script discovery.
    let mut authored = BTreeMap::new();
    for token in html::tokens(source) {
        if let Token::StartTag(tag) = token {
            for (name, value) in &tag.attributes {
                if name.as_ref() == b"hydrate" || name.starts_with(b"hydrate:") {
                    return Err(error(
                        source,
                        value.span.start,
                        "hydration is unsupported by backend v1",
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
    );
    Ok(GeneratedSource {
        rust,
        source_map: crate::SourceMap::new(locations).expect("ordered compiler locations"),
    })
}

// The v1 facade delivers component trees, not a browser document shell. Refuse
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
                "backend v1 requires markup inside rust:component or App; document shells and external assets belong to the build helper",
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
            "rust:render is a browser/server delivery contract; unsupported by backend v1",
        ));
    }
    if component.javascript.is_some() {
        return Err(error(
            source,
            component.range.start,
            "JavaScript component modules are unsupported by backend v1",
        ));
    }
    for binding in Binding::walk(&component.bindings) {
        let capability = match binding {
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
            _ => {
                return Err(error(
                    source,
                    binding.origin().offset,
                    "backend v1 does not support routing, hydration, coherent Async/Await, or opaque/projected content",
                ));
            }
        };
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
    let mut nodes = Vec::new();
    let mut stack: Vec<(String, Option<usize>)> = Vec::new();
    let at = |offset: usize| origin(source, component.html_origins[offset]);
    for token in html::tokens(&component.html) {
        let parent = stack.last().and_then(|(_, index)| *index);
        match token {
            Token::StartTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name).into_owned();
                // Only the outer compiler template wrapper is delivery metadata.
                if stack.is_empty()
                    && name == "template"
                    && html::attribute(&tag, template::COMPONENT_ATTRIBUTE.as_bytes()).is_some()
                {
                    stack.push((name, None));
                    continue;
                }
                let original = authored.get(&at(tag.span.start).offset);
                let attributes = tag
                    .attributes
                    .iter()
                    .filter(|(name, _)| {
                        !template::reserved_attribute(&String::from_utf8_lossy(name))
                    })
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
                    .collect();
                let anchor = html::attribute(&tag, template::ELEMENT_ATTRIBUTE.as_bytes())
                    .map(|id| id.parse::<usize>().expect("compiler id"));
                let text = html::attribute(&tag, template::TEXT_ELEMENT_ATTRIBUTE.as_bytes())
                    .map(|id| id.parse::<usize>().expect("compiler id"));
                let index = nodes.len();
                nodes.push(Node {
                    parent,
                    kind: NodeKind::Element {
                        tag: name.clone(),
                        attributes,
                        anchor,
                    },
                    origin: at(tag.span.start),
                });
                if let Some(anchor) = text {
                    nodes.push(Node {
                        parent: Some(index),
                        kind: NodeKind::Text {
                            value: String::new(),
                            anchor: Some(anchor),
                        },
                        origin: at(tag.span.start),
                    });
                }
                if !tag.self_closing && !tags::void_element(&name) {
                    stack.push((name, Some(index)));
                }
            }
            Token::EndTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name);
                if let Some(index) = stack.iter().rposition(|(open, _)| open == &name) {
                    stack.truncate(index);
                }
            }
            Token::String(text) => nodes.push(Node {
                parent,
                kind: NodeKind::Text {
                    value: String::from_utf8_lossy(&text).into_owned(),
                    anchor: None,
                },
                origin: at(text.span.start),
            }),
            Token::Comment(comment) => {
                let value = String::from_utf8_lossy(&comment);
                let kind = match (MountMarker::parse(&value), TextMarker::parse(&value)) {
                    (Ok(Some(MountMarker::Start(id))), _) => NodeKind::Mount { anchor: id.index() },
                    (_, Ok(Some(TextMarker::Start(id)))) => NodeKind::Text {
                        value: String::new(),
                        anchor: Some(id.index()),
                    },
                    (Ok(Some(MountMarker::End(_))), _) | (_, Ok(Some(TextMarker::End(_)))) => {
                        continue;
                    }
                    _ => NodeKind::Comment(value.into_owned()),
                };
                nodes.push(Node {
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
        nodes,
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
