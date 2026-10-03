use crate::signature::{self, Flags};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemFn, Type, spanned::Spanned};

pub fn expand(attributes: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let flags = Flags::parse(attributes, true)?;
    let function: ItemFn = syn::parse2(item)?;
    let task = Task::parse(&function, &flags)?;
    let name = &function.sig.ident;
    let visibility = &function.vis;
    let kind = match (flags.stream, function.sig.asyncness.is_some()) {
        (true, _) => "stream",
        (false, true) => "async",
        (false, false) => "sync",
    };
    let metadata = signature::metadata(name, kind, flags.pool);
    let client = task.client(&function, flags.pool);
    let adapter = task.adapter(&function);
    let pool = flags.pool;
    Ok(quote! {
        #function
        #visibility mod #name {
            #[allow(unused_imports, reason = "generated adapters resolve types in the task's module")] use super::*;
            const __ID: &str = concat!(env!("CARGO_PKG_NAME"), "@", env!("CARGO_PKG_VERSION"), ":", module_path!());
            #metadata
            #client
            ::fusor_worker::__private::inventory::submit! {
                ::fusor_worker::__private::Registration { id: __ID, pool: #pool, kind: #kind, invoke: __invoke }
            }
            #adapter
        }
    })
}

struct Task {
    inputs: Vec<Type>,
    output: Type,
    error: Type,
    context: Option<Type>,
    batch: Option<Type>,
}

impl Task {
    fn parse(function: &ItemFn, flags: &Flags) -> syn::Result<Self> {
        signature::validate(&function.sig)?;
        let (output, error) = signature::result(&function.sig)?;
        let asynchronous = function.sig.asyncness.is_some();
        if flags.stream
            && (!asynchronous || !matches!(&output, Type::Tuple(t) if t.elems.is_empty()))
        {
            return Err(syn::Error::new(
                function.sig.span(),
                "stream producers must be async and return TaskResult<(), E>",
            ));
        }
        let mut inputs = function
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
        let batch = flags
            .stream
            .then(|| take_sender(&mut inputs, function))
            .transpose()?;
        let context_name = if asynchronous {
            "TaskContext"
        } else {
            "ComputeContext"
        };
        let context = signature::take_context(&mut inputs, context_name)?;
        for ty in &inputs {
            if ["TaskContext", "ComputeContext", "StreamSender"]
                .iter()
                .any(|name| signature::arguments(ty, name).is_some())
            {
                return Err(syn::Error::new(
                    ty.span(),
                    "context must be final (before the stream sender) and match sync/async execution",
                ));
            }
            signature::owned(ty)?;
        }
        if let Some(progress) = &context {
            signature::owned(progress)?;
        }
        Ok(Self {
            inputs: inputs
                .iter()
                .map(|ty| signature::scoped(ty, None))
                .collect(),
            output: signature::scoped(&output, None),
            error: signature::scoped(&error, None),
            context: context.map(|ty| signature::scoped(&ty, None)),
            batch: batch.map(|ty| signature::scoped(&ty, None)),
        })
    }

    fn client(&self, function: &ItemFn, pool: bool) -> TokenStream {
        let names = input_names(&self.inputs);
        let inputs = names
            .iter()
            .zip(&self.inputs)
            .map(|(name, ty)| quote!(#name: #ty));
        let attributes = signature::client_attributes(&function.attrs);
        let error = &self.error;
        let progress = match &self.context {
            Some(progress) => quote!(#progress),
            None => quote!(()),
        };
        let (method, constructor, result, output) = match &self.batch {
            Some(batch) => (quote!(stream), quote!(stream), quote!(ResultStream), batch),
            None => (quote!(run), quote!(job), quote!(Job), &self.output),
        };
        quote! {
            #(#attributes)*
            pub fn #method(__owner: &::fusor_worker::__private::OwnerHandle, #(#inputs),*) -> ::fusor_worker::#result<#output, #error, #progress, ::fusor_worker::Unbound> {
                ::fusor_worker::__private::#constructor(__owner, __ID, #pool, move |__codec| {
                    let mut __arguments = ::fusor_worker::__private::Arguments::new(__codec);
                    #(__arguments.push(&#names)?;)*
                    Ok(__arguments.finish())
                })
            }
        }
    }

    fn adapter(&self, function: &ItemFn) -> TokenStream {
        let names = input_names(&self.inputs);
        let decodes = names
            .iter()
            .zip(&self.inputs)
            .map(|(name, ty)| quote! { let #name: #ty = __args.take()?; });
        let mut call_args: Vec<_> = names.iter().map(|name| quote!(#name)).collect();
        if let Some(progress) = &self.context {
            call_args.push(if function.sig.asyncness.is_some() {
                quote!(__ctx.typed::<#progress>())
            } else {
                quote!(__ctx.compute_context::<#progress>())
            });
        }
        if let Some(batch) = &self.batch {
            call_args.push(quote!(__ctx.sender::<#batch>()));
        }
        let await_call = function.sig.asyncness.is_some().then(|| quote!(.await));
        let name = &function.sig.ident;
        quote! {
            #[allow(deprecated, reason = "an adapter must remain callable for a deprecated task")]
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
    }
}

fn input_names(inputs: &[Type]) -> Vec<syn::Ident> {
    (0..inputs.len())
        .map(|i| format_ident!("__input_{i}"))
        .collect()
}

fn take_sender(inputs: &mut Vec<Type>, function: &ItemFn) -> syn::Result<Type> {
    let missing = |span| syn::Error::new(span, "stream producer requires a final StreamSender<T>");
    let ty = inputs.pop().ok_or_else(|| missing(function.sig.span()))?;
    let args = signature::arguments(&ty, "StreamSender")
        .filter(|args| args.len() == 1)
        .ok_or_else(|| missing(ty.span()))?;
    signature::owned(&args[0])?;
    Ok(args[0].clone())
}
