use crate::{
    context::Context,
    dev::{
        proxy::{Rule, Rules},
        server::{self, DevApp},
    },
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

pub(crate) struct Options {
    pub port: u16,
    pub open: bool,
    pub proxy: Vec<Rule>,
}

pub(crate) fn run(cx: &Context, project: Project, options: Options) -> Result {
    let started = Instant::now();
    let proxy = Rules::new([&project], &options.proxy)?;
    // Finding out the port is busy after a two-minute compile is a bad trade.
    server::check_port(options.port)?;
    cx.reporter.banner("development");
    cx.reporter.field(
        "Local",
        server::url(options.port, &project.config.base_path),
    );
    cx.reporter.blank();
    let app = prepare(cx, project)?;
    server::dev(cx, vec![app], &options, started, proxy)
}

/// Every application the site mounts, each with its own watcher, served from
/// one origin the way `build --site` lays them out.
pub(crate) fn site(cx: &Context, options: Options) -> Result {
    let started = Instant::now();
    server::check_port(options.port)?;
    let metadata = workspace::read_metadata(cx, cx.manifest_path.as_deref())?;
    let config = SiteConfig::from_metadata(&metadata.metadata)?;
    let root = config
        .mounts
        .keys()
        .min_by_key(|path| path.len())
        .map_or("/", String::as_str);
    cx.reporter.banner("development");
    cx.reporter.field("Local", server::url(options.port, root));
    cx.reporter.field("Apps", site::apps(&config.mounts));
    cx.reporter.blank();
    let manifest = metadata.workspace_root.join("Cargo.toml");
    let mut discovered = Vec::new();
    for (path, name) in &config.mounts {
        let app = cx.for_manifest(manifest.clone(), Some(name.clone()));
        let project = Project::discover(&app)?;
        site::check_mount(path, &project)?;
        discovered.push((app, project));
    }
    let proxy = Rules::new(
        discovered.iter().map(|(_, project)| project),
        &options.proxy,
    )?;
    let mut apps = Vec::new();
    for (app, project) in discovered {
        apps.push(prepare(&app, project)?);
    }
    server::dev(cx, apps, &options, started, proxy)
}

/// Prepare tools and make the first development build into `.fusor/dev`.
fn prepare(cx: &Context, project: Project) -> Result<DevApp> {
    toolchain::prepare(cx, &project, false, false)?;
    let mut cx = cx.clone();
    cx.output = Some(project.root.join(layout::DEV_OUTPUT));
    super::build::run(&cx, &project, crate::pipeline::BuildMode::Development, None)
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
