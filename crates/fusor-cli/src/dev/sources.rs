//! Hash polling rather than filesystem events behaves the same on every
//! platform and with editors that write through a temporary file.
use crate::{
    context::Context,
    error::Result,
    layout,
    pipeline::{assets, manifest::OutputManifest},
    workspace::Project,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
};

pub(crate) type Snapshot = BTreeMap<PathBuf, u64>;

pub(crate) fn snapshot(cx: &Context, project: &Project) -> Result<Snapshot> {
    let mut files = source_snapshot(cx, project)?;
    // Package imports and authored modules outside the Cargo package roots are
    // exact dependencies, even though scanning all node_modules is unnecessary.
    // Missing files disappear from the snapshot and therefore trigger a rebuild.
    let Some(javascript) =
        OutputManifest::read_optional(&project.output(cx))?.and_then(|output| output.javascript)
    else {
        return Ok(files);
    };
    for input in javascript.inputs {
        let path = PathBuf::from(input);
        match fs::read(&path) {
            Ok(contents) => {
                files.insert(path, fingerprint(&contents));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(files)
}

/// These files stay watched across publications. Previous output metadata may
/// add JavaScript inputs that disappear after the next full build.
pub(crate) fn source_snapshot(cx: &Context, project: &Project) -> Result<Snapshot> {
    let generated = [
        project.target.clone(),
        project.output(cx),
        project.root.join(&project.config.output),
    ];
    let mut files = Snapshot::new();
    for root in &project.watch_roots {
        collect(root, &generated, &mut files)?;
    }
    // A dependency or profile change is a source change too.
    for name in ["Cargo.toml", "Cargo.lock", ".cargo"] {
        let path = project.workspace.join(name);
        if path.exists() {
            collect(&path, &generated, &mut files)?;
        }
    }
    record_asset_files(project, &mut files)?;
    Ok(files)
}

/// Assets are copied after the hook. The earlier snapshot of every other file
/// is kept, so edits made during the hook and afterward still trigger a build.
pub(crate) fn record_assets(
    project: &Project,
    hooked: &BTreeSet<PathBuf>,
    previous: &mut Snapshot,
) -> Result {
    previous.retain(|path, _| !hooked.contains(path));
    record_asset_files(project, previous)
}

/// Asset files can lie outside every watch root, such as `../../install.sh`.
fn record_asset_files(project: &Project, files: &mut Snapshot) -> Result {
    for path in assets::sources(project)? {
        let contents = fs::read(&path)?;
        files.insert(path, fingerprint(&contents));
    }
    Ok(())
}

fn collect(path: &Path, generated: &[PathBuf], out: &mut Snapshot) -> io::Result<()> {
    if generated.iter().any(|generated| generated == path) {
        return Ok(());
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Ok(());
    };
    if IGNORED.contains(&name) || name.starts_with(".fusor-") {
        return Ok(());
    }
    let kind = fs::symlink_metadata(path)?;
    if kind.is_symlink() {
        return Ok(());
    }
    if kind.is_dir() {
        for entry in fs::read_dir(path)? {
            collect(&entry?.path(), generated, out)?;
        }
    } else if kind.is_file() {
        out.insert(path.to_owned(), fingerprint(&fs::read(path)?));
    }
    Ok(())
}

fn fingerprint(contents: &[u8]) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    contents.hash(&mut hash);
    hash.finish()
}

/// Walking these would make every build look like a source change.
const IGNORED: &[&str] = &[
    "target",
    "dist",
    ".git",
    "node_modules",
    ".cache",
    ".codex",
    ".agents",
    layout::STATE,
    "test-results",
];
