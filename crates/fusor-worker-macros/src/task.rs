use crate::signature::{self, Flags};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemFn, Type, spanned::Spanned};

pub fn expand(attributes: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let flags = Flags::parse(attributes, true)?;
    let function: ItemFn = syn::parse2(item)?;
    signature::validate(&function.sig)?;
    let (output, error) = signature::result(&function.sig)?;
    let asynchronous = function.sig.asyncness.is_some();
    if flags.stream && (!asynchronous || !matches!(&output, Type::Tuple(t) if t.elems.is_empty())) {
        return Err(syn::Error::new(
            function.sig.span(),
            "stream producers must be async and return TaskResult<(), E>",
        ));
    }
    let mut types = function
        .sig
        .inputs
        .iter()
        .map(|argument| match argument {
            FnArg::Typed(argument) => Ok((*argument.ty).clone()),
            FnArg::Receiver(receiver) => Err(syn::Error::new(
                receiver.span(),
                "task requires a free function",
            )),
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let batch = if flags.stream {
        let sender_error =
            |span| syn::Error::new(span, "stream producer requires a final StreamSender<T>");
        let ty = types
            .pop()
            .ok_or_else(|| sender_error(function.sig.span()))?;
        let args = signature::arguments(&ty, "StreamSender")
            .filter(|args| args.len() == 1)
            .ok_or_else(|| sender_error(ty.span()))?;
        signature::owned(&args[0])?;
        Some(args[0].clone())
    } else {
        None
    };
    let context_name = if asynchronous {
        "TaskContext"
    } else {
        "ComputeContext"
    };
    let context = types
        .last()
        .and_then(|ty| signature::arguments(ty, context_name));
    let progress: Type = if let Some(args) = &context {
        if args.len() > 1 {
            return Err(syn::Error::new(
                types.last().unwrap().span(),
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
    for ty in &types {
        if signature::arguments(ty, "TaskContext").is_some()
            || signature::arguments(ty, "ComputeContext").is_some()
            || signature::arguments(ty, "StreamSender").is_some()
        {
            return Err(syn::Error::new(
                ty.span(),
                "context must be final (before the stream sender) and match sync/async execution",
            ));
        }
        signature::owned(ty)?;
    }
    signature::owned(&progress)?;
    let types: Vec<_> = types.iter().map(|ty| signature::scoped(ty, None)).collect();
    let output = signature::scoped(&output, None);
    let error = signature::scoped(&error, None);
    let progress = signature::scoped(&progress, None);
    let batch = batch.map(|ty| signature::scoped(&ty, None));
    let attributes = signature::client_attributes(&function.attrs);
    let name = &function.sig.ident;
    let visibility = &function.vis;
    let names: Vec<_> = (0..types.len())
        .map(|i| format_ident!("__input_{i}"))
        .collect();
    let inputs = names
        .iter()
        .zip(&types)
        .map(|(name, ty)| quote!(#name: #ty));
    let decodes = names.iter().zip(&types).map(|(name, ty)| {
        quote! {
            let #name: #ty = __args.take()?;
        }
    });
    // No leading comma when the function takes only an injected context/sender.
    let mut call_args: Vec<TokenStream> = names.iter().map(|name| quote!(#name)).collect();
    if context.is_some() {
        call_args.push(if asynchronous {
            quote!(__ctx.typed::<#progress>())
        } else {
            quote!(__ctx.compute_context::<#progress>())
        });
    }
    if let Some(batch) = &batch {
        call_args.push(quote!(__ctx.sender::<#batch>()));
    }
    let await_call = asynchronous.then(|| quote!(.await));
    let kind = if flags.stream {
        "stream"
    } else if asynchronous {
        "async"
    } else {
        "sync"
    };
    let metadata = signature::metadata(name, kind, flags.pool);
    let pool = flags.pool;
    let (method, constructor, result, output) = match batch {
        Some(batch) => (quote!(stream), quote!(stream), quote!(ResultStream), batch),
        None => (quote!(run), quote!(job), quote!(Job), output),
    };
    Ok(quote! {
        #function
        #visibility mod #name {
            #[allow(unused_imports)] use super::*;
            const __ID: &str = concat!(env!("CARGO_PKG_NAME"), "@", env!("CARGO_PKG_VERSION"), ":", module_path!());
            #metadata
            #(#attributes)*
            pub fn #method(__owner: &::fusor_worker::__private::OwnerHandle, #(#inputs),*) -> ::fusor_worker::#result<#output, #error, #progress, ::fusor_worker::Unbound> {
                ::fusor_worker::__private::#constructor(__owner, __ID, #pool, move |__codec| {
                    let mut __arguments = ::fusor_worker::__private::Arguments::new(__codec);
                    #(__arguments.push(&#names)?;)*
                    Ok(__arguments.finish())
                })
            }
            ::fusor_worker::__private::inventory::submit! {
                ::fusor_worker::__private::Registration { id: __ID, pool: #pool, kind: #kind, invoke: __invoke }
            }
            #[allow(deprecated)]
            fn __invoke(__arguments: Vec<::fusor_worker::__private::Payload>, __ctx: ::fusor_worker::__private::Context) -> ::fusor_worker::__private::Invocation {
                Box::pin(async move {
                    let mut __args = ::fusor_worker::__private::Inputs::new(__arguments, __ctx.codec());
                    #(#decodes)*
                    __args.finish()?;
                    let __result = super::#name(#(#call_args),*) #await_call;
                    ::fusor_worker::__private::encode_result(__result, &__ctx.codec())
                })
            }
        }
    })
}
