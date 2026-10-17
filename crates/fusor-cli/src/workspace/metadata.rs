//! The fields of `cargo metadata` this CLI reads. Unknown fields are ignored so
//! newer Cargo output still parses.
use crate::error::{Error, Result};
use serde::Deserialize;
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Deserialize)]
pub(crate) struct Metadata {
    pub packages: Vec<Package>,
    pub workspace_members: Vec<String>,
    pub workspace_root: PathBuf,
    pub target_directory: PathBuf,
    /// Absent only under `--no-deps`, which this CLI never passes.
    pub resolve: Option<Resolve>,
    /// `[workspace.metadata]`.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Deserialize)]
pub(crate) struct Package {
    pub id: String,
    pub name: String,
    pub version: String,
    pub manifest_path: PathBuf,
    /// `None` for a path dependency.
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub targets: Vec<Target>,
}

#[derive(Deserialize)]
pub(crate) struct Target {
    pub name: String,
    #[serde(default)]
    pub kind: Vec<String>,
    #[serde(default)]
    pub crate_types: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct Resolve {
    pub nodes: Vec<Node>,
}

#[derive(Deserialize)]
pub(crate) struct Node {
    pub id: String,
    pub dependencies: Vec<String>,
}

impl Metadata {
    pub fn decode(json: &[u8]) -> Result<Self> {
        serde_json::from_slice(json)
            .map_err(|error| Error::internal(format!("could not read Cargo metadata: {error}")))
    }

    pub fn is_member(&self, package: &Package) -> bool {
        self.workspace_members.contains(&package.id)
    }

    /// Includes `root`. Excluding unrelated members keeps the dev watcher from
    /// rebuilding on an edit to a sibling crate.
    pub fn dependency_ids(&self, root: &str) -> Result<BTreeSet<String>> {
        let nodes = &self
            .resolve
            .as_ref()
            .ok_or_else(|| Error::internal("Cargo metadata has no resolved graph"))?
            .nodes;
        let mut found = BTreeSet::new();
        let mut pending = vec![root.to_owned()];
        while let Some(id) = pending.pop() {
            if !found.insert(id.clone()) {
                continue;
            }
            let node = nodes.iter().find(|node| node.id == id).ok_or_else(|| {
                Error::internal(format!("resolved Cargo package {id} is missing"))
            })?;
            pending.extend(node.dependencies.iter().cloned());
        }
        Ok(found)
    }
}

impl Package {
    /// Must agree with `select::candidate`, which reads the manifest instead.
    pub fn is_application(&self) -> bool {
        self.metadata
            .get("fusor")
            .is_some_and(|fusor| fusor.get("library") != Some(&serde_json::Value::Bool(true)))
    }

    pub fn has_target(&self, name: &str, kind: &str) -> bool {
        self.targets
            .iter()
            .any(|target| target.name == name && target.kind.iter().any(|k| k == kind))
    }

    pub fn has_crate_type(&self, crate_type: &str) -> bool {
        self.targets
            .iter()
            .any(|target| target.crate_types.iter().any(|t| t == crate_type))
    }

    pub fn root(&self) -> Result<&std::path::Path> {
        self.manifest_path.parent().ok_or_else(|| {
            Error::internal(format!(
                "Cargo manifest {} has no parent directory",
                self.manifest_path.display()
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(json: serde_json::Value) -> Metadata {
        Metadata::decode(&serde_json::to_vec(&json).unwrap()).unwrap()
    }

    #[test]
    fn watching_excludes_unrelated_workspace_packages_but_keeps_transitive_dependencies() {
        let metadata = metadata(serde_json::json!({
            "packages": [], "workspace_members": [], "workspace_root": "/w",
            "target_directory": "/w/target",
            "resolve": { "nodes": [
                {"id": "app", "dependencies": ["runtime", "compiler"]},
                {"id": "runtime", "dependencies": ["shared"]},
                {"id": "compiler", "dependencies": ["shared"]},
                {"id": "shared", "dependencies": []},
                {"id": "unrelated", "dependencies": ["runtime"]}
            ]}
        }));
        assert_eq!(
            metadata.dependency_ids("app").unwrap(),
            ["app", "compiler", "runtime", "shared"]
                .into_iter()
                .map(str::to_owned)
                .collect()
        );
    }

    #[test]
    fn only_a_package_with_fusor_metadata_and_no_library_flag_is_an_application() {
        let metadata = metadata(serde_json::json!({
            "packages": [
                {"id": "a", "name": "a", "version": "0.1.0", "manifest_path": "/w/a/Cargo.toml"},
                {"id": "b", "name": "b", "version": "0.1.0", "manifest_path": "/w/b/Cargo.toml",
                 "metadata": {"fusor": {}}},
                {"id": "c", "name": "c", "version": "0.1.0", "manifest_path": "/w/c/Cargo.toml",
                 "metadata": {"fusor": {"library": true}}}
            ],
            "workspace_members": ["b", "c"], "workspace_root": "/w",
            "target_directory": "/w/target", "resolve": {"nodes": []}
        }));
        let package = |name: &str| {
            metadata
                .packages
                .iter()
                .find(|package| package.name == name)
                .expect("declared package")
        };
        assert!(!package("a").is_application());
        assert!(package("b").is_application());
        assert!(!package("c").is_application());
        assert!(!metadata.is_member(package("a")));
    }
}
