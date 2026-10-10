use crate::bindings::ir::{Component, Edit};
use crate::{ExtractError, JavaScriptModule, error};
use html5gum::StartTag;
use sha2::{Digest, Sha256};
use std::ops::Range;

pub(super) struct PendingModule {
    start: usize,
    body: usize,
    module: JavaScriptModule,
}

impl PendingModule {
    pub(super) fn begin(
        source: &str,
        tag: &StartTag<usize>,
        component: &Component,
    ) -> Result<Self, ExtractError> {
        if component.javascript.is_some() {
            return Err(error(
                source,
                tag.span.start,
                "a component supports one module script; compose modules with ordinary imports",
            ));
        }
        if tag.self_closing
            || tag
                .attributes
                .keys()
                .any(|key| !matches!(key.as_ref(), b"type" | b"src"))
        {
            return Err(error(
                source,
                tag.span.start,
                "component modules accept type=module and optional src, with an explicit closing script tag",
            ));
        }
        let src = tag
            .attributes
            .get(b"src".as_slice())
            .map(|src| String::from_utf8_lossy(src).into_owned());
        if src.as_ref().is_some_and(|src| {
            src.is_empty() || src.starts_with('/') || src.contains([':', '?', '#', '\\'])
        }) {
            return Err(error(
                source,
                tag.span.start,
                "component module src must be a relative .js, .mjs, or .ts file path",
            ));
        }
        let digest = format!("{:x}", Sha256::digest(source.as_bytes()));
        let (line, column) = crate::location(source, tag.span.end);
        let component_name = component_name(component);
        let module = JavaScriptModule {
            id: format!(
                "{component_name}:{}:{}",
                &digest[..16],
                component.id.index()
            ),
            component: component_name,
            src,
            content: String::new(),
            line,
            column,
        };
        Ok(Self {
            start: tag.span.start,
            body: tag.span.end,
            module,
        })
    }

    pub(super) fn finish(
        self,
        source: &str,
        closing: Range<usize>,
    ) -> Result<(JavaScriptModule, Edit), ExtractError> {
        let Self {
            start,
            body,
            mut module,
        } = self;
        module.content = source[body..closing.start].to_owned();
        if module.src.is_some() && !module.content.trim().is_empty() {
            return Err(error(
                source,
                body,
                "a component module with src must have an empty body",
            ));
        }
        let edit = Edit {
            range: start..closing.end,
            replacement: String::new(),
        };
        Ok((module, edit))
    }
}

fn component_name(component: &Component) -> String {
    component.app().map_or_else(
        || component.ty.tokens.to_string(),
        |app| {
            match syn::parse2::<syn::Expr>(app.tokens.clone()) {
                Ok(expression) => app_name(&expression).unwrap_or_else(|| "App".into()),
                // Rustc diagnoses malformed factories; App is only the editor alias.
                Err(_) => "App".into(),
            }
        },
    )
}

fn app_name(expression: &syn::Expr) -> Option<String> {
    match expression {
        syn::Expr::Call(call) => {
            let syn::Expr::Path(path) = &*call.func else {
                return None;
            };
            path.path
                .segments
                .iter()
                .rev()
                .nth(1)
                .map(|segment| segment.ident.to_string())
        }
        syn::Expr::Struct(value) => value
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        syn::Expr::Path(value) => value
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        syn::Expr::Paren(value) => app_name(&value.expr),
        syn::Expr::Block(value) => value
            .block
            .stmts
            .last()
            .and_then(|statement| match statement {
                syn::Stmt::Expr(value, _) => app_name(value),
                _ => None,
            }),
        _ => None,
    }
}
