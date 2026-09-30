use crate::signature::{self, Flags};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    FnArg, ImplItem, ImplItemFn, ItemImpl, Type, Visibility, parse::Parser, spanned::Spanned,
};

struct Method<'a> {
    item: &'a ImplItemFn,
    input: Type,
    output: Type,
    error: Type,
    progress: Type,
    context: bool,
}
impl<'a> Method<'a> {
    fn parse(item: &'a ImplItemFn, constructor: bool) -> syn::Result<Self> {
        signature::validate(&item.sig)?;
        let (output, error) = signature::result(&item.sig)?;
        let mut args = item.sig.inputs.iter();
        if !constructor {
            if !matches!(args.next(), Some(FnArg::Receiver(r)) if matches!(&*r.ty,
                Type::Reference(reference) if reference.mutability.is_some()
                    && matches!(&*reference.elem, Type::Path(p) if p.path.is_ident("Self"))))
            {
                return Err(syn::Error::new(
                    item.sig.span(),
                    "worker methods require &mut self",
                ));
            }
        } else if !matches!(&output, Type::Path(p) if p.path.is_ident("Self")) {
            return Err(syn::Error::new(
                output.span(),
                "worker new must return TaskResult<Self, E>",
            ));
        }
        let mut types = args
            .map(|arg| match arg {
                FnArg::Typed(arg) => Ok((*arg.ty).clone()),
                FnArg::Receiver(r) => Err(syn::Error::new(
                    r.span(),
                    "worker constructor cannot take self",
                )),
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let context = types
            .last()
            .and_then(|ty| signature::arguments(ty, "TaskContext"));
        let progress = if let Some(args) = &context {
            if args.len() > 1 {
                return Err(syn::Error::new(
                    item.sig.span(),
                    "context takes one progress type",
                ));
            }
            types.pop();
            args.first()
                .cloned()
                .unwrap_or_else(|| syn::parse_quote!(()))
        } else {
            syn::parse_quote!(())
        };
        if types.len() != 1 {
            return Err(syn::Error::new(
                item.sig.span(),
                "worker calls take one owned input; use () for no input",
            ));
        }
        let input = types.pop().unwrap();
        signature::owned(&input)?;
        signature::owned(&progress)?;
        Ok(Self {
            item,
            input,
            output,
            error,
            progress,
            context: context.is_some(),
        })
    }
}

pub fn expand(attributes: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let flags = Flags::parse(attributes, false)?;
    let implementation: ItemImpl = syn::parse2(item)?;
    if implementation.trait_.is_some()
        || !implementation.generics.params.is_empty()
        || implementation.generics.where_clause.is_some()
    {
        return Err(syn::Error::new(
            implementation.span(),
            "worker requires a nongeneric inherent impl",
        ));
    }
    let Type::Path(path) = &*implementation.self_ty else {
        return Err(syn::Error::new(
            implementation.self_ty.span(),
            "expected a worker type",
        ));
    };
    if path
        .path
        .segments
        .iter()
        .any(|part| !matches!(part.arguments, syn::PathArguments::None))
    {
        return Err(syn::Error::new(
            path.span(),
            "worker types cannot be generic",
        ));
    }
    let name = &path.path.segments.last().unwrap().ident;
    let identity = path
        .path
        .segments
        .iter()
        .map(|part| {
            let ident = part.ident.to_string();
            let ident = ident.trim_start_matches("r#");
            format!("{}_{}", ident.len(), ident)
        })
        .collect::<Vec<_>>()
        .join("_");
    let module = format_ident!("__fusor_worker_{identity}");
    let mut scoped = path.clone();
    if scoped.path.leading_colon.is_none() && scoped.path.segments[0].ident != "crate" {
        if scoped.path.segments[0].ident == "self" {
            scoped.path.segments[0].ident = format_ident!("super");
        } else {
            scoped.path.segments.insert(0, syn::parse_quote!(super));
        }
    }
    let scoped = Type::Path(scoped);
    let mut methods = Vec::new();
    for item in &implementation.items {
        let ImplItem::Fn(item) = item else {
            continue;
        };
        if item.sig.ident == "new" || matches!(item.vis, Visibility::Public(_)) {
            methods.push(item);
        }
    }
    if !methods.iter().any(|item| item.sig.ident == "new") {
        return Err(syn::Error::new(
            implementation.span(),
            "worker requires new(input) -> TaskResult<Self, E>",
        ));
    }
    let pool = flags.pool;
    let methods = methods.iter().map(|item| {
        let attributes = &item.attrs;
        let signature = &item.sig;
        // Let rustc apply method-level cfg before validating or generating adapters.
        quote! {
            #[::fusor_worker::__private::__worker_method(#scoped, #pool)]
            #(#attributes)*
            #signature {}
        }
    });
    let metadata = signature::metadata(name, "worker", pool);
    Ok(quote! {
        #implementation
        #[doc(hidden)]
        #[allow(non_snake_case)]
        pub mod #module {
            #[allow(unused_imports)] use super::*;
            macro_rules! __ID_PREFIX { () => { concat!(env!("CARGO_PKG_NAME"), "@", env!("CARGO_PKG_VERSION"), ":", module_path!(), "::") }; }
            pub(super) const __CONSTRUCTOR: &str = concat!(__ID_PREFIX!(), "new");
            #metadata
            #[derive(Clone)]
            pub struct __Client(pub(super) ::fusor_worker::__private::Service);
            impl __Client {
                pub fn close(&self) -> ::fusor_worker::Close { self.0.close() }
            }
            #(#methods)*
        }
    })
}

pub fn expand_method(attributes: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let (ty, pool) = (|input: syn::parse::ParseStream| {
        let ty = input.parse::<Type>()?;
        input.parse::<syn::Token![,]>()?;
        Ok((ty, input.parse::<syn::LitBool>()?.value))
    })
    .parse2(attributes)?;
    let item: ImplItemFn = syn::parse2(item)?;
    if item.sig.ident == "close" {
        return Err(syn::Error::new(
            item.sig.ident.span(),
            "close is a reserved worker control name",
        ));
    }
    let constructor = item.sig.ident == "new";
    let method = Method::parse(&item, constructor)?;
    let input = signature::scoped(&method.input, Some(&ty));
    let output = signature::scoped(&method.output, Some(&ty));
    let error = signature::scoped(&method.error, Some(&ty));
    let progress = signature::scoped(&method.progress, Some(&ty));
    let proxy = if constructor {
        quote! {
            impl ::fusor_worker::Worker for #ty {
                type Input = #input;
                type InitError = #error;
                type InitProgress = #progress;
                type Client = __Client;
                fn __id() -> &'static str { __CONSTRUCTOR }
                fn __pool() -> bool { #pool }
                fn __client(service: ::fusor_worker::__private::Service) -> Self::Client { __Client(service) }
            }
        }
    } else {
        let name = &item.sig.ident;
        let attributes = signature::client_attributes(&item.attrs);
        quote! {
            impl __Client {
                #(#attributes)*
                pub fn #name(&self, input: #input) -> ::fusor_worker::Job<#output, #error, #progress> {
                    self.0.job(concat!(__ID_PREFIX!(), stringify!(#name)), move |codec| Ok(vec![::fusor_worker::__private::encode(&input, codec)?]))
                }
            }
        }
    };
    let adapter = adapter(&method, &ty, pool, constructor);
    Ok(quote! { #proxy #adapter })
}

fn adapter(method: &Method<'_>, ty: &Type, pool: bool, constructor: bool) -> TokenStream {
    let Method {
        item,
        input,
        progress,
        context,
        ..
    } = method;
    let input = signature::scoped(input, Some(ty));
    let progress = signature::scoped(progress, Some(ty));
    let name = &item.sig.ident;
    let adapter = format_ident!("__invoke_{name}");
    let ctx = context.then(|| quote!(, ctx.typed::<#progress>()));
    let await_call = item.sig.asyncness.is_some().then(|| quote!(.await));
    let (kind, execute) = if constructor {
        (
            "init",
            quote! {
                let state = <#ty>::new(input #ctx) #await_call;
                match state {
                    Ok(state) => Ok(ctx.store(state)),
                    Err(error) => ::fusor_worker::__private::encode_result::<(), _>(Err(error), &ctx.codec()),
                }
            },
        )
    } else {
        (
            "method",
            quote! {
                let mut state = ctx.take::<#ty>()?;
                let result = state.#name(input #ctx) #await_call;
                ctx.restore(state);
                ::fusor_worker::__private::encode_result(result, &ctx.codec())
            },
        )
    };
    quote! {
        ::fusor_worker::__private::inventory::submit! {
            ::fusor_worker::__private::Registration { id: concat!(__ID_PREFIX!(), stringify!(#name)), pool: #pool, kind: #kind, invoke: #adapter }
        }
        #[allow(deprecated)]
        fn #adapter(args: Vec<::fusor_worker::__private::Payload>, ctx: ::fusor_worker::__private::Context) -> ::fusor_worker::__private::Invocation {
            Box::pin(async move {
                let mut args = ::fusor_worker::__private::Inputs::new(args, ctx.codec());
                let input: #input = args.take()?;
                args.finish()?;
                #execute
            })
        }
    }
}
