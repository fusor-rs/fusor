//! Must work on a host with a published site and nothing else, so the base path
//! and history fallback come from the output manifest, not from Cargo.
use crate::{
    context::Context,
    dev::{http, server},
    error::{Error, Result},
    pipeline::{
        manifest::OutputManifest,
        site::{SiteManifest, describe, route, shown},
    },
    workspace::select,
};
use std::path::{Path, PathBuf};

const READY: &str = "Ready. Press Ctrl+C to stop.";

pub(crate) fn run(cx: &Context, directory: Option<&Path>, port: u16, open: bool) -> Result {
    let directory = match directory {
        Some(directory) => directory.to_owned(),
        None => configured_output(cx)?,
    };
    if let Some(site) = SiteManifest::read(&directory)? {
        return serve_site(cx, &directory, &site, port, open);
    }
    let output = OutputManifest::read(&directory)?;
    server::check_port(port)?;
    let listener = server::bind(port)?;
    cx.reporter
        .banner(format!("preview of {}", shown(&directory)));
    cx.reporter
        .field("Local", server::url(port, &output.base_path));
    cx.reporter.blank();
    server::ready(cx, port, &output.base_path, open, READY)?;
    server::serve(cx, listener, move |method, url, accept| {
        http::respond(
            method,
            url,
            accept,
            http::Route {
                directory: &directory,
                base: &output.base_path,
                history_fallback: &output.history_fallback,
            },
            http::Serving::Preview,
        )
    })
}

/// Each request goes to the application with the longest matching base path,
/// which answers with its own history fallback.
fn serve_site(cx: &Context, root: &Path, site: &SiteManifest, port: u16, open: bool) -> Result {
    let mounts: Vec<(String, (PathBuf, OutputManifest))> = site
        .outputs(root)?
        .into_iter()
        .map(|(directory, output)| (output.base_path.clone(), (directory, output)))
        .collect();
    server::check_port(port)?;
    let listener = server::bind(port)?;
    let base = mounts
        .iter()
        .map(|(base, _)| base.as_str())
        .min_by_key(|base| base.len())
        .unwrap_or("/");
    cx.reporter.banner(format!("preview of {}", shown(root)));
    cx.reporter.field("Local", server::url(port, base));
    cx.reporter.field(
        "Apps",
        describe(
            root,
            site.mounts
                .iter()
                .map(|mount| (mount.path.as_str(), mount.application.as_str())),
        ),
    );
    cx.reporter.blank();
    server::ready(cx, port, base, open, READY)?;
    server::serve(cx, listener, move |method, url, accept| {
        match route(&mounts, url) {
            Some((directory, output)) => http::respond(
                method,
                url,
                accept,
                http::Route {
                    directory,
                    base: &output.base_path,
                    history_fallback: &output.history_fallback,
                },
                http::Serving::Preview,
            ),
            None => http::build_response(404, "text/plain", b"Not found".to_vec(), false),
        }
    })
}

/// A host with no Cargo.toml at all uses the default application output.
fn configured_output(cx: &Context) -> Result<PathBuf> {
    let Some(candidate) = select::from_filesystem(cx)? else {
        return Ok(fusor_build::app::AppConfig::default().output);
    };
    let config = fusor_build::app::AppConfig::load(&candidate.manifest).map_err(|error| {
        Error::project(format!(
            "cannot read {}: {error}",
            candidate.manifest.display()
        ))
        .remedy("pass the output directory explicitly, as in `fusor preview dist`")
    })?;
    Ok(candidate
        .manifest
        .parent()
        .ok_or_else(|| Error::internal("manifest has no parent directory"))?
        .join(config.output))
}
