//! The builder through which generated template code supplies component inputs.
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote, quote_spanned};
use syn::{
    Attribute, DeriveInput, Error, Expr, GenericParam, Generics, Ident, Meta, Path, Result, Type,
    Visibility, fold::Fold, parse_quote, parse_quote_spanned, spanned::Spanned,
};

/// The method that finishes a builder; no input may share its name.
const BUILD: &str = "build";

pub(crate) struct InputField {
    pub name: Ident,
    pub ty: Type,
    pub vis: Visibility,
    /// `None` when the parent must supply the input.
    pub default: Option<Expr>,
}

/// A struct that templates construct through `Inputs::builder`.
pub(crate) struct InputsStruct<'a> {
    pub name: &'a Ident,
    pub vis: &'a Visibility,
    pub generics: &'a Generics,
    pub fields: Vec<InputField>,
}

/// The spellings of `Option` the derives recognize; an alias of it stays required.
const OPTION_PATHS: [&[&str]; 3] = [
    &["Option"],
    &["std", "option", "Option"],
    &["core", "option", "Option"],
];

enum Marker {
    Required,
    Default(Box<Expr>),
}

/// The default of an input the parent may omit, or `None` when it is required.
/// An `Option` input defaults to `None` unless marked `#[input(required)]`;
/// `#[input(default)]` and `#[input(default = expression)]` give any input a
/// default. `Self` in an expression names `owner`, not the builder evaluating it.
pub(crate) fn default_value(
    attr: Option<&Attribute>,
    ty: &Type,
    owner: &Ident,
) -> Result<Option<Expr>> {
    let Some(attr) = attr.filter(|attr| !matches!(attr.meta, Meta::Path(_))) else {
        return Ok(
            is_option(ty).then(|| parse_quote_spanned!(ty.span()=> ::core::option::Option::None))
        );
    };
    let mut marker = None;
    attr.parse_nested_meta(|meta| {
        if !meta.path.is_ident("required") && !meta.path.is_ident("default") {
            return Err(meta.error("expected `default`, `default = expression` or `required`"));
        }
        if marker.is_some() {
            return Err(meta.error("choose one of `default`, `default = expression` or `required`"));
        }
        marker = Some(if meta.path.is_ident("required") {
            Marker::Required
        } else {
            let default = if meta.input.peek(syn::Token![=]) {
                crate::ComponentSelf(owner.clone()).fold_expr(meta.value()?.parse()?)
            } else {
                parse_quote_spanned!(meta.path.span()=> ::core::default::Default::default())
            };
            Marker::Default(Box::new(default))
        });
        Ok(())
    })?;
    match marker {
        Some(Marker::Required) => Ok(None),
        Some(Marker::Default(default)) => Ok(Some(*default)),
        None => Err(Error::new_spanned(
            attr,
            "expected #[input(default)], #[input(default = expression)] or #[input(required)]",
        )),
    }
}

fn is_option(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    let segments: Vec<String> = path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    path.qself.is_none()
        && OPTION_PATHS
            .iter()
            .any(|option| segments.iter().eq(option.iter()))
}

/// `#[derive(Inputs)]` on a hand-written inputs struct: every field is an input.
pub(crate) fn expand(input: DeriveInput) -> Result<TokenStream> {
    let runtime = crate::runtime_path(&input, "inputs", &["input"])?;
    let mut fields = Vec::new();
    for field in crate::named_fields(&input, "Inputs")? {
        if let Some(attr) = field
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("inputs"))
        {
            return Err(Error::new_spanned(
                attr,
                "#[inputs(crate = path)] belongs on the struct",
            ));
        }
        let mut markers = field
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("input"));
        let default = match (markers.next(), markers.next()) {
            (_, Some(extra)) => {
                return Err(Error::new_spanned(extra, "use one #[input(...)] per field"));
            }
            (attr, None) => default_value(attr, &field.ty, &input.ident)?,
        };
        fields.push(InputField {
            name: field.ident.clone().expect("unnamed fields were rejected"),
            ty: field.ty.clone(),
            vis: field.vis.clone(),
            default,
        });
    }
    let target = InputsStruct {
        name: &input.ident,
        vis: &input.vis,
        generics: &input.generics,
        fields,
    };
    builder(&target, &runtime)
}

/// Implement `Inputs` with a typestate builder. Each required input is a type
/// parameter that its setter changes from unset to set; `build` requires every
/// one through its own trait, so a missing input is an error that names it.
pub(crate) fn builder(target: &InputsStruct<'_>, runtime: &Path) -> Result<TokenStream> {
    if let Some(field) = target.fields.iter().find(|field| field.name == BUILD) {
        return Err(Error::new_spanned(
            &field.name,
            "`build` finishes the generated input builder; rename this input",
        ));
    }
    let builder = Builder::new(target);
    let name = target.name;
    let vis = target.vis;
    let ty = &builder.ty;
    let typestate = builder.typestate();
    let required_names = builder.required_names();
    let optional_names = builder.optional_names();
    let states = &builder.states;
    let optional_types = builder.optional.iter().map(|field| &field.ty);
    let initial = builder.arguments(builder.states.iter().map(|_| quote!(__FusorUnset)));
    let (impl_generics, type_generics, where_clause) = target.generics.split_for_impl();
    let builder_generics = &builder.generics;
    let (builder_impl, builder_type, builder_where) = builder_generics.split_for_impl();
    let setters = builder.setters();
    let build = builder.build();
    Ok(quote! {
        #[allow(clippy::type_complexity, reason = "builder types spell out every input's state")]
        const _: () = {
            #typestate
            #vis struct #ty #builder_generics #builder_where {
                #(#required_names: #states,)*
                #(#optional_names: ::core::option::Option<#optional_types>,)*
                __fusor_inputs: ::core::marker::PhantomData<fn() -> #name #type_generics>,
            }
            impl #impl_generics #runtime::Inputs for #name #type_generics #where_clause {
                type Builder = #ty<#(#initial),*>;
                fn builder() -> Self::Builder {
                    #ty {
                        #(#required_names: __FusorUnset,)*
                        #(#optional_names: ::core::option::Option::None,)*
                        __fusor_inputs: ::core::marker::PhantomData,
                    }
                }
            }
            #[allow(dead_code, reason = "templates call only the setters of supplied inputs")]
            impl #builder_impl #ty #builder_type #builder_where {
                #setters
                #build
            }
        };
    })
}

struct Builder<'a> {
    target: &'a InputsStruct<'a>,
    ty: Ident,
    required: Vec<&'a InputField>,
    optional: Vec<&'a InputField>,
    /// One type parameter per required input, in declaration order.
    states: Vec<Ident>,
    /// The inputs struct's own generics followed by `states`.
    generics: Generics,
    /// The arguments naming the inputs struct's own generic parameters.
    struct_arguments: Vec<TokenStream>,
}

impl<'a> Builder<'a> {
    fn new(target: &'a InputsStruct<'a>) -> Self {
        let (required, optional): (Vec<_>, Vec<_>) = target
            .fields
            .iter()
            .partition(|field| field.default.is_none());
        let states: Vec<Ident> = (0..required.len())
            .map(|index| format_ident!("__FusorInput{index}"))
            .collect();
        let mut generics = target.generics.clone();
        for param in &mut generics.params {
            match param {
                GenericParam::Type(param) => param.default = None,
                GenericParam::Const(param) => param.default = None,
                GenericParam::Lifetime(_) => {}
            }
        }
        generics.params.extend(
            states
                .iter()
                .map(|state| -> GenericParam { parse_quote!(#state) }),
        );
        let struct_arguments = target
            .generics
            .params
            .iter()
            .map(|param| match param {
                GenericParam::Lifetime(param) => param.lifetime.to_token_stream(),
                GenericParam::Type(param) => param.ident.to_token_stream(),
                GenericParam::Const(param) => param.ident.to_token_stream(),
            })
            .collect();
        Self {
            target,
            ty: format_ident!(
                "{}Builder",
                target.name.to_string().trim_start_matches("r#"),
                span = target.name.span()
            ),
            required,
            optional,
            states,
            generics,
            struct_arguments,
        }
    }

    fn required_names(&self) -> Vec<&'a Ident> {
        self.required.iter().map(|field| &field.name).collect()
    }

    fn optional_names(&self) -> Vec<&'a Ident> {
        self.optional.iter().map(|field| &field.name).collect()
    }

    fn requirement(index: usize) -> Ident {
        format_ident!("__FusorRequired{index}")
    }

    /// Builder type arguments: the struct's own, then one per required input.
    fn arguments(&self, states: impl IntoIterator<Item = TokenStream>) -> Vec<TokenStream> {
        self.struct_arguments
            .iter()
            .cloned()
            .chain(states)
            .collect()
    }

    /// The unset and set states, and one trait per required input whose
    /// diagnostic names that input when `build` is called without it.
    fn typestate(&self) -> Option<TokenStream> {
        if self.required.is_empty() {
            return None;
        }
        let requirements = self.required.iter().enumerate().map(|(index, field)| {
            let requirement = Self::requirement(index);
            let input = field.name.to_string();
            let input = input.trim_start_matches("r#");
            let message = format!("missing required input `{input}` of `{}`", self.target.name);
            let label = format!("add a `{input}` attribute to this component tag");
            quote! {
                #[diagnostic::on_unimplemented(
                    message = #message,
                    label = #label,
                    note = "make an input optional with #[input(default)] or #[input(default = expression)]"
                )]
                pub trait #requirement<Value> {
                    fn into_value(self) -> Value;
                }
                impl<Value> #requirement<Value> for __FusorSet<Value> {
                    fn into_value(self) -> Value {
                        self.0
                    }
                }
            }
        });
        Some(quote! {
            pub struct __FusorUnset;
            pub struct __FusorSet<Value>(Value);
            #(#requirements)*
        })
    }

    /// One setter per input, named after its field and with its visibility.
    fn setters(&self) -> TokenStream {
        let ty = &self.ty;
        let required_names = self.required_names();
        let optional_names = self.optional_names();
        let required = self.required.iter().enumerate().map(|(index, field)| {
            let InputField {
                name,
                ty: value,
                vis,
                ..
            } = field;
            let mut states: Vec<TokenStream> =
                self.states.iter().map(ToTokens::to_token_stream).collect();
            states[index] = quote!(__FusorSet<#value>);
            let set = self.arguments(states);
            let others = required_names.iter().filter(|other| **other != name);
            quote_spanned! {name.span()=>
                #vis fn #name(self, value: #value) -> #ty<#(#set),*> {
                    #ty {
                        #name: __FusorSet(value),
                        #(#others: self.#others,)*
                        #(#optional_names: self.#optional_names,)*
                        __fusor_inputs: ::core::marker::PhantomData,
                    }
                }
            }
        });
        let optional = self
            .optional
            .iter()
            .map(|InputField { name, ty, vis, .. }| {
                quote_spanned! {name.span()=>
                    #vis fn #name(mut self, value: #ty) -> Self {
                        self.#name = ::core::option::Option::Some(value);
                        self
                    }
                }
            });
        quote! { #(#required)* #(#optional)* }
    }

    /// Defaults run here, only for omitted inputs, in declaration order.
    fn build(&self) -> TokenStream {
        let InputsStruct { name, vis, .. } = self.target;
        let (_, type_generics, _) = self.target.generics.split_for_impl();
        let states = &self.states;
        let requirements: Vec<Ident> = (0..states.len()).map(Self::requirement).collect();
        let required_names = self.required_names();
        let required_types = self.required.iter().map(|field| &field.ty);
        let optional_names = self.optional_names();
        let defaults = self.optional.iter().map(|field| &field.default);
        quote! {
            #vis fn build(self) -> #name #type_generics
            where
                #(#states: #requirements<#required_types>,)*
            {
                #name {
                    #(#required_names: <#states as #requirements<_>>::into_value(
                        self.#required_names,
                    ),)*
                    #(#optional_names: self.#optional_names.unwrap_or_else(|| #defaults),)*
                }
            }
        }
    }
}
