use crate::commands::add::Capability;
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum Color {
    #[default]
    Auto,
    Always,
    Never,
}

impl Color {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}

#[derive(Parser)]
#[command(
    name = "fusor",
    version,
    about = "Build reactive applications from HTML and ordinary Rust",
    after_help = "Examples:\n  fusor new my-app\n  fusor dev\n  fusor add router\n  fusor build\n  fusor preview"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub command: Option<Action>,
    /// Cargo.toml of the application or its workspace
    #[arg(long, global = true)]
    pub manifest_path: Option<PathBuf>,
    /// Select an application in a Cargo workspace
    #[arg(short, long, global = true)]
    pub package: Option<String>,
    #[arg(long, global = true)]
    pub offline: bool,
    #[arg(long, global = true)]
    pub locked: bool,
    /// Require unchanged locks and disable all network access
    #[arg(long, global = true)]
    pub frozen: bool,
    /// Cargo features to enable (comma or space separated)
    #[arg(long, global = true)]
    pub features: Vec<String>,
    #[arg(long, global = true, conflicts_with = "quiet")]
    pub verbose: bool,
    #[arg(long, global = true)]
    pub quiet: bool,
    #[arg(long, global = true, value_enum, default_value = "auto")]
    pub color: Color,
}

#[derive(Subcommand)]
pub(crate) enum Action {
    /// Create and prepare an independent Cargo application
    New {
        path: PathBuf,
        /// Use this framework checkout for dependencies (before registry release)
        #[arg(long)]
        framework_path: Option<PathBuf>,
        #[arg(long)]
        javascript: bool,
        /// Write sources without resolving dependencies or preparing tools
        #[arg(long)]
        skip_install: bool,
        /// Use the documented starter defaults without prompts
        #[arg(long)]
        yes: bool,
    },
    /// Fetch dependencies and prepare matching WebAssembly and JavaScript tools
    #[command(visible_alias = "setup")]
    Install,
    /// Report project and tool problems without downloads or repairs
    Doctor,
    /// Add a first-party capability without generating application source
    Add {
        #[arg(value_enum)]
        capability: Capability,
        #[arg(long)]
        dry_run: bool,
    },
    /// Type-check Rust and HTML using unchanged dependency locks
    Check,
    /// Build a deployable static site using prepared tools (release by default)
    Build {
        #[arg(long)]
        debug: bool,
        /// Build every application in [workspace.metadata.fusor.site] into one site
        #[arg(long, conflicts_with = "package")]
        site: bool,
    },
    /// Prepare tools, build, watch, serve and reload after successful changes
    Dev {
        #[arg(long, default_value_t = 4173, value_parser = clap::value_parser!(u16).range(1..))]
        port: u16,
        #[arg(long)]
        open: bool,
        /// Develop every application in [workspace.metadata.fusor.site] together
        #[arg(long, conflicts_with = "package")]
        site: bool,
    },
    /// Preview existing production output locally, without Cargo or installation
    #[command(visible_alias = "serve")]
    Preview {
        directory: Option<PathBuf>,
        #[arg(long, default_value_t = 4173, value_parser = clap::value_parser!(u16).range(1..))]
        port: u16,
        #[arg(long)]
        open: bool,
    },
    /// Replace this fusor with the latest stable release
    Upgrade,
    /// Print generated Rust for one HTML module without compiling Wasm
    Expand {
        #[arg(long, default_value = "app")]
        module: String,
    },
    /// Contributor operations for the Fusor framework checkout
    #[command(hide = true)]
    Repo {
        #[command(subcommand)]
        command: RepoAction,
    },
}

#[derive(Subcommand)]
pub(crate) enum RepoAction {
    /// Run native contributor verification
    Check,
}
