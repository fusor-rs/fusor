//! `[package.metadata.fusor.assets]`: the files copied into the published site.
//! A literal directory publishes everything beneath it; a pattern publishes the
//! files it matches, relative to the literal path before its first wildcard.
use crate::{
    error::{Error, Result},
    layout,
    workspace::Project,
};
use fusor_build::app::{AssetPattern, AssetSource};
use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    fs, io,
    path::{Component, Path, PathBuf},
};

const RESERVED: [&str; 4] = [
    "index.html",
    layout::GENERATED,
    layout::OUTPUT_MANIFEST,
    layout::WORKER_HEADERS,
];

/// `*` and `?` never cross a `/`, and hidden files match only when the
/// pattern spells out the leading dot.
const MATCHING: glob::MatchOptions = glob::MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: true,
};

struct Match {
    relative: PathBuf,
    file: PathBuf,
}

pub(crate) fn copy(project: &Project, staging: &Path) -> Result {
    for (destination, file) in resolve(project)? {
        let target = staging.join(destination);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(file, target)?;
    }
    Ok(())
}

/// Every file the configuration currently matches, for the source watcher.
/// Unlike publishing, a pattern that matches nothing is not an error here.
pub(crate) fn sources(project: &Project) -> Result<BTreeSet<PathBuf>> {
    let mut files = BTreeSet::new();
    for source in project.config.assets.values() {
        for pattern in source.patterns() {
            files.extend(
                matches(&project.root, pattern)?
                    .into_iter()
                    .map(|found| found.file),
            );
        }
    }
    Ok(files)
}

/// Each published path, relative to the site root, and the file copied there.
fn resolve(project: &Project) -> Result<BTreeMap<PathBuf, PathBuf>> {
    let mut assets = BTreeMap::new();
    for (url, source) in &project.config.assets {
        let mut found = Vec::new();
        for pattern in source.patterns() {
            let files = matches(&project.root, pattern)?;
            if files.is_empty() {
                return Err(Error::project(format!(
                    "the asset pattern {:?} for {url} matches no files",
                    pattern.as_str()
                ))
                .remedy("create a matching file, or correct the pattern in [package.metadata.fusor.assets]"));
            }
            found.extend(files);
        }
        for (destination, file) in destinations(url, source, found)? {
            insert(&mut assets, destination, file)?;
        }
    }
    Ok(assets)
}

fn destinations(
    url: &str,
    source: &AssetSource,
    found: Vec<Match>,
) -> Result<Vec<(PathBuf, PathBuf)>> {
    let directory = url.ends_with('/');
    if !directory && found.len() != 1 {
        return Err(Error::project(format!(
            "the asset file {url} matches {} files",
            found.len()
        ))
        .remedy("end the URL with '/' to publish a directory, or narrow the pattern"));
    }
    let path = Path::new(url.trim_start_matches('/'));
    Ok(found
        .into_iter()
        .map(|Match { relative, file }| {
            let mut destination = if directory {
                path.join(relative)
            } else {
                path.to_owned()
            }
            .into_os_string();
            if let Some(suffix) = source.suffix() {
                destination.push(suffix);
            }
            (PathBuf::from(destination), file)
        })
        .collect())
}

fn insert(assets: &mut BTreeMap<PathBuf, PathBuf>, destination: PathBuf, file: PathBuf) -> Result {
    if destination
        .components()
        .next()
        .is_some_and(|first| RESERVED.iter().any(|name| first.as_os_str() == *name))
    {
        return Err(Error::project(format!(
            "the asset {} would be overwritten by generated output",
            destination.display()
        ))
        .remedy("publish it under another URL; fusor owns that name in the published site"));
    }
    match assets.entry(destination) {
        Entry::Vacant(entry) => {
            entry.insert(file);
        }
        Entry::Occupied(entry) if *entry.get() == file => {}
        Entry::Occupied(entry) => {
            return Err(Error::project(format!(
                "both {} and {} would be published as {}",
                entry.get().display(),
                file.display(),
                entry.key().display()
            ))
            .remedy("give one of them another URL in [package.metadata.fusor.assets]"));
        }
    }
    Ok(())
}

fn matches(root: &Path, pattern: &AssetPattern) -> Result<Vec<Match>> {
    let (base, wildcards) = pattern.split();
    let base = normalize(&root.join(base));
    let mut found = Vec::new();
    let Some(wildcards) = wildcards else {
        match fs::symlink_metadata(&base) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => walk(&base, &base, &mut found)?,
        }
        return Ok(found);
    };
    let literal = base
        .to_str()
        .ok_or_else(|| Error::project(format!("the asset path {} is not UTF-8", base.display())))?;
    let search = format!("{}/{wildcards}", glob::Pattern::escape(literal));
    let entries = glob::glob_with(&search, MATCHING)
        .map_err(|error| Error::from(error).context(pattern.as_str()))?;
    for entry in entries {
        let path = entry?;
        if !fs::symlink_metadata(&path)?.is_dir() {
            walk(&path, &base, &mut found)?;
        }
    }
    Ok(found)
}

fn walk(path: &Path, base: &Path, found: &mut Vec<Match>) -> Result {
    let kind = fs::symlink_metadata(path)?.file_type();
    if kind.is_dir() {
        for entry in fs::read_dir(path)? {
            walk(&entry?.path(), base, found)?;
        }
    } else if kind.is_file() {
        let relative = if path == base {
            Path::new(path.file_name().expect("a matched file has a name"))
        } else {
            path.strip_prefix(base)?
        };
        found.push(Match {
            relative: relative.to_owned(),
            file: path.to_owned(),
        });
    } else {
        return Err(Error::project(format!(
            "{} is a symlink or special file; published sites contain only files and directories",
            path.display()
        )));
    }
    Ok(())
}

/// Resolves `.` and `..` without the filesystem, so a file reached through
/// `../` has the same path wherever the watcher records it.
fn normalize(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normal.pop();
            }
            Component::CurDir => {}
            other => normal.push(other),
        }
    }
    normal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::Staging;

    struct Fixture {
        project: Project,
        _cleanup: Staging,
    }

    impl Fixture {
        fn new(assets: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "fusor-assets-{}",
                crate::pipeline::publish::generation().unwrap()
            ));
            let app = root.join("app");
            for (path, contents) in [
                ("install.sh", "installer"),
                ("app/public/style.css", "css"),
                ("app/public/.well-known/security.txt", "contact"),
                ("app/public/brand/logo.svg", "svg"),
                ("app/src/a.rs", "a"),
                ("app/src/nested/b.rs", "b"),
                ("app/src/.hidden.rs", "hidden"),
            ] {
                let path = root.join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, contents).unwrap();
            }
            let project = Project {
                id: "test".into(),
                name: "test".into(),
                manifest: app.join("Cargo.toml"),
                workspace: root.clone(),
                target: root.join("target"),
                watch_roots: vec![app.clone()],
                config: toml::from_str(&format!("[assets]\n{assets}")).unwrap(),
                root: app,
            };
            Self {
                project,
                _cleanup: Staging(root),
            }
        }

        fn published(&self) -> Result<BTreeMap<String, String>> {
            let staging = self.project.workspace.join("stage");
            copy(&self.project, &staging)?;
            let mut files = BTreeMap::new();
            list(&staging, &staging, &mut files);
            Ok(files)
        }
    }

    fn list(directory: &Path, staging: &Path, files: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                list(&path, staging, files);
            } else {
                let relative = path.strip_prefix(staging).unwrap().to_string_lossy();
                files.insert(
                    relative.replace('\\', "/"),
                    fs::read_to_string(&path).unwrap(),
                );
            }
        }
    }

    #[test]
    fn directories_files_and_patterns_publish_at_their_urls() {
        let fixture = Fixture::new(
            r#"
"/" = "public"
"/install.sh" = "../install.sh"
"/source/src/" = { files = "src/**/*.rs", suffix = ".txt" }
"#,
        );
        let expected = [
            (".well-known/security.txt", "contact"),
            ("brand/logo.svg", "svg"),
            ("install.sh", "installer"),
            ("source/src/a.rs.txt", "a"),
            ("source/src/nested/b.rs.txt", "b"),
            ("style.css", "css"),
        ];
        assert_eq!(
            fixture.published().unwrap(),
            expected
                .into_iter()
                .map(|(path, contents)| (path.to_owned(), contents.to_owned()))
                .collect()
        );
        assert!(
            sources(&fixture.project)
                .unwrap()
                .contains(&fixture.project.workspace.join("install.sh"))
        );
    }

    #[test]
    fn ambiguous_or_empty_entries_are_rejected_with_the_url() {
        for (assets, message) in [
            (r#""/data/" = "missing/*.json""#, "matches no files"),
            (r#""/one.rs" = "src/**/*.rs""#, "/one.rs matches 2 files"),
            (
                "\"/\" = \"public\"\n\"/style.css\" = \"src/a.rs\"",
                "would be published as style.css",
            ),
            (
                r#""/index.html" = "src/a.rs""#,
                "overwritten by generated output",
            ),
        ] {
            let error = Fixture::new(assets).published().unwrap_err().to_string();
            assert!(error.contains(message), "{assets}: {error}");
        }
    }
}
