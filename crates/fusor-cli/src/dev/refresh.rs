//! An edit that leaves the Rust the browser runs unchanged is patched in place,
//! keeping page state. When unclear, the answer is no and a normal build runs.
use crate::{
    context::Context,
    error::{Error, Result},
    layout,
    pipeline::{Publication, html, manifest::OutputManifest},
    workspace::Project,
};
use fusor_build::app::{self, ArtifactManifest};
use proc_macro2::{TokenStream, TokenTree};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub(crate) const CLIENT: &str = include_str!("refresh.js");

/// Checked once after a full build, not on the fast path. A file include is
/// invisible to this comparison; `fusor-build` decides which includes are its
/// own.
pub(crate) fn enabled(
    cx: &Context,
    project: &Project,
    artifact: &ArtifactManifest,
) -> Result<bool> {
    if !project.config.dev_refresh || !artifact.javascript.is_empty() {
        return Ok(false);
    }
    let watched = super::sources::snapshot(cx, project)?;
    let rust_files = watched
        .keys()
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .chain(artifact.sources.iter().map(|source| &source.rust));
    for path in rust_files {
        let source = fs::read_to_string(path)?;
        // A source that does not tokenize cannot be cleared.
        match app::includes_foreign_file(&source) {
            Some(false) => {}
            _ => return Ok(false),
        }
    }
    Ok(true)
}

/// `false` for any edit that needs a real build.
pub(crate) fn try_refresh(cx: &Context, project: &Project, changed: &[PathBuf]) -> Result<bool> {
    let started = std::time::Instant::now();
    if project.config.delivery.is_some() {
        return Ok(false);
    }
    let html: Vec<_> = project
        .config
        .discover_sources(&project.root)?
        .into_iter()
        .map(|source| project.root.join(source.path))
        .collect();
    let assets = project
        .config
        .assets
        .as_ref()
        .map(|path| project.root.join(path));
    let only_html_and_assets = changed.iter().all(|path| {
        html.contains(path)
            || assets
                .as_ref()
                .is_some_and(|assets| path.starts_with(assets))
    });
    if !only_html_and_assets {
        cx.reporter
            .note("a changed file is neither authored HTML nor an asset; rebuilding");
        return Ok(false);
    }

    let output = OutputManifest::read(&project.output(cx))?;
    let Some(previous) = output.rust_signature.as_ref() else {
        return Ok(false);
    };
    // Equal token trees and locations mean the compiled Wasm still matches.
    let work = project.target.join("fusor/refresh").join(&project.name);
    let artifact = app::generate(&project.manifest, &work)?;
    let Ok(next) = signature(&artifact) else {
        return Ok(false);
    };
    if &next != previous {
        cx.reporter
            .note("the Rust generated from this HTML changed; rebuilding");
        return Ok(false);
    }

    publish_revision(cx, project, &artifact, output, &html, changed, started)?;
    Ok(true)
}

fn publish_revision(
    cx: &Context,
    project: &Project,
    artifact: &ArtifactManifest,
    mut output: OutputManifest,
    html: &[PathBuf],
    changed: &[PathBuf],
    started: std::time::Instant,
) -> Result {
    let site = project.output(cx);
    let publication = Publication::revise(cx, project, output.generation.clone())?;
    let revision = output
        .revision()
        .checked_add(1)
        .ok_or_else(|| Error::internal("the refresh revision counter overflowed"))?;
    // Only CSS and authored HTML are patched live. Anything else needs a
    // reload, and the barrier catches clients that missed an update.
    if changed
        .iter()
        .any(|path| !html.contains(path) && path.extension().is_none_or(|ext| ext != "css"))
    {
        output.reload_after = Some(revision);
    }
    output.revision = Some(revision);

    if non_css_assets(&site)? != non_css_assets(publication.staging())? {
        output.reload_after = Some(revision);
    }
    fs::write(
        publication.staging().join("index.html"),
        html::render(artifact, &publication.url_prefix(), Some(revision), &[])?,
    )?;
    publication.commit(&output)?;
    let effect = if output.reload_after == Some(revision) {
        "page reloads"
    } else {
        "page kept its state"
    };
    cx.reporter.done(format!(
        "Refreshed {} in {}, {effect}",
        project.name,
        crate::reporter::elapsed_text(started.elapsed())
    ));
    Ok(())
}

/// Locations matter as much as tokens: `line!()` changes behavior without
/// changing a token.
pub(crate) fn signature(artifact: &ArtifactManifest) -> Result<Value> {
    let mut sources = BTreeMap::new();
    for source in &artifact.sources {
        let rust = fs::read_to_string(&source.fingerprint)?;
        let stream: TokenStream = rust
            .parse()
            .map_err(|error| Error::internal(format!("invalid generated Rust: {error}")))?;
        let mut signature = Vec::new();
        tokens(stream, &mut signature);
        // Registration is executable Rust too: an HTML-only change to `src` or
        // `rust:module` must never reuse the previous Wasm.
        signature.push(json!(source.external));
        if let Some(registration) = &source.registration {
            let rust = fs::read_to_string(&registration.rust)?;
            let stream = rust.parse().map_err(|error| {
                Error::internal(format!("invalid generated registration: {error}"))
            })?;
            tokens(stream, &mut signature);
        }
        sources.insert(source.name.clone(), signature);
    }
    Ok(serde_json::to_value(sources)?)
}

fn tokens(stream: TokenStream, output: &mut Vec<Value>) {
    for token in stream {
        let span = token.span();
        let (start, end) = (span.start(), span.end());
        match token {
            TokenTree::Group(group) => {
                output.push(json!([
                    format!("{:?}", group.delimiter()),
                    start.line,
                    start.column,
                    end.line,
                    end.column
                ]));
                tokens(group.stream(), output);
                output.push(json!(["end"]));
            }
            token => output.push(json!([
                token.to_string(),
                start.line,
                start.column,
                end.line,
                end.column
            ])),
        }
    }
}

/// A change here means the page must reload.
fn non_css_assets(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    fn collect(root: &Path, directory: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path.strip_prefix(root)?;
            if relative.starts_with(layout::GENERATED)
                || relative == Path::new("index.html")
                || relative == Path::new(layout::OUTPUT_MANIFEST)
            {
                continue;
            }
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(Error::project(
                    "published output contains a symlink; refresh cannot compare it",
                ));
            }
            if kind.is_dir() {
                collect(root, &path, out)?;
            } else if path.extension().is_none_or(|extension| extension != "css") {
                out.insert(relative.to_owned(), fs::read(path)?);
            }
        }
        Ok(())
    }
    let mut assets = BTreeMap::new();
    collect(root, root, &mut assets)?;
    Ok(assets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(source: &str) -> Vec<Value> {
        let mut out = Vec::new();
        tokens(source.parse().unwrap(), &mut out);
        out
    }

    #[test]
    fn the_signature_preserves_rust_token_and_location_semantics() {
        assert_eq!(fingerprint("fn f() {}      "), fingerprint("fn f() {} "));
        assert_ne!(
            fingerprint("fn f() { line!(); }"),
            fingerprint("\nfn f() { line!(); }")
        );
        assert_ne!(
            fingerprint("fn f() { 'a: loop { break 'a } }"),
            fingerprint("fn f() { 'b: loop { break 'b } }")
        );
        assert_ne!(
            fingerprint("fn f() { r#\"a b\"#; }"),
            fingerprint("fn f() { r#\"ab\"#; }")
        );
    }

    #[test]
    fn bind_changes_participate_in_native_refresh_compatibility() {
        let html = r#"<script type="text/rust">struct Editor;</script>
<main rust:component="Editor"><input bind="state.title" placeholder="First"></main>"#;
        let source = |html: &str| fusor_build::extract(html).unwrap().fingerprint;
        let before = fingerprint(&source(html));
        assert_eq!(
            before,
            fingerprint(&source(&html.replace("First", "Other")))
        );
        assert_ne!(
            before,
            fingerprint(&source(&html.replace("state.title", "state.other")))
        );
    }
}
