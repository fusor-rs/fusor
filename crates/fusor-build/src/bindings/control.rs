//! Structural control flow. Rust patterns are parsed by syn and checked by rustc.
use super::{ir::*, tag_input::TagInput, tokens::Rust};
use crate::ExtractError;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::Parser;

pub(super) fn pattern(input: &TagInput) -> Result<(Rust, Vec<Rust>), ExtractError> {
    let source = input.source;
    input.accepts(&["pattern"], "only pattern=\"Rust pattern\"")?;
    let (value, offset) = input
        .text("pattern")
        .ok_or_else(|| input.error("Case requires pattern=\"Rust pattern\""))?;
    let pattern = syn::Pat::parse_multi_with_leading_vert
        .parse_str(&value)
        .map_err(|e| input.error_at(offset, format!("invalid Rust pattern: {e}")))?;
    let mut bindings = Vec::new();
    pattern_names(&pattern, &mut bindings).map_err(|e| input.error_at(offset, e))?;
    let bindings = bindings
        .into_iter()
        .map(|name| {
            let text = name.to_string();
            let text = text.trim_start_matches("r#");
            if super::tags::reserved_scope_name(text) {
                return Err(
                    input.error_at(offset, "Case bindings cannot shadow framework scope names")
                );
            }
            Rust::new(source, name.into_token_stream(), offset)
        })
        .collect::<Result<_, _>>()?;
    Ok((
        Rust::new(source, pattern.into_token_stream(), offset)?,
        bindings,
    ))
}

// Nested pairs have no tuple-arity trait limit. Only the selected case owns data.
fn pair(values: impl DoubleEndedIterator<Item = TokenStream>) -> TokenStream {
    values
        .rev()
        .fold(quote! { () }, |tail, head| quote! { (#head, #tail) })
}
fn field(index: usize) -> TokenStream {
    let tails = (0..index).map(|_| quote! { .1 });
    quote! { #(#tails)* .0 }
}

pub(super) fn selection(
    value: &Rust,
    cases: &[CaseBranch],
    snapshots: &[(Rust, Rust)],
) -> TokenStream {
    let environment = pair(
        snapshots
            .iter()
            .map(|(_, value)| quote! { ::fusor_components::Captured::new(#value) }),
    );
    let arms = cases.iter().enumerate().map(|(index, case)| {
        let pattern = &case.pattern;
        let payload = pair(case.names.iter().map(|name| quote! { #name }));
        let data = pair(cases.iter().enumerate().map(|(other, _)| {
            if other == index {
                quote! { ::std::option::Option::Some(#payload) }
            } else {
                quote! { ::std::option::Option::None }
            }
        }));
        quote! { #pattern => (#index, (#data, #environment)) }
    });
    quote! {{ let __fusor_value = { #value }; #[deny(non_snake_case)] match __fusor_value { #(#arms),* } }}
}

/// Each lexical capture is a normal Memo, identical to ForEach's public contract.
pub(super) fn projections(
    case: &CaseBranch,
    index: usize,
    snapshots: &[(Rust, Rust)],
) -> TokenStream {
    let variant = field(index);
    let fields = case.names.iter().enumerate().map(|(i, name)| {
        let field = field(i);
        quote! {
            #[allow(unused_variables, reason = "a Case may not read every captured field")]
            let #name = {
                let __fusor_data = __fusor_data.clone();
                ::fusor::memo(move || __fusor_data.with(|__fusor_data| {
                    #[allow(clippy::clone_on_copy, reason = "pattern captures have inferred types and may not be Copy")]
                    __fusor_data.0 #variant .as_ref().expect("active branch data") #field .clone()
                }))
            };
        }
    });
    let environment = snapshots.iter().enumerate().map(|(index, (name, _))| {
        let field = field(index);
        quote! {
            #[allow(unused_variables, reason = "a Case may not read every enclosing capture")]
            let #name = { let __fusor_data = __fusor_data.clone(); ::fusor::derived(move || __fusor_data.with(|__fusor_data| __fusor_data.1 #field .get())) };
        }
    });
    quote! { #(#fields)* #(#environment)* }
}

// Visit binding positions only. Paths and struct member names are not locals.
fn pattern_names(pat: &syn::Pat, result: &mut Vec<syn::Ident>) -> Result<(), &'static str> {
    match pat {
        syn::Pat::Ident(p) => {
            if p.by_ref.is_some() || p.mutability.is_some() {
                return Err(
                    "Case captures are owned read-only reactive values; use owned patterns without ref or mut",
                );
            }
            // syn cannot resolve a bare identifier as a unit variant,
            // constant, or binding. Reserve snake_case for captures, as in
            // ordinary Rust style; leave capitalized names (including None)
            // to rustc's pattern/name resolution without inventing a local.
            if !p.ident.to_string().chars().any(char::is_uppercase) && !result.contains(&p.ident) {
                result.push(p.ident.clone());
            }
            if let Some((_, sub)) = &p.subpat {
                pattern_names(sub, result)?;
            }
        }
        syn::Pat::Or(p) => {
            for case in &p.cases {
                pattern_names(case, result)?;
            }
        }
        syn::Pat::Paren(p) => pattern_names(&p.pat, result)?,
        syn::Pat::Slice(p) => {
            for item in &p.elems {
                pattern_names(item, result)?;
            }
        }
        syn::Pat::Struct(p) => {
            for field in &p.fields {
                pattern_names(&field.pat, result)?;
            }
        }
        syn::Pat::Tuple(p) => {
            for item in &p.elems {
                pattern_names(item, result)?;
            }
        }
        syn::Pat::TupleStruct(p) => {
            for item in &p.elems {
                pattern_names(item, result)?;
            }
        }
        syn::Pat::Reference(_) | syn::Pat::Macro(_) | syn::Pat::Verbatim(_) => {
            return Err("Case requires an owned Rust pattern without references or pattern macros");
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn value(
    source: &str,
    tag: &html5gum::StartTag<usize>,
    builtin: super::tags::BuiltIn,
) -> Result<Rust, ExtractError> {
    use super::tags::BuiltIn;
    let input = TagInput::new(source, tag, builtin.spelling());
    let attribute = if builtin == BuiltIn::If {
        "condition"
    } else {
        "value"
    };
    input.accepts(&[attribute], &format!("only {attribute}"))?;
    input.expression(attribute)
}
