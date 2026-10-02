//! Built-in DOM compiler backend.
mod bindings;
mod coherent;
mod component;
mod template;

use crate::bindings::codegen::{self, *};
use crate::bindings::emit::{element, point, string, text};
use fusor::template::{ElementId, MountId, RootKind, TextId};
use proc_macro2::Literal;
use quote::format_ident;
use std::collections::BTreeMap;

pub(in crate::bindings) use component::delivery;

#[derive(Clone, Copy, Default)]
pub(super) struct DomBackend;

// Hoist recursive factories once before selecting an ordinary or coherent mode.
#[derive(Default)]
struct BrowserBinding {
    shared: TokenStream,
    ordinary: TokenStream,
    coherent: TokenStream,
}

/// Descriptor ordinals replace typed handles when the server supplies a bundle.
struct Bundle<'a> {
    elements: &'a BTreeMap<ElementId, u32>,
    texts: &'a BTreeMap<TextId, u32>,
}

impl CompilerBackend for DomBackend {
    fn runtime(&self) -> Runtime {
        Runtime {
            scope: syn::parse_quote!(::fusor::dom::Scope),
            error: syn::parse_quote!(::fusor::dom::JsValue),
            children: syn::parse_quote!(::fusor::dom::Children),
            convert_error: syn::parse_quote!(::fusor::dom::IntoMountError::into_mount_error),
            coherent_frame: Some(syn::parse_quote!(::fusor::dom::coherent::Frame)),
        }
    }

    fn component(&self, component: &Component, ctx: Ctx<'_>) -> TokenStream {
        component::emit(component, ctx)
    }

    fn binding(&self, binding: &Binding, ctx: Ctx<'_>, locals: &[Rust]) -> TokenStream {
        bindings::emit(binding, ctx, locals)
    }

    fn operation(
        &self,
        binding: &Binding,
        operation: OperationKind,
        _mode: OperationMode,
    ) -> TokenStream {
        let span = binding.span();
        let anchor = emit::handle(binding.anchor());
        match operation {
            OperationKind::Component {
                identity,
                make,
                children,
                ..
            } => {
                quote_spanned! {span=> __fusor_scope.component_at(&#anchor, #identity, #make, #children)?; }
            }
            OperationKind::Router { routes } => {
                let routes = routes.into_iter().map(|route| {
                    let prepare = route.prepare;
                    match route.pattern {
                        Some(pattern) => quote! {
                            ::fusor_router::browser::declarative::RouteView::new(#pattern, #prepare)?
                        },
                        None => quote! {
                            ::fusor_router::browser::declarative::RouteView::fallback(#prepare)
                        },
                    }
                });
                quote_spanned! {span=> {
                    ::fusor_router::browser::declarative::mount_routes(
                        &mut __fusor_scope, &#anchor, ::std::env!("FUSOR_BASE_PATH"),
                        ::std::vec![#(#routes),*]
                    )?;
                }}
            }
            _ => unreachable!("DOM leaf bindings use typed and bundled emission"),
        }
    }

    fn content(&self, factory: TokenStream) -> TokenStream {
        quote! { ::fusor::dom::Content::from_prepared(#factory) }
    }
}

fn literal(span: Span, value: &str) -> Literal {
    let mut literal = Literal::string(value);
    literal.set_span(span);
    literal
}
