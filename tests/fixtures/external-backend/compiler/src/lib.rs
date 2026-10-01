//! Test-only emitter. Every import here is a supported public API.
use fusor_build::{
    ExtractError,
    backend::{
        Anchor, Backend, Capability, ComponentCode, Control, NodeKind, Operation, OperationKind,
        Runtime, Template,
    },
};
use proc_macro2::TokenStream;
use quote::quote;
use syn::parse_quote;

pub struct Memory;

impl Backend for Memory {
    fn version(&self) -> u32 {
        1
    }
    fn name(&self) -> &str {
        "memory fixture"
    }
    fn runtime(&self) -> Runtime {
        Runtime {
            scope: parse_quote!(::memory_renderer::Scope),
            error: parse_quote!(::memory_renderer::Error),
            children: parse_quote!(::memory_renderer::Children),
            convert_error: parse_quote!(::memory_renderer::error),
        }
    }
    fn supports(&self, capability: Capability<'_>) -> bool {
        matches!(
            capability,
            Capability::Text
                | Capability::Event("click" | "input" | "change" | "blur")
                | Capability::Branch
                | Capability::Keyed
                | Capability::Component
                | Capability::Children
                | Capability::Bind(Control::Text | Control::Checkbox)
        )
    }
    fn validate_binding(
        &self,
        template: &Template,
        capability: Capability<'_>,
        anchor: Anchor,
        origin: &fusor_build::backend::Origin,
    ) -> Result<(), ExtractError> {
        if let Capability::Event(name) = capability {
            let control = template.nodes.iter().find_map(|node| match &node.kind {
                NodeKind::Element {
                    tag,
                    attributes,
                    anchor: Some(id),
                } if anchor == Anchor::Element(*id) => Some((
                    tag.as_str(),
                    attributes
                        .iter()
                        .find(|attribute| attribute.name == "type")
                        .map_or("text", |attribute| attribute.value.as_str()),
                )),
                _ => None,
            });
            let supported = match name {
                "click" => control.is_some_and(|(tag, _)| tag == "button"),
                "input" | "blur" => matches!(control, Some(("input", "text" | "number"))),
                "change" => control == Some(("input", "checkbox")),
                _ => false,
            };
            if !supported {
                return Err(origin.error("memory fixture events require a directly supported control; bubbling is unsupported"));
            }
        }
        Ok(())
    }
    fn validate(&self, template: &Template) -> Result<(), ExtractError> {
        for node in &template.nodes {
            if let NodeKind::Element {
                tag, attributes, ..
            } = &node.kind
            {
                if !matches!(
                    tag.as_str(),
                    "div" | "section" | "p" | "span" | "button" | "ul" | "li" | "input" | "label"
                ) {
                    return Err(node
                        .origin
                        .error(format!("memory fixture does not support element <{tag}>")));
                }
                for attribute in attributes {
                    if !matches!(attribute.name.as_str(), "id" | "class" | "type" | "value") {
                        return Err(attribute.origin.error(format!(
                            "memory fixture does not support attribute {}",
                            attribute.name
                        )));
                    }
                    if tag == "input"
                        && attribute.name == "type"
                        && !matches!(attribute.value.as_str(), "text" | "number" | "checkbox")
                    {
                        return Err(attribute.origin.error(
                            "memory fixture supports only text, number and checkbox input controls",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
    fn mount(&self, template: &Template) -> TokenStream {
        let nodes = template.nodes.iter().map(|node| {
            let parent = option(node.parent);
            let kind = match &node.kind {
                NodeKind::Element {
                    tag,
                    attributes,
                    anchor,
                } => {
                    let anchor = option(*anchor);
                    let attrs = attributes.iter().map(|attribute| {
                        let (name, value) = (&attribute.name, &attribute.value);
                        quote! { (#name, #value) }
                    });
                    quote! { ::memory_renderer::Kind::Element(#tag, &[#(#attrs),*], #anchor) }
                }
                NodeKind::Text { value, anchor } => {
                    let anchor = option(*anchor);
                    quote! { ::memory_renderer::Kind::Text(#value, #anchor) }
                }
                NodeKind::Mount { anchor } => quote! { ::memory_renderer::Kind::Mount(#anchor) },
                NodeKind::Comment(value) => quote! { ::memory_renderer::Kind::Comment(#value) },
            };
            quote! { ::memory_renderer::StaticNode { parent: #parent, kind: #kind } }
        });
        quote! { ::memory_renderer::Scope::new(parent, &[#(#nodes),*]) }
    }
    fn operation(&self, operation: Operation) -> TokenStream {
        let anchor = match operation.anchor {
            Anchor::Element(id) | Anchor::Text(id) | Anchor::Mount(id) => id,
        };
        match operation.kind {
            OperationKind::Text { read } => quote! { __fusor_scope.text(#anchor, #read)?; },
            OperationKind::Event { name, handler } => {
                quote! { __fusor_scope.on(#anchor, #name, #handler)?; }
            }
            OperationKind::Branch { read, prepare } => {
                quote! { __fusor_scope.branch(#anchor, #read, #prepare)?; }
            }
            OperationKind::Keyed { read, key, prepare } => {
                quote! { __fusor_scope.keyed(#anchor, #read, #key, #prepare)?; }
            }
            OperationKind::Component {
                ty,
                identity,
                make,
                children,
            } => quote! {
                __fusor_scope.component::<#ty, _, _, _>(#anchor, #identity, #make, #children)?;
            },
            OperationKind::Children { children } => {
                quote! { __fusor_scope.children(#anchor, #children)?; }
            }
            OperationKind::Bind {
                control: Control::Text,
                value,
                ..
            } => quote! { __fusor_scope.bind_text(#anchor, #value)?; },
            OperationKind::Bind {
                control: Control::Checkbox,
                value,
                choice,
            } => quote! { __fusor_scope.bind_checkbox(#anchor, #value, #choice)?; },
            _ => unreachable!("capabilities are checked before emission"),
        }
    }
    fn component(&self, component: ComponentCode) -> TokenStream {
        let ComponentCode {
            ty,
            body,
            app_state,
        } = component;
        assert!(
            app_state.is_none(),
            "fixture compiles reusable components only"
        );
        quote! {
            const _: () = assert!(::memory_renderer::VERSION == 1);
            const _: () = assert!(::fusor_components::BACKEND_VERSION == 1);
            #[allow(unused_variables, unused_braces, non_snake_case, clippy::unused_unit, clippy::unit_arg, clippy::clone_on_copy)]
            impl ::memory_renderer::Component for #ty {
                fn prepare(
                    parent: ::std::option::Option<&::fusor::OwnerHandle>,
                    make: ::std::boxed::Box<dyn FnOnce(::fusor::OwnerHandle) -> ::std::result::Result<Self, ::memory_renderer::Error> + '_>,
                    children: ::memory_renderer::Children,
                ) -> ::std::result::Result<::memory_renderer::Scope, ::memory_renderer::Error> {
                    children.with(|| { #body })
                }
            }
        }
    }
}

fn option(value: Option<usize>) -> TokenStream {
    match value {
        Some(value) => quote!(Some(#value)),
        None => quote!(None),
    }
}
