//! Validation for structural route declarations. Uses the runtime matcher grammar.
use super::{ir::RouteBranch, tag_input::TagInput, tokens::Rust};
use crate::{ExtractError, error};
use fusor_router::pattern::{Pattern, ambiguous};

pub(super) struct Declaration {
    pub path: Option<String>,
    pub alias: Option<Rust>,
    pub names: Vec<Rust>,
}

pub(super) fn route(input: &TagInput) -> Result<Declaration, ExtractError> {
    input.accepts(
        &["path", "let", "fallback"],
        "path and optional let, or fallback",
    )?;
    if input.has("fallback") {
        if input.has("path") || input.has("let") || !input.text("fallback").unwrap().0.is_empty() {
            return Err(input.error("write <Route fallback> without path or let"));
        }
        return Ok(Declaration {
            path: None,
            alias: None,
            names: Vec::new(),
        });
    }
    let (path, offset) = input
        .text("path")
        .ok_or_else(|| input.error("Route requires a path or fallback"))?;
    let pattern = Pattern::new(&path).map_err(|e| input.error_at(offset, e.to_string()))?;
    let alias = input.binding("let")?;
    let names = pattern
        .names()
        .map(|name| input.field("path parameter", name, offset))
        .collect::<Result<_, _>>()?;
    Ok(Declaration {
        path: Some(path),
        alias,
        names,
    })
}

pub(super) fn validate(
    source: &str,
    offset: usize,
    routes: &[RouteBranch],
    path: Option<&str>,
) -> Result<(), ExtractError> {
    // Declared paths were validated when their tags were parsed.
    let pattern = |path: &str| Pattern::new(path).expect("validated route path");
    let this = path.map(pattern);
    for route in routes {
        if ambiguous(this.as_ref(), route.path.as_deref().map(pattern).as_ref()) {
            return Err(error(
                source,
                offset,
                "Router contains ambiguous route patterns or multiple fallbacks",
            ));
        }
    }
    Ok(())
}
