//! Derives re-exported by `fusor` with its `dom` feature.
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote, quote_spanned};
use syn::{
    Data, DeriveInput, Error, Expr, Fields, Meta, Path, Result, fold::Fold, parse_macro_input,
    parse_quote, spanned::Spanned,
};

/// Generate a component's named input struct and `FromInputs` implementation.
/// Each field must be `#[input]` or `#[local(init = expression)]`.
/// See `fusor::FromInputs` for the application-facing documentation.
#[proc_macro_derive(FromInputs, attributes(input, local, from_inputs))]
pub fn derive_from_inputs(input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as DeriveInput))
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

// Self in an input's type means the component, not the generated Inputs struct.
struct ComponentSelf(syn::Ident);
impl Fold for ComponentSelf {
    fn fold_path(&mut self, mut path: Path) -> Path {
        if path.leading_colon.is_none() {
            if let Some(first) = path
                .segments
                .first_mut()
                .filter(|part| part.ident == "Self")
            {
                first.ident = self.0.clone();
            }
        }
        syn::fold::fold_path(self, path)
    }
}

fn inputs_runtime(input: &DeriveInput) -> Result<Path> {
    let mut runtime: Path = parse_quote!(::fusor);
    let mut custom_crate = false;
    for attr in &input.attrs {
        if attr.path().is_ident("input") || attr.path().is_ident("local") {
            return Err(Error::new_spanned(
                attr,
                "place #[input] or #[local(init = ...)] on a field",
            ));
        }
        if attr.path().is_ident("from_inputs") {
            if custom_crate {
                return Err(Error::new_spanned(
                    attr,
                    "duplicate #[from_inputs] attribute",
                ));
            }
            attr.parse_nested_meta(|meta| {
                if !meta.path.is_ident("crate") {
                    return Err(meta.error("expected `crate = path`"));
                }
                if custom_crate {
                    return Err(meta.error("duplicate crate path"));
                }
                runtime = meta.value()?.parse()?;
                custom_crate = true;
                Ok(())
            })?;
            if !custom_crate {
                return Err(Error::new_spanned(
                    attr,
                    "expected #[from_inputs(crate = path)]",
                ));
            }
        }
    }
    Ok(runtime)
}

fn input_fields(input: &DeriveInput) -> Result<&Fields> {
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(Error::new_spanned(
            &input.generics,
            "FromInputs derive currently supports concrete structs; implement FromInputs manually for generic components",
        ));
    }
    let Data::Struct(data) = &input.data else {
        return Err(Error::new_spanned(
            &input.ident,
            "FromInputs can only be derived for a struct",
        ));
    };
    if matches!(data.fields, Fields::Unnamed(_)) {
        return Err(Error::new_spanned(
            &data.fields,
            "FromInputs requires named fields or a unit struct",
        ));
    }
    Ok(&data.fields)
}

struct InputFields {
    declarations: Vec<proc_macro2::TokenStream>,
    initializers: Vec<proc_macro2::TokenStream>,
}

fn lower_fields(
    fields: &Fields,
    name: &syn::Ident,
    inputs_var: &syn::Ident,
) -> Result<InputFields> {
    let mut inputs_fields = Vec::new();
    let mut values = Vec::new();
    let mut errors: Option<Error> = None;
    for field in fields {
        let field_name = field.ident.as_ref().expect("unnamed fields were rejected");
        let parsed = field_kind(field);
        match parsed {
            Ok(InputKind::Input) => {
                let ty = ComponentSelf(name.clone()).fold_type(field.ty.clone());
                let docs = field
                    .attrs
                    .iter()
                    .filter(|attr| attr.path().is_ident("doc"));
                inputs_fields.push(quote_spanned!(field.span()=> #(#docs)* pub #field_name: #ty));
                values.push(quote_spanned!(field.span()=> #field_name: #inputs_var.#field_name));
            }
            Ok(InputKind::Local(init)) => {
                values.push(quote_spanned!(field.span()=> #field_name: { #init }))
            }
            Err(error) => match &mut errors {
                Some(errors) => errors.combine(error),
                None => errors = Some(error),
            },
        }
    }
    if let Some(errors) = errors {
        return Err(errors);
    }
    Ok(InputFields {
        declarations: inputs_fields,
        initializers: values,
    })
}

enum InputKind {
    Input,
    Local(Box<Expr>),
}

fn field_kind(field: &syn::Field) -> Result<InputKind> {
    let mut kind = None;
    for attr in &field.attrs {
        if attr.path().is_ident("from_inputs") {
            return Err(Error::new_spanned(
                attr,
                "#[from_inputs(crate = path)] belongs on the struct",
            ));
        }
        if !attr.path().is_ident("input") && !attr.path().is_ident("local") {
            continue;
        }
        if kind.is_some() {
            return Err(Error::new_spanned(
                attr,
                "choose exactly one #[input] or #[local(init = ...)] per field",
            ));
        }
        kind = Some(if attr.path().is_ident("input") {
            if !matches!(attr.meta, Meta::Path(_)) {
                return Err(Error::new_spanned(
                    attr,
                    "#[input] takes no arguments; inputs are required and keep their exact Rust type",
                ));
            }
            InputKind::Input
        } else {
            InputKind::Local(Box::new(local_initializer(attr)?))
        });
    }
    kind.ok_or_else(|| {
        Error::new_spanned(
            &field.ident,
            "field needs #[input] or #[local(init = expression)]; no value is inferred",
        )
    })
}

fn local_initializer(attr: &syn::Attribute) -> Result<Expr> {
    let mut init = None;
    attr.parse_nested_meta(|meta| {
        if !meta.path.is_ident("init") {
            return Err(meta.error("expected `init = expression`"));
        }
        if init.is_some() {
            return Err(meta.error("duplicate local initializer"));
        }
        init = Some(meta.value()?.parse::<Expr>()?);
        Ok(())
    })?;
    init.ok_or_else(|| Error::new_spanned(attr, "local state requires #[local(init = expression)]"))
}

fn expand(input: DeriveInput) -> Result<proc_macro2::TokenStream> {
    let runtime = inputs_runtime(&input)?;
    let fields = input_fields(&input)?;
    let name = &input.ident;
    let visibility = &input.vis;
    let inputs_name = format_ident!(
        "{}Inputs",
        name.to_string().trim_start_matches("r#"),
        span = name.span()
    );
    let inputs_var = syn::Ident::new("__fusor_inputs", Span::mixed_site());
    let owner_var = syn::Ident::new("__fusor_owner", Span::mixed_site());
    let InputFields {
        declarations: inputs_fields,
        initializers: values,
    } = lower_fields(fields, name, &inputs_var)?;
    let doc = format!("Parent-supplied inputs generated by `FromInputs` for `{name}`.");
    let declaration = if inputs_fields.is_empty() {
        quote!(#[doc = #doc] #visibility struct #inputs_name;)
    } else {
        quote!(#[doc = #doc] #visibility struct #inputs_name { #(#inputs_fields,)* })
    };
    let construct = if matches!(fields, Fields::Unit) {
        quote!(Self)
    } else {
        quote!(Self { #(#values,)* })
    };
    Ok(quote! {
        #declaration
        impl #runtime::FromInputs for #name {
            type Inputs = #inputs_name;
            type Error = ::core::convert::Infallible;
            fn from_inputs(
                #inputs_var: Self::Inputs,
                #owner_var: #runtime::OwnerHandle,
            ) -> ::core::result::Result<Self, Self::Error> {
                ::core::result::Result::Ok(#construct)
            }
        }
    })
}

#[cfg(test)]
mod tests;

/// Expose only marked `Signal<T>` fields to this component's JavaScript module.
#[proc_macro_derive(JsInputs, attributes(js, js_inputs))]
pub fn derive_js_inputs(input: TokenStream) -> TokenStream {
    expand_js_inputs(parse_macro_input!(input as DeriveInput))
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

fn supported_js_value(ty: &syn::Type) -> bool {
    let syn::Type::Path(path) = ty else {
        return false;
    };
    let Some(segment) = path.path.segments.last() else {
        return false;
    };
    match segment.ident.to_string().as_str() {
        "bool" | "String" | "f64" | "i32" | "u32" | "JsValue" => {
            matches!(segment.arguments, syn::PathArguments::None)
        }
        "Option" | "Vec" => {
            let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
                return false;
            };
            args.args.len() == 1
                && matches!(args.args.first(), Some(syn::GenericArgument::Type(inner)) if supported_js_value(inner))
        }
        _ => false,
    }
}

fn expand_js_inputs(input: DeriveInput) -> Result<proc_macro2::TokenStream> {
    let mut runtime: Path = parse_quote!(::fusor);
    for attr in &input.attrs {
        if attr.path().is_ident("js_inputs") {
            attr.parse_nested_meta(|meta| {
                if !meta.path.is_ident("crate") {
                    return Err(meta.error("expected `crate = path`"));
                }
                runtime = meta.value()?.parse()?;
                Ok(())
            })?;
        }
        if attr.path().is_ident("js") {
            return Err(Error::new_spanned(
                attr,
                "place #[js] on an exposed Signal field",
            ));
        }
    }
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(Error::new_spanned(
            &input.generics,
            "JsInputs derive supports concrete structs",
        ));
    }
    let Data::Struct(data) = input.data else {
        return Err(Error::new_spanned(
            input.ident,
            "JsInputs requires a struct",
        ));
    };
    if matches!(data.fields, Fields::Unnamed(_)) {
        return Err(Error::new_spanned(
            data.fields,
            "JsInputs requires named fields or a unit struct",
        ));
    }
    let name = input.ident;
    let fields = data
        .fields
        .iter()
        .filter_map(|field| expose_js_field(field).transpose())
        .collect::<Result<Vec<_>>>()?;
    Ok(quote! {
        impl #runtime::js::JsInputs for #name {
            fn js_inputs(&self) -> #runtime::js::Inputs {
                let mut inputs = #runtime::js::Inputs::default();
                #(#fields)*
                inputs
            }
        }
    })
}

fn expose_js_field(field: &syn::Field) -> Result<Option<proc_macro2::TokenStream>> {
    let attributes: Vec<_> = field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("js"))
        .collect();
    if attributes.is_empty() {
        return Ok(None);
    }
    if attributes.len() != 1 || !matches!(attributes[0].meta, Meta::Path(_)) {
        return Err(Error::new_spanned(
            attributes[0],
            "use a single #[js] marker without arguments",
        ));
    }
    let supported = if let syn::Type::Path(path) = &field.ty {
        path.path.segments.last().is_some_and(|segment| {
                if segment.ident != "Signal" { return false; }
                let syn::PathArguments::AngleBracketed(args) = &segment.arguments else { return false; };
                args.args.len() == 1 && matches!(args.args.first(), Some(syn::GenericArgument::Type(inner)) if supported_js_value(inner))
            })
    } else {
        false
    };
    if !supported {
        return Err(Error::new_spanned(
            &field.ty,
            "#[js] requires Signal<T>, where T is bool, String, f64, i32, u32, JsValue, or Option/Vec of these; arbitrary structs and 64-bit integers are unsupported",
        ));
    }
    let field_name = field.ident.as_ref().expect("unnamed fields were rejected");
    let exposed = field_name.to_string().trim_start_matches("r#").to_owned();
    Ok(Some(
        quote_spanned! {field.span()=> inputs.add(#exposed, self.#field_name.clone()); },
    ))
}
