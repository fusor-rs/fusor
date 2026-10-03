//! Token helpers shared by compiler lowerings: generated names, lint allowances
//! and portable typed construction.
use super::{
    ir::{Anchor, Input, InputValue, InterpolatedString, StringPart},
    tokens::Rust,
};
use fusor::template::{ElementId, MountId, TextId};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};

/// A generated local, `__fusor_<kind>_<index>`.
pub(super) fn indexed(kind: &str, index: usize) -> Ident {
    format_ident!("__fusor_{}_{}", kind, index)
}

/// A generated constant, `__FUSOR_<KIND>_<index>`.
pub(super) fn indexed_constant(kind: &str, index: usize) -> Ident {
    format_ident!("__FUSOR_{}_{}", kind, index)
}

pub(super) fn element(id: ElementId) -> Ident {
    indexed("element", id.index())
}

pub(super) fn point(id: MountId) -> Ident {
    indexed("mount", id.index())
}

pub(super) fn text(id: TextId) -> Ident {
    indexed("text", id.index())
}

/// The name of an enum variant shared with a runtime crate, to write after its
/// path: `::fusor_islands::Activation::#variant`.
pub(super) fn variant(span: Span, value: impl std::fmt::Debug) -> Ident {
    Ident::new(&format!("{value:?}"), span)
}

/// The typed handle the template hands out for a binding's anchor.
pub(super) fn handle(anchor: Anchor) -> Ident {
    match anchor {
        Anchor::Element(id) => element(id),
        Anchor::Mount(id) => point(id),
        Anchor::Text(id) => text(id),
    }
}

/// Give a closure its own copy of each lexical local it reads.
pub(super) fn clone_locals(locals: &[Rust]) -> TokenStream {
    quote! { #(
        #[allow(unused_variables, clippy::clone_on_copy, reason = "lexical captures have inferred types and may be unused in this closure")]
        let #locals = ::std::clone::Clone::clone(&#locals);
    )* }
}

/// Preserve the typed construction contract while letting a renderer convert
/// errors at its own mounting boundary.
pub(super) fn construct_inputs(
    span: Span,
    ty: &Rust,
    fields: impl IntoIterator<Item = TokenStream>,
    converter: impl quote::ToTokens,
) -> TokenStream {
    let fields = fields.into_iter();
    quote_spanned! {span=>
        type __FusorInputs = <#ty as ::fusor::FromInputs>::Inputs;
        <#ty as ::fusor::FromInputs>::from_inputs(__FusorInputs { #(#fields),* }, owner)
            .map_err(#converter)
    }
}

/// The `name: value` fields of a component's inputs struct; `value` lowers each input.
pub(super) fn fields(
    inputs: &[Input],
    value: impl Fn(&InputValue) -> TokenStream,
) -> Vec<TokenStream> {
    inputs
        .iter()
        .map(|input| {
            let name = &input.name;
            let value = value(&input.value);
            quote_spanned! {name.span()=> #name: #value }
        })
        .collect()
}

/// An expression or literal input as its own block.
pub(super) fn braced(value: &InputValue) -> TokenStream {
    let value = value
        .value()
        .expect("projected content is lowered by its caller");
    quote_spanned! {value.span()=> { #value } }
}

/// An interpolated attribute value as an owned `String`.
pub(super) fn string(value: &InterpolatedString) -> TokenStream {
    if let Some(text) = value.as_literal() {
        return quote! { ::std::string::String::from(#text) };
    }
    let (format, expressions) = format_parts(value);
    if let ("{}", [expression]) = (format.as_str(), expressions.as_slice()) {
        quote! { ::std::string::ToString::to_string(&(#expression)) }
    } else {
        quote! { ::std::format!(#format #(, (#expressions))*) }
    }
}

/// An interpolated attribute value as `format_args!`, for a writer that escapes
/// it at once, so borrowed arguments never outlive their statement.
pub(super) fn format_args(value: &InterpolatedString) -> TokenStream {
    let (format, expressions) = format_parts(value);
    quote! { ::std::format_args!(#format #(, (#expressions))*) }
}

fn format_parts(value: &InterpolatedString) -> (String, Vec<&Rust>) {
    let mut format = String::new();
    let mut expressions = Vec::new();
    for part in &value.0 {
        match part {
            StringPart::Literal(text) => {
                format.push_str(&text.replace('{', "{{").replace('}', "}}"));
            }
            StringPart::Expression(expression) => {
                format.push_str("{}");
                expressions.push(expression);
            }
        }
    }
    (format, expressions)
}

/// An authored optional expression, or the value used when it is absent.
pub(super) fn or(value: Option<&Rust>, absent: TokenStream) -> TokenStream {
    value.map_or(absent, |value| quote! { #value })
}

/// A mounted component's optional identity, with unit identity when unkeyed.
pub(super) fn identity(condition: Option<&Rust>, key: Option<&Rust>) -> TokenStream {
    let key = key.map_or_else(
        || quote! { ::std::option::Option::Some(()) },
        |key| quote_spanned! {key.span()=> ::std::option::Option::Some({ #key }) },
    );
    match condition {
        Some(condition) => quote_spanned! {condition.span()=>
            if #condition { #key } else { ::std::option::Option::None }
        },
        None => key,
    }
}

pub(super) fn option(value: Option<TokenStream>) -> TokenStream {
    match value {
        Some(value) => quote! { ::std::option::Option::Some(#value) },
        None => quote! { ::std::option::Option::None },
    }
}
