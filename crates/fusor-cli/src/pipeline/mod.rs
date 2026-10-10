//! [`Publication`] is the staged build both paths share: stage beside the
//! output, optionally keep the previous generation, swap atomically.
pub(crate) mod assets;
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
    dev::sources::{self, Snapshot},
    error::{Error, Result},
    layout,
    process::checked,
    transaction::Staging,
    workspace::Project,
};
use manifest::{Javascript, OutputManifest};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildMode {
    Development,
    Debug,
    Release,
}

impl From<bool> for BuildMode {
    fn from(debug: bool) -> Self {
        if debug { Self::Debug } else { Self::Release }
    }
}

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
    pub fn output_manifest(
        &self,
        artifact: &fusor_build::app::ArtifactManifest,
        bundle: Javascript,
        mode: BuildMode,
    ) -> Result<OutputManifest> {
        let mut output = OutputManifest::new(self.generation.clone(), &self.project.config);
        if !artifact.javascript.is_empty() {
            output.javascript = Some(bundle);
        }
        if mode != BuildMode::Development {
            return Ok(output);
        }
        output.revision = Some(0);
        output.reload_after = Some(0);
        // Recorded only when reuse is possible, so the watcher does not
        // re-establish eligibility on every edit.
        if crate::dev::refresh::enabled(self.cx, self.project, artifact)? {
            output.rust_signature = Some(crate::dev::refresh::signature(artifact)?);
        }
        Ok(output)
    }

    pub fn begin(
        cx: &'a Context,
        project: &'a Project,
        watched: Option<&mut Snapshot>,
    ) -> Result<Self> {
        let publication = Self::stage(cx, project, publish::generation()?, watched)?;
        std::fs::create_dir(publication.staging.join(layout::GENERATED))?;
        std::fs::create_dir(&publication.generated)?;
        Ok(publication)
    }

    /// A development refresh republishes the current generation, and the one
    /// it retained, under a new document.
    pub fn revise(
        cx: &'a Context,
        project: &'a Project,
        generation: String,
        watched: &mut Snapshot,
    ) -> Result<Self> {
        let publication = Self::stage(cx, project, generation, Some(watched))?;
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

    fn stage(
        cx: &'a Context,
        project: &'a Project,
        generation: String,
        watched: Option<&mut Snapshot>,
    ) -> Result<Self> {
        project.validate_output(cx)?;
        build_assets(cx, project, watched)?;
        let site = project.output(cx);
        let parent = site
            .parent()
            .ok_or_else(|| Error::project("the output directory has no parent"))?;
        std::fs::create_dir_all(parent)?;
        // Named apart from the generation, which a revision reuses.
        let staging = parent.join(format!(
            "{}{}",
            layout::STAGE_PREFIX,
            publish::generation()?
        ));
        std::fs::create_dir(&staging)?;
        let guard = Staging(staging.clone());
        assets::copy(project, &staging)?;
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
        let Some(previous) = OutputManifest::read_optional(&self.site)? else {
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
        assets::copy(self.project, &self.staging)?;
        manifest.write(&self.staging)?;
        publish::publish(self.cx, self.project, &self.staging)
    }
}

fn build_assets(cx: &Context, project: &Project, watched: Option<&mut Snapshot>) -> Result {
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
    let hooked = assets::sources(project)?;
    let result = checked(&mut command);
    if let Some(watched) = watched {
        sources::record_assets(project, &hooked, watched)?;
    }
    result
}
