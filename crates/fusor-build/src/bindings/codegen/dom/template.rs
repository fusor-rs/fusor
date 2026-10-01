use super::{Bundle, Ctx, bindings::scoped, captures};
use crate::bindings::{
    emit::{self, element, point, text},
    ir::{Anchor, Binding, Component},
    tokens::Rust,
};
use fusor::template::{self, ChildPolicy, MountId};
use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};

pub(super) struct TemplateCode {
    pub(super) declarations: TokenStream,
    pub(super) typed_handles: TokenStream,
    pub(super) bundle: Option<Vec<TokenStream>>,
}

pub(super) fn lower(component: &Component, ctx: Ctx) -> TemplateCode {
    let id = component.id.index();
    let version = template::VERSION;
    let kind = emit::variant(Span::call_site(), component.kind());
    let elements = component.elements.iter().map(|node| {
        let id = node.id.index();
        let tag = &node.tag;
        let children = emit::variant(Span::call_site(), node.children);
        quote! { ::fusor::template::ElementDescriptor {
            id: ::fusor::template::ElementId::new(#id), tag: #tag, children: ::fusor::template::ChildPolicy::#children,
        }}
    });
    let texts = component.texts.iter().map(|id| {
        let id = id.index();
        quote! { ::fusor::template::TextId::new(#id) }
    });
    let text_elements = component.text_elements.iter().map(|element| {
        let id = element.id.index();
        let tag = &element.tag;
        let host = emit::option(element.host.map(|id| {
            let id = id.index();
            quote! { ::fusor::template::ElementId::new(#id) }
        }));
        quote! { ::fusor::template::TextElementDescriptor {
            id: ::fusor::template::TextId::new(#id), host: #host, tag: #tag,
        } }
    });
    let handles = component.elements.iter().map(|node| {
        let name = element(node.id);
        let id = node.id.index();
        if node.tag == "input" {
            quote! { let #name = __fusor_nodes.take_input(::fusor::template::ElementId::new(#id))?; }
        } else {
            quote! { let #name = __fusor_nodes.take_element(::fusor::template::ElementId::new(#id))?; }
        }
    });
    let text_handles = component
        .texts
        .iter()
        .chain(component.text_elements.iter().map(|element| &element.id))
        .map(|id| {
            let name = text(*id);
            let id = id.index();
            quote! { let #name = __fusor_nodes.take_text(::fusor::template::TextId::new(#id))?; }
        });
    let mounts: Vec<MountId> = Binding::walk(&component.bindings)
        .into_iter()
        .filter_map(|binding| match binding.anchor() {
            Anchor::Mount(point) => Some(point),
            _ => None,
        })
        .collect();
    let mount_ids = mounts.iter().map(|id| {
        let id = id.index();
        quote! { ::fusor::template::MountId::new(#id) }
    });
    let mount_handles = mounts.iter().map(|id| {
        let name = point(*id);
        let id = id.index();
        quote! { let #name = __fusor_nodes.take_mount_point(::fusor::template::MountId::new(#id))?; }
    });
    TemplateCode {
        declarations: quote! {
            const __FUSOR_TEMPLATE: ::fusor::template::TemplateDescriptor = ::fusor::template::TemplateDescriptor {
                version: #version,
                component: ::fusor::template::ComponentId::new(#id),
                kind: ::fusor::template::RootKind::#kind,
                elements: &[#(#elements),*],
                texts: &[#(#texts),*],
                text_elements: &[#(#text_elements),*],
            };
            const __FUSOR_MOUNTS: &[::fusor::template::MountId] = &[#(#mount_ids),*];
        },
        typed_handles: quote! {
            #(#handles)*
            #(#text_handles)*
            #(#mount_handles)*
        },
        bundle: binding_bundle(component, ctx, &component.async_locals),
    }
}

// The bundle path preserves ordinary binding effects. Inputs, managed regions,
// fragments and all other binding kinds keep the existing typed interface.
fn binding_bundle(component: &Component, ctx: Ctx, locals: &[Rust]) -> Option<Vec<TokenStream>> {
    if component.fragment()
        || component.bindings.is_empty()
        || component.elements.iter().any(|element| {
            element.children != ChildPolicy::Static
                || matches!(element.tag.as_str(), "input" | "textarea" | "select")
        })
    {
        return None;
    }
    // IDs are global and sparse; the bundle uses descriptor ordinals, not
    // raw IDs. Its Text section follows elements, then anchored/direct order.
    let elements = component
        .elements
        .iter()
        .enumerate()
        .map(|(index, element)| (element.id, u32::try_from(index).expect("bundle too large")))
        .collect();
    let texts = component
        .texts
        .iter()
        .chain(component.text_elements.iter().map(|element| &element.id))
        .enumerate()
        .map(|(index, id)| {
            (
                *id,
                u32::try_from(component.elements.len() + index).expect("bundle too large"),
            )
        })
        .collect();
    let bundle = Bundle {
        elements: &elements,
        texts: &texts,
    };
    component
        .bindings
        .iter()
        .map(|binding| {
            let operation = scoped(binding, Some(&bundle))?;
            let captures = captures(binding.span(), ctx, locals);
            Some(quote_spanned! {binding.span()=> {
                #captures
                #operation
            }})
        })
        .collect()
}
