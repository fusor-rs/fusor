//! Several applications published as one site, each at its own base path.
//!
//! Each application builds into its own directory first. The site is then
//! assembled in one staging directory and swapped in, so a failure in any
//! application leaves the last complete site serving.
use super::{manifest::OutputManifest, publish};
use crate::{
    context::Context,
    error::{Error, Result},
    layout,
    transaction::Staging,
    workspace::Project,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

/// `[workspace.metadata.fusor.site]` in the workspace `Cargo.toml`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct SiteConfig {
    #[serde(default = "default_output")]
    pub output: PathBuf,
    /// URL path → package, such as `"/docs/" = "my-docs"`.
    pub mounts: BTreeMap<String, String>,
}

fn default_output() -> PathBuf {
    PathBuf::from("dist")
}

impl SiteConfig {
    pub fn from_metadata(metadata: &serde_json::Value) -> Result<Self> {
        let Some(site) = metadata.get("fusor").and_then(|fusor| fusor.get("site")) else {
            return Err(Error::usage("this workspace declares no site").remedy(
                "add [workspace.metadata.fusor.site.mounts] mapping each URL path to an application",
            ));
        };
        let config: Self = serde_json::from_value(site.clone()).map_err(|error| {
            Error::project(format!("invalid [workspace.metadata.fusor.site]: {error}"))
        })?;
        if config.mounts.is_empty() {
            return Err(Error::project("the site mounts no applications"));
        }
        let inside = config
            .output
            .components()
            .all(|part| matches!(part, Component::Normal(_)));
        if config.output.as_os_str().is_empty() || !inside {
            return Err(Error::project(format!(
                "site output {} must be a path inside the workspace",
                config.output.display()
            )));
        }
        Ok(config)
    }
}

/// One application's built output and the URL it is served under.
pub(crate) struct Mount {
    pub name: String,
    pub base_path: String,
    pub built: PathBuf,
}

impl Mount {
    pub fn new(path: &str, project: &Project, built: PathBuf) -> Result<Self> {
        check_mount(path, project)?;
        Ok(Self {
            name: project.name.clone(),
            base_path: path.to_owned(),
            built,
        })
    }

    /// The mount's directory inside the site: `/docs/` lives in `docs/`.
    fn directory(&self) -> &str {
        self.base_path.trim_matches('/')
    }
}

/// The application's own `base-path` decides the URLs it generates, so it has
/// to be the path the site mounts it at.
pub(crate) fn check_mount(path: &str, project: &Project) -> Result {
    if project.config.base_path != path {
        return Err(Error::project(format!(
            "the site mounts {} at {path}, but its base-path is {}",
            project.name, project.config.base_path
        ))
        .remedy(format!(
            "make them match, in [workspace.metadata.fusor.site.mounts] or in {}",
            project.manifest.display()
        )));
    }
    Ok(())
}

/// Assemble the mounts into `site` and publish them together.
pub(crate) fn publish(cx: &Context, site: &Path, mut mounts: Vec<Mount>) -> Result {
    validate_mounts(&mounts)?;
    // Parents first, so a parent's files can be checked against each child's mount.
    mounts.sort_by_key(|mount| {
        mount
            .directory()
            .split('/')
            .filter(|s| !s.is_empty())
            .count()
    });

    validate_output(site)?;
    let parent = site
        .parent()
        .ok_or_else(|| Error::project("the site output has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!("{}{}", layout::STAGE_PREFIX, publish::generation()));
    fs::create_dir(&staging)?;
    let _guard = Staging(staging.clone());

    for mount in &mounts {
        let target = staging.join(mount.directory());
        if !mount.directory().is_empty() && target.exists() {
            return Err(Error::project(format!(
                "{} is mounted at {}, but another application already publishes {}/",
                mount.name,
                mount.base_path,
                mount.directory()
            ))
            .remedy("rename that directory, or move the application to another base-path"));
        }
        publish::copy_tree(&mount.built, &target)?;
    }
    let manifest = staging.join(layout::SITE_MANIFEST);
    let _access = publish::OUTPUT_ACCESS
        .lock()
        .map_err(|_| Error::internal("the output publication lock was poisoned"))?;
    if manifest.exists() {
        return Err(Error::project(format!(
            "an application publishes {}, which the site owns",
            layout::SITE_MANIFEST
        )));
    }
    let record = SiteManifest {
        version: 1,
        mounts: mounts
            .iter()
            .map(|mount| SiteMount {
                path: mount.base_path.clone(),
                application: mount.name.clone(),
            })
            .collect(),
    };
    fs::write(&manifest, serde_json::to_vec_pretty(&record)?)?;
    publish::swap(cx, site, &staging)
}

fn validate_mounts(mounts: &[Mount]) -> Result {
    for (index, mount) in mounts.iter().enumerate() {
        if let Some(other) = mounts[..index]
            .iter()
            .find(|other| other.base_path == mount.base_path)
        {
            return Err(Error::project(format!(
                "{} and {} are both served at {}",
                other.name, mount.name, mount.base_path
            ))
            .remedy("give each site application its own base-path"));
        }
    }
    Ok(())
}

/// The site's own ownership marker, the counterpart of an application's output
/// manifest: an existing directory must be empty or a site we published.
fn validate_output(site: &Path) -> Result {
    let occupied = site.exists()
        && (!site.is_dir()
            || (fs::read_dir(site)?.next().is_some()
                && !site.join(layout::SITE_MANIFEST).is_file()));
    if occupied {
        return Err(Error::project(format!(
            "{} is not empty and is not a fusor site",
            site.display()
        ))
        .remedy("set output in [workspace.metadata.fusor.site] to a directory fusor owns"));
    }
    Ok(())
}

/// What `preview` needs to route a request to the right application.
#[derive(Serialize, Deserialize)]
pub(crate) struct SiteManifest {
    version: u32,
    pub mounts: Vec<SiteMount>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub(crate) struct SiteMount {
    pub path: String,
    pub application: String,
}

impl SiteManifest {
    /// `None` when the directory holds a single application rather than a site.
    pub fn read(directory: &Path) -> Result<Option<Self>> {
        let path = directory.join(layout::SITE_MANIFEST);
        if !path.is_file() {
            return Ok(None);
        }
        let manifest: Self = serde_json::from_slice(&fs::read(&path)?).map_err(|error| {
            Error::project(format!("the site manifest is corrupt: {error}"))
                .remedy("run `fusor build --site` to regenerate it")
        })?;
        if manifest.version != 1 || manifest.mounts.is_empty() {
            return Err(
                Error::project("the site manifest is unsupported or corrupt")
                    .remedy("run `fusor build --site` to regenerate it"),
            );
        }
        // Mounts become filesystem paths; hold them to the base-path rules.
        for mount in &manifest.mounts {
            let config = fusor_build::app::AppConfig {
                base_path: mount.path.clone(),
                ..toml::from_str("")?
            };
            config.validate()?;
        }
        Ok(Some(manifest))
    }

    /// Every mount with its own output manifest, which carries its history fallback.
    pub fn outputs(&self, site: &Path) -> Result<Vec<(PathBuf, OutputManifest)>> {
        self.mounts
            .iter()
            .map(|SiteMount { path: base, .. }| {
                let directory = site.join(base.trim_matches('/'));
                let output = OutputManifest::read(&directory)?;
                if &output.base_path != base {
                    return Err(Error::project(format!(
                        "the site mounts {} at {base}, but it was built for {}",
                        directory.display(),
                        output.base_path
                    ))
                    .remedy("run `fusor build --site` to rebuild the site"));
                }
                Ok((directory, output))
            })
            .collect()
    }
}

/// One line per mount: its URL, the application, and where its files are.
pub(crate) fn describe<'a>(
    site: &Path,
    mounts: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> String {
    let mounts: Vec<_> = mounts.into_iter().collect();
    let width = mounts.iter().map(|(path, _)| path.len()).max().unwrap_or(0);
    let name_width = mounts.iter().map(|(_, name)| name.len()).max().unwrap_or(0);
    mounts
        .iter()
        .map(|(path, name)| {
            let directory = shown(&site.join(path.trim_matches('/')));
            format!("{path:width$}  {name:name_width$}  → {directory}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The banner's `Apps:` block: each URL path and the application serving it.
pub(crate) fn apps<'a>(mounts: impl IntoIterator<Item = (&'a String, &'a String)>) -> String {
    let mounts: Vec<_> = mounts.into_iter().collect();
    let width = mounts.iter().map(|(path, _)| path.len()).max().unwrap_or(0);
    mounts
        .iter()
        .map(|(path, name)| format!("{path:width$}  {name}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A directory as the user would type it: relative to the working directory
/// when it is inside it, always ending in `/`.
pub(crate) fn shown(directory: &Path) -> String {
    let relative = std::env::current_dir()
        .ok()
        .and_then(|cwd| directory.strip_prefix(cwd).ok().map(Path::to_owned))
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| directory.to_owned());
    format!("{}/", relative.display().to_string().trim_end_matches('/'))
}

/// The longest base path that serves `url`, including its bare form without the
/// trailing slash, which the application answers with a redirect.
pub(crate) fn route<'a, T>(mounts: &'a [(String, T)], url: &str) -> Option<&'a T> {
    let path = url.split('?').next().unwrap_or("");
    mounts
        .iter()
        .filter(|(base, _)| path.starts_with(base.as_str()) || path == base.trim_end_matches('/'))
        .max_by_key(|(base, _)| base.len())
        .map(|(_, value)| value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn requests_go_to_the_most_specific_mount() {
        let mounts = vec![
            ("/".to_owned(), "landing"),
            ("/docs/".to_owned(), "docs"),
            ("/docs/api/".to_owned(), "api"),
        ];
        assert_eq!(route(&mounts, "/"), Some(&"landing"));
        assert_eq!(route(&mounts, "/about"), Some(&"landing"));
        assert_eq!(route(&mounts, "/docs/installation?tab=1"), Some(&"docs"));
        assert_eq!(route(&mounts, "/docs"), Some(&"docs"));
        assert_eq!(route(&mounts, "/docs/api/signal"), Some(&"api"));
        assert_eq!(route(&mounts, "/documents"), Some(&"landing"));
        assert_eq!(route(&[("/docs/".to_owned(), "docs")], "/other"), None);
    }

    fn built(root: &Path, name: &str, base_path: &str, files: &[&str]) -> Mount {
        let built = root.join(name);
        for file in files {
            let path = built.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, name).unwrap();
        }
        Mount {
            name: name.into(),
            base_path: base_path.into(),
            built,
        }
    }

    fn scratch() -> (PathBuf, Staging) {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("fusor-site-{}-{sequence}", publish::generation()));
        fs::create_dir(&root).unwrap();
        (root.clone(), Staging(root))
    }

    #[test]
    fn applications_are_assembled_at_their_base_paths() {
        let (root, _cleanup) = scratch();
        let site = root.join("dist");
        let mounts = vec![
            built(&root, "docs", "/docs/", &["index.html"]),
            built(&root, "landing", "/", &["index.html", "brand/logo.svg"]),
        ];
        publish(&Context::default(), &site, mounts).unwrap();
        assert_eq!(
            fs::read_to_string(site.join("index.html")).unwrap(),
            "landing"
        );
        assert_eq!(
            fs::read_to_string(site.join("docs/index.html")).unwrap(),
            "docs"
        );
        let manifest = SiteManifest::read(&site).unwrap().unwrap();
        let paths: Vec<_> = manifest
            .mounts
            .iter()
            .map(|mount| mount.path.as_str())
            .collect();
        assert_eq!(paths, ["/", "/docs/"]);
        assert_eq!(manifest.mounts[1].application, "docs");

        // A rebuild replaces a site it published.
        let again = vec![built(&root, "landing2", "/", &["index.html"])];
        publish(&Context::default(), &site, again).unwrap();
        assert!(!site.join("docs").exists());
    }

    #[test]
    fn mounts_that_collide_fail_before_anything_is_published() {
        let (root, _cleanup) = scratch();
        let site = root.join("dist");
        let same = vec![
            built(&root, "a", "/docs/", &["index.html"]),
            built(&root, "b", "/docs/", &["index.html"]),
        ];
        let error = publish(&Context::default(), &site, same).unwrap_err();
        assert!(
            error.to_string().contains("both served at /docs/"),
            "{error}"
        );

        let occupied = vec![
            built(&root, "landing", "/", &["index.html", "docs/old.html"]),
            built(&root, "docs", "/docs/", &["index.html"]),
        ];
        let error = publish(&Context::default(), &site, occupied).unwrap_err();
        assert!(
            error.to_string().contains("already publishes docs/"),
            "{error}"
        );
        assert!(!site.exists());

        fs::create_dir_all(site.join("mine")).unwrap();
        let error = publish(
            &Context::default(),
            &site,
            vec![built(&root, "x", "/", &["index.html"])],
        )
        .unwrap_err();
        assert!(error.to_string().contains("not a fusor site"), "{error}");
    }

    #[test]
    fn a_site_manifest_cannot_point_outside_the_site() {
        let (root, _cleanup) = scratch();
        fs::write(
            root.join(layout::SITE_MANIFEST),
            r#"{"version":1,"mounts":[{"path":"/../escape/","application":"x"}]}"#,
        )
        .unwrap();
        assert!(SiteManifest::read(&root).is_err());
    }

    #[test]
    fn site_configuration_rejects_outputs_outside_the_workspace() {
        for output in ["../dist", "/tmp/dist", ""] {
            let metadata = serde_json::json!({
                "fusor": { "site": { "output": output, "mounts": { "/": "a" } } }
            });
            assert!(SiteConfig::from_metadata(&metadata).is_err(), "{output}");
        }
        let metadata = serde_json::json!({ "fusor": { "site": { "mounts": { "/": "a" } } } });
        assert_eq!(
            SiteConfig::from_metadata(&metadata).unwrap().output,
            PathBuf::from("dist")
        );
        assert!(SiteConfig::from_metadata(&serde_json::json!({})).is_err());
        let empty = serde_json::json!({ "fusor": { "site": { "mounts": {} } } });
        assert!(SiteConfig::from_metadata(&empty).is_err());
    }

    #[test]
    fn a_mount_must_match_the_applications_base_path() {
        let root = PathBuf::from("/w/docs");
        let project = Project {
            id: "docs".into(),
            name: "docs".into(),
            manifest: root.join("Cargo.toml"),
            workspace: PathBuf::from("/w"),
            target: PathBuf::from("/w/target"),
            watch_roots: vec![],
            config: toml::from_str(r#"base-path = "/docs/""#).unwrap(),
            root,
        };
        assert!(Mount::new("/docs/", &project, PathBuf::new()).is_ok());
        let error = Mount::new("/documentation/", &project, PathBuf::new())
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("mounts docs at /documentation/, but its base-path is /docs/"),
            "{error}"
        );
    }
}
