use crate::{
    context::Context,
    error::{Error, Result},
    workspace::Project,
};
use std::path::Path;

pub(crate) fn run(cx: &Context, project: &Project, module: &str) -> Result {
    let out = project.target.join("fusor/expand").join(&project.name);
    let artifact = fusor_build::app::generate(&project.manifest, &out)?;
    let source = artifact
        .sources
        .iter()
        .find(|source| matches(source, project, module))
        .ok_or_else(|| {
            let registered = artifact
                .sources
                .iter()
                .map(|source| display_name(&source.name))
                .collect::<Vec<_>>()
                .join(", ");
            Error::usage(format!("no HTML module {module:?}"))
                .remedy(format!("registered modules: {registered}"))
        })?;
    cx.reporter
        .result(std::fs::read_to_string(&source.rust)?.trim_end())?;
    Ok(())
}

/// By registered name, or by path relative to the application root, as in
/// `expand --module web/app.html`.
fn matches(source: &fusor_build::app::SourceArtifact, project: &Project, module: &str) -> bool {
    display_name(&source.name) == module
        || source.name == module
        || source
            .source
            .strip_prefix(&project.root)
            .is_ok_and(|path| path == Path::new(module))
}

/// Discovered templates carry an `@` prefix internally; users do not type it.
fn display_name(name: &str) -> &str {
    name.strip_prefix('@').unwrap_or(name)
}
