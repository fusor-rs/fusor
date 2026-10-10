//! Which application a command operates on, from Cargo's metadata or, for
//! `doctor` and `preview` on a host without Rust, from the manifests alone.
//! Both enumerate candidates and then share [`choose`].
use crate::{
    context::Context,
    error::{Error, Result},
    workspace::metadata::Package,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub manifest: PathBuf,
    pub name: String,
}

/// `preferred` is the manifest the working directory or `--manifest-path`
/// points at. It wins, so `fusor build` inside a workspace member does the
/// obvious thing.
pub(crate) fn choose(
    candidates: Vec<Candidate>,
    package: Option<&str>,
    preferred: Option<&Path>,
) -> Result<Candidate> {
    if candidates.is_empty() {
        return Err(Error::project("no Fusor application found").remedy(
            "pass --manifest-path pointing at an application's Cargo.toml; an application declares [package.metadata.fusor]",
        ));
    }
    let mut matching: Vec<_> = candidates
        .iter()
        .filter(|candidate| package.is_none_or(|name| candidate.name == name))
        .collect();
    if let Some(preferred) = preferred {
        if let Some(candidate) = matching
            .iter()
            .find(|candidate| candidate.manifest == preferred)
        {
            return Ok((*candidate).clone());
        }
    }
    if matching.len() == 1 {
        return Ok(matching.remove(0).clone());
    }
    Err(ambiguous(&candidates, package))
}

fn ambiguous(candidates: &[Candidate], package: Option<&str>) -> Error {
    let list = candidates
        .iter()
        .map(|candidate| format!("  {} ({})", candidate.name, candidate.manifest.display()))
        .collect::<Vec<_>>()
        .join("\n");
    let message = match package {
        Some(name) => format!("no Fusor application named {name}. Applications here:\n{list}"),
        None => format!("several Fusor applications are in scope:\n{list}"),
    };
    Error::usage(message)
        .remedy("select one with --package NAME or --manifest-path PATH (preview also accepts an output directory)")
}

/// Members only: a path dependency outside the workspace is not buildable
/// from here.
pub(crate) fn from_metadata<'a>(
    metadata: &'a super::metadata::Metadata,
    cx: &Context,
    preferred: Option<&Path>,
) -> Result<&'a Package> {
    let candidates: Vec<_> = metadata
        .packages
        .iter()
        .filter(|package| metadata.is_member(package) && package.is_application())
        .map(|package| Candidate {
            manifest: package.manifest_path.clone(),
            name: package.name.clone(),
        })
        .collect();
    let chosen = choose(candidates, cx.package.as_deref(), preferred)?;
    metadata
        .packages
        .iter()
        .find(|package| package.manifest_path == chosen.manifest)
        .ok_or_else(|| Error::internal("selected package left Cargo's metadata"))
}

/// `None` when there is no Cargo manifest at all, which artifact-only
/// `preview` allows.
pub(crate) fn from_filesystem(cx: &Context) -> Result<Option<Candidate>> {
    let Some(located) = locate(cx)? else {
        return Ok(None);
    };
    let mut candidates = Vec::new();
    if let Some(candidate) = application(&located)? {
        candidates.push(candidate);
    }
    candidates.extend(workspace_members(&located)?);
    candidates.sort_by(|a, b| a.manifest.cmp(&b.manifest));
    candidates.dedup();
    choose(candidates, cx.package.as_deref(), Some(&located)).map(Some)
}

fn locate(cx: &Context) -> Result<Option<PathBuf>> {
    if let Some(manifest) = &cx.manifest_path {
        return Ok(Some(manifest.canonicalize().map_err(|error| {
            Error::usage(format!("--manifest-path {}: {error}", manifest.display()))
        })?));
    }
    Ok(std::env::current_dir()?
        .ancestors()
        .map(|directory| directory.join("Cargo.toml"))
        .find(|manifest| manifest.is_file()))
}

fn application(manifest: &Path) -> Result<Option<Candidate>> {
    let document: toml::Value = std::fs::read_to_string(manifest)?.parse()?;
    Ok(candidate(manifest, &document))
}

fn candidate(manifest: &Path, document: &toml::Value) -> Option<Candidate> {
    let package = document.get("package")?;
    package.get("metadata")?.get("fusor")?;
    Some(Candidate {
        manifest: manifest.to_owned(),
        name: package.get("name")?.as_str()?.to_owned(),
    })
}

/// The closest enclosing workspace's applications, honoring `members` globs
/// and `exclude` patterns as Cargo does.
fn workspace_members(located: &Path) -> Result<Vec<Candidate>> {
    let start = located
        .parent()
        .ok_or_else(|| Error::internal("manifest has no parent directory"))?;
    let Some((manifest, document)) = enclosing_workspace(start)? else {
        return Ok(Vec::new());
    };
    let root = manifest
        .parent()
        .ok_or_else(|| Error::internal("workspace has no parent directory"))?;
    let workspace = &document["workspace"];
    let excludes = workspace
        .get("exclude")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .map(glob::Pattern::new)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut candidates = Vec::new();
    if let Some(candidate) = candidate(&manifest, &document) {
        candidates.push(candidate);
    }
    let members = workspace
        .get("members")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str);
    for member in members {
        for directory in expand(root, Path::new(member))? {
            let relative = directory.strip_prefix(root)?;
            if excludes
                .iter()
                .any(|pattern| pattern.matches_path(relative))
            {
                continue;
            }
            let manifest = directory.join("Cargo.toml");
            if manifest.is_file() {
                candidates.extend(application(&manifest)?);
            }
        }
    }
    Ok(candidates)
}

fn enclosing_workspace(start: &Path) -> Result<Option<(PathBuf, toml::Value)>> {
    for directory in start.ancestors() {
        let manifest = directory.join("Cargo.toml");
        let contents = match std::fs::read_to_string(&manifest) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(Error::from(error).context(manifest.display())),
        };
        let document: toml::Value = contents
            .parse()
            .map_err(|error: toml::de::Error| Error::from(error).context(manifest.display()))?;
        if document.get("workspace").is_some() {
            return Ok(Some((manifest, document)));
        }
    }
    Ok(None)
}

fn expand(root: &Path, pattern: &Path) -> Result<Vec<PathBuf>> {
    let root = root
        .to_str()
        .ok_or_else(|| Error::project("workspace path is not UTF-8"))?;
    let pattern = pattern
        .to_str()
        .ok_or_else(|| Error::project("workspace member pattern is not UTF-8"))?;
    let pattern = format!("{}/{}", glob::Pattern::escape(root), pattern);
    let mut directories = Vec::new();
    for entry in glob::glob(&pattern)? {
        let directory = entry?;
        if directory.is_dir() {
            directories.push(directory);
        }
    }
    Ok(directories)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates() -> Vec<Candidate> {
        ["one", "two"]
            .into_iter()
            .map(|name| Candidate {
                manifest: PathBuf::from(format!("/w/{name}/Cargo.toml")),
                name: name.into(),
            })
            .collect()
    }

    #[test]
    fn the_working_directorys_application_wins_over_an_ambiguous_workspace() {
        let preferred = PathBuf::from("/w/two/Cargo.toml");
        assert_eq!(
            choose(candidates(), None, Some(&preferred)).unwrap().name,
            "two"
        );
    }

    #[test]
    fn a_sole_application_needs_no_selection() {
        let mut only = candidates();
        only.truncate(1);
        assert_eq!(choose(only, None, None).unwrap().name, "one");
    }

    #[test]
    fn ambiguity_and_unknown_names_are_usage_errors_listing_every_candidate() {
        for package in [None, Some("three")] {
            let error = choose(candidates(), package, None).unwrap_err();
            assert_eq!(error.kind(), crate::error::Kind::Usage);
            let text = error.to_string();
            assert!(text.contains("/w/one/Cargo.toml"), "{text}");
            assert!(text.contains("/w/two/Cargo.toml"), "{text}");
        }
    }

    #[test]
    fn no_application_at_all_reports_the_manifest_remedy() {
        let error = choose(Vec::new(), None, None).unwrap_err();
        assert!(error.to_string().contains("--manifest-path"));
    }
}
