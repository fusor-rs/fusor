use proc_macro2::TokenStream;
use quote::quote;
use syn::{GenericArgument, PathArguments, ReturnType, Signature, Type, spanned::Spanned};

fn error(node: &impl Spanned, message: &str) -> syn::Error {
    syn::Error::new(node.span(), message)
}

pub struct Flags {
    pub pool: bool,
    pub stream: bool,
}
impl Flags {
    pub fn parse(tokens: TokenStream, stream_allowed: bool) -> syn::Result<Self> {
        use syn::parse::Parser;
        let names = syn::punctuated::Punctuated::<syn::Ident, syn::Token![,]>::parse_terminated
            .parse2(tokens)?;
        let mut flags = Self {
            pool: false,
            stream: false,
        };
        for name in names {
            let slot = match name.to_string().as_str() {
                "pool" => &mut flags.pool,
                "stream" if stream_allowed => &mut flags.stream,
                _ => {
                    return Err(error(
                        &name,
                        if stream_allowed {
                            "expected pool or stream"
                        } else {
                            "expected pool"
                        },
                    ));
                }
            };
            if *slot {
                return Err(error(&name, "duplicate worker flag"));
            }
            *slot = true;
        }
        Ok(flags)
    }
}

pub fn validate(signature: &Signature) -> syn::Result<()> {
    if !signature.generics.params.is_empty() || signature.generics.where_clause.is_some() {
        return Err(error(
            &signature.generics,
            "worker entry points cannot be generic",
        ));
    }
    if signature.unsafety.is_some()
        || signature.abi.is_some()
        || signature.variadic.is_some()
        || signature.constness.is_some()
    {
        return Err(error(
            signature,
            "worker entry points must be ordinary safe Rust functions",
        ));
    }
    Ok(())
}

pub fn result(signature: &Signature) -> syn::Result<(Type, Type)> {
    let ReturnType::Type(_, ty) = &signature.output else {
        return Err(error(
            &signature.output,
            "worker entry points must return TaskResult<T, E>",
        ));
    };
    let args = arguments(ty, "TaskResult")
        .filter(|args| matches!(args.len(), 1..=2))
        .ok_or_else(|| error(ty, "expected TaskResult<T, E>"))?;
    owned(&args[0])?;
    let error = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| syn::parse_quote!(::fusor_worker::NoError));
    owned(&error)?;
    Ok((args[0].clone(), error))
}

pub fn arguments(ty: &Type, name: &str) -> Option<Vec<Type>> {
    let Type::Path(path) = ty else {
        return None;
    };
    let last = path.path.segments.last()?;
    if last.ident != name {
        return None;
    }
    match &last.arguments {
        PathArguments::None => Some(Vec::new()),
        PathArguments::AngleBracketed(args) => args
            .args
            .iter()
            .map(|arg| {
                if let GenericArgument::Type(ty) = arg {
                    Some(ty.clone())
                } else {
                    None
                }
            })
            .collect(),
        _ => None,
    }
}

pub fn take_context(types: &mut Vec<Type>, name: &str) -> syn::Result<Option<Type>> {
    let Some((span, args)) = types
        .last()
        .and_then(|ty| arguments(ty, name).map(|args| (ty.span(), args)))
    else {
        return Ok(None);
    };
    if args.len() > 1 {
        return Err(syn::Error::new(span, "context takes one progress type"));
    }
    types.pop();
    Ok(Some(
        args.into_iter()
            .next()
            .unwrap_or_else(|| syn::parse_quote!(())),
    ))
}

pub fn owned(ty: &Type) -> syn::Result<()> {
    use syn::visit::Visit;
    #[derive(Default)]
    struct Check(Option<syn::Error>);
    impl<'ast> Visit<'ast> for Check {
        fn visit_type(&mut self, ty: &'ast Type) {
            if matches!(
                ty,
                Type::Reference(_) | Type::ImplTrait(_) | Type::Infer(_) | Type::Ptr(_)
            ) {
                self.0 = Some(error(
                    ty,
                    "worker messages must have an owned, concrete type",
                ));
            } else {
                syn::visit::visit_type(self, ty);
            }
        }
    }
    let mut check = Check::default();
    check.visit_type(ty);
    check.0.map_or(Ok(()), Err)
}

pub fn metadata(name: &syn::Ident, kind: &str, pool: bool) -> TokenStream {
    let pool = if pool { "pool" } else { "ordinary" };
    quote! {
        // wasm-bindgen's custom-section expansion resolves this name locally.
        #[cfg(target_arch = "wasm32")]
        use ::fusor_worker::__private::wasm_bindgen;
        #[cfg(target_arch = "wasm32")]
        #[wasm_bindgen::prelude::wasm_bindgen(typescript_custom_section)]
        const __FUSOR_WORKER_METADATA: &str = concat!(
            "\n// fusor-worker:1:", env!("CARGO_PKG_NAME"), ":", env!("CARGO_PKG_VERSION"), ":",
            module_path!(), "::", stringify!(#name), ":", #kind, ":", #pool, "\n"
        );
    }
}

pub fn client_attributes(attributes: &[syn::Attribute]) -> Vec<&syn::Attribute> {
    attributes
        .iter()
        .filter(|attr| attr.path().is_ident("doc") || attr.path().is_ident("deprecated"))
        .collect()
}

// Generated adapters live one module below the authored signature.
pub fn scoped(ty: &Type, state: Option<&Type>) -> Type {
    use syn::visit_mut::VisitMut;
    struct Scope<'a>(Option<&'a Type>);
    impl VisitMut for Scope<'_> {
        fn visit_path_mut(&mut self, path: &mut syn::Path) {
            syn::visit_mut::visit_path_mut(self, path);
            if path.leading_colon.is_none() {
                let first = &mut path.segments[0].ident;
                if first == "Self" {
                    if let Some(Type::Path(state)) = self.0 {
                        let mut scoped = state.path.clone();
                        scoped
                            .segments
                            .extend(path.segments.iter().skip(1).cloned());
                        *path = scoped;
                    }
                } else if first == "self" {
                    *first = syn::Ident::new("super", first.span());
                } else if first == "super" {
                    path.segments.insert(0, syn::parse_quote!(super));
                }
            }
        }
    }
    let mut ty = ty.clone();
    Scope(state).visit_type_mut(&mut ty);
    ty
}
