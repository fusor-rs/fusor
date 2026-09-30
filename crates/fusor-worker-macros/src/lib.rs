//! Parse authored signatures once; every generated adapter uses the same types.
mod signature;
mod task;
mod worker;
use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn task(attributes: TokenStream, item: TokenStream) -> TokenStream {
    task::expand(attributes.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
#[proc_macro_attribute]
pub fn worker(attributes: TokenStream, item: TokenStream) -> TokenStream {
    worker::expand(attributes.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[doc(hidden)]
#[proc_macro_attribute]
pub fn __worker_method(attributes: TokenStream, item: TokenStream) -> TokenStream {
    worker::expand_method(attributes.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
