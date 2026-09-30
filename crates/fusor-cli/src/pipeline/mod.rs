//! [`Publication`] is the staged build both paths share: stage beside the
//! output, optionally keep the previous generation, swap atomically.
pub(crate) mod cargo;
pub(crate) mod declarations;
pub(crate) mod diagnostics;
pub(crate) mod html;
pub(crate) mod islands;
pub(crate) mod manifest;
pub(crate) mod publish;
pub(crate) mod site;
pub(crate) mod wasm;
pub(crate) mod workers;

use crate::{
    context::Context,
    error::{Error, Result},
    layout,
    process::checked,
    transaction::Staging,
    workspace::Project,
};
use manifest::OutputManifest;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) struct Publication<'a> {
    cx: &'a Context,
    project: &'a Project,
    pub generation: String,
    site: PathBuf,
    staging: PathBuf,
    generated: PathBuf,
    _guard: Staging,
}

impl<'a> Publication<'a> {
    pub fn begin(cx: &'a Context, project: &'a Project) -> Result<Self> {
        let publication = Self::stage(cx, project, publish::generation())?;
        std::fs::create_dir(publication.staging.join(layout::GENERATED))?;
        std::fs::create_dir(&publication.generated)?;
        Ok(publication)
    }

    /// A development refresh republishes the current generation, and the one
    /// it retained, under a new document.
    pub fn revise(cx: &'a Context, project: &'a Project, generation: String) -> Result<Self> {
        let publication = Self::stage(cx, project, generation)?;
        publish::copy_tree(
            &publication.site.join(layout::GENERATED),
            &publication.staging.join(layout::GENERATED),
        )?;
        let requirements = publication.site.join(layout::WORKER_HEADERS);
        if requirements.is_file() {
            std::fs::copy(
                requirements,
                publication.staging.join(layout::WORKER_HEADERS),
            )?;
        }
        Ok(publication)
    }

    fn stage(cx: &'a Context, project: &'a Project, generation: String) -> Result<Self> {
        project.validate_output(cx)?;
        build_assets(cx, project)?;
        let site = project.output(cx);
        let parent = site
            .parent()
            .ok_or_else(|| Error::project("the output directory has no parent"))?;
        std::fs::create_dir_all(parent)?;
        // Named apart from the generation, which a revision reuses.
        let staging = parent.join(format!("{}{}", layout::STAGE_PREFIX, publish::generation()));
        std::fs::create_dir(&staging)?;
        let guard = Staging(staging.clone());
        publish::copy_assets(project, &staging)?;
        let generated = layout::generated(&staging, &generation);
        cx.reporter.note(format!(
            "staging generation {generation} in {}",
            staging.display()
        ));
        Ok(Self {
            cx,
            project,
            generation,
            site,
            staging,
            generated,
            _guard: guard,
        })
    }

    /// A page loaded a moment ago still references the previous generation's
    /// URLs. Only one predecessor is kept.
    pub fn retain_previous(&self) -> Result {
        let Ok(previous) = OutputManifest::read(&self.site) else {
            return Ok(());
        };
        let old = layout::generated(&self.site, &previous.generation);
        if old.is_dir() {
            publish::copy_tree(
                &old,
                &layout::generated(&self.staging, &previous.generation),
            )?;
        }
        Ok(())
    }

    pub fn staging(&self) -> &Path {
        &self.staging
    }

    /// Everything the build generates belongs here, so generations never
    /// collide.
    pub fn generated(&self) -> &Path {
        &self.generated
    }

    pub fn url_prefix(&self) -> String {
        format!(
            "{}{}/{}",
            self.project.config.base_path,
            layout::GENERATED,
            self.generation
        )
    }

    /// Assets are copied again, after Cargo has run the build scripts that
    /// generate some of them; staging already had them for the refresh check.
    pub fn commit(self, manifest: &OutputManifest) -> Result {
        publish::copy_assets(self.project, &self.staging)?;
        manifest.write(&self.staging)?;
        publish::publish(self.cx, self.project, &self.staging)
    }
}

fn build_assets(cx: &Context, project: &Project) -> Result {
    let Some((program, args)) = project.config.assets_build.split_first() else {
        return Ok(());
    };
    let mut command = Command::new(program);
    command.args(args).current_dir(&project.root);
    if cx.offline {
        command
            .env("CARGO_NET_OFFLINE", "true")
            .env("npm_config_offline", "true");
    }
    checked(&mut command)
}
