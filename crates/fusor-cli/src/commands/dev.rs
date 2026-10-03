use crate::{
    context::Context,
    dev::server::{self, DevApp},
    error::Result,
    layout,
    pipeline::{
        manifest::OutputManifest,
        site::{self, SiteConfig},
    },
    toolchain,
    workspace::{self, Project},
};
use std::time::Instant;

pub(crate) fn run(cx: &Context, project: Project, port: u16, open: bool) -> Result {
    let started = Instant::now();
    // Finding out the port is busy after a two-minute compile is a bad trade.
    server::check_port(port)?;
    cx.reporter.banner("development");
    cx.reporter
        .field("Local", server::url(port, &project.config.base_path));
    cx.reporter.blank();
    let app = prepare(cx, project)?;
    server::dev(cx, vec![app], port, open, started)
}

/// Every application the site mounts, each with its own watcher, served from
/// one origin the way `build --site` lays them out.
pub(crate) fn site(cx: &Context, port: u16, open: bool) -> Result {
    let started = Instant::now();
    server::check_port(port)?;
    let metadata = workspace::read_metadata(cx, cx.manifest_path.as_deref())?;
    let config = SiteConfig::from_metadata(&metadata.metadata)?;
    let root = config
        .mounts
        .keys()
        .min_by_key(|path| path.len())
        .map_or("/", String::as_str);
    cx.reporter.banner("development");
    cx.reporter.field("Local", server::url(port, root));
    cx.reporter.field("Apps", site::apps(&config.mounts));
    cx.reporter.blank();
    let manifest = metadata.workspace_root.join("Cargo.toml");
    let mut apps = Vec::new();
    for (path, name) in &config.mounts {
        let app = cx.for_manifest(manifest.clone(), Some(name.clone()));
        let project = Project::discover(&app)?;
        site::check_mount(path, &project)?;
        apps.push(prepare(&app, project)?);
    }
    server::dev(cx, apps, port, open, started)
}

/// Prepare tools and make the first development build into `.fusor/dev`.
fn prepare(cx: &Context, project: Project) -> Result<DevApp> {
    toolchain::prepare(cx, &project, false, false)?;
    let mut cx = cx.clone();
    cx.output = Some(project.root.join(layout::DEV_OUTPUT));
    super::build::run(&cx, &project, crate::pipeline::BuildMode::Development)
        .map_err(|error| error.context(&project.name))?;
    // Said once, here, rather than on every rebuild.
    let output = OutputManifest::read(&project.output(&cx))?;
    if output.rust_signature.is_none() && project.config.delivery.is_none() {
        cx.reporter.warn(format!(
            "Fast refresh is off for {}: every edit rebuilds. Either dev-refresh is false, or a source includes a file.",
            project.name
        ));
    }
    Ok(DevApp { cx, project })
}
