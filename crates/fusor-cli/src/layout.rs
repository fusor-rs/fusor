//! Every generated name this CLI writes, and every third-party version it pins.
use std::path::{Path, PathBuf};

/// Immutable per-generation output inside the published site.
pub(crate) const GENERATED: &str = "__fusor";

/// Application modules inside each immutable generation.
pub(crate) const PACKAGE: &str = "pkg";
pub(crate) const APP_NAME: &str = "app";
pub(crate) const APP_MODULE: &str = "app.js";
pub(crate) const APP_WASM: &str = "app_bg.wasm";
pub(crate) const BOOT_MODULE: &str = "boot.js";
pub(crate) const REFRESH_MODULE: &str = "refresh.js";

/// Output metadata. Its presence also marks a directory as fusor-owned, which
/// is what makes replacing it safe.
pub(crate) const OUTPUT_MANIFEST: &str = ".fusor-output.json";

/// Project-local CLI state; safe to delete.
pub(crate) const STATE: &str = ".fusor";

/// Kept apart from `dist/` so a dev run never clobbers a deployable build.
pub(crate) const DEV_OUTPUT: &str = ".fusor/dev";

/// Under Cargo's target directory: where each application builds before
/// `build --site` assembles them.
pub(crate) const SITE_BUILD: &str = "fusor/site";

/// Marks a directory as an assembled site and lists its mounts.
pub(crate) const SITE_MANIFEST: &str = ".fusor-site.json";

/// Editor-facing TypeScript declarations emitted by the compiler.
pub(crate) const TYPES: &str = ".fusor/types";

/// Records a completed npm installation, keyed by manifest and lock contents.
pub(crate) const NPM_STAMP: &str = ".fusor/npm-install.json";

/// Marks a new project whose first npm resolution has not completed yet.
pub(crate) const PENDING_JAVASCRIPT: &str = ".fusor/pending-javascript";

/// Sibling directories holding uncommitted work. The scaffolded `.gitignore`
/// and the watcher both exclude `.fusor-*`.
pub(crate) const STAGE_PREFIX: &str = ".fusor-stage-";
pub(crate) const SCAFFOLD_PREFIX: &str = ".fusor-new-";

/// A partial download inside the tool cache.
pub(crate) const INSTALL_PREFIX: &str = ".install-";

pub(crate) const TARGET: &str = "wasm32-unknown-unknown";

/// Applications are checked against these, so changing one is part of a CLI
/// release.
pub(crate) const BINDGEN_VERSION: &str = "0.2.117";
pub(crate) const ESBUILD_VERSION: &str = "0.28.2";
pub(crate) const WASM_OPT_VERSION: &str = "132";

/// The Rust toolchain new projects pin. The CLI itself builds on older Rust;
/// see the workspace `rust-version`.
pub(crate) const RUST_VERSION: &str = "1.95.0";

pub(crate) fn generated(site: &Path, generation: &str) -> PathBuf {
    site.join(GENERATED).join(generation)
}
