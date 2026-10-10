//! The Fusor application CLI. `ARCHITECTURE.md` describes the module layout.
mod cli;
mod commands;
mod context;
mod dev;
mod error;
mod layout;
mod pipeline;
mod process;
mod reporter;
mod toolchain;
mod transaction;
mod workspace;

pub use error::{Error, Kind};

use clap::Parser;
use cli::Cli;
use context::Context;
use std::ffi::OsString;

/// `args` includes the executable name, as `env::args_os` yields.
pub fn run(args: impl IntoIterator<Item = impl Into<OsString> + Clone>) -> Result<(), Error> {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        // `--help` and `--version` arrive here as errors with a zero exit code.
        Err(error) if error.exit_code() == 0 => {
            error.print()?;
            return Ok(());
        }
        Err(error) => return Err(Error::usage(error.render().to_string())),
    };
    let cx = Context::new(&cli);
    let Some(action) = cli.command else {
        use clap::CommandFactory;
        Cli::command().print_help()?;
        cx.reporter.result("")?;
        return Ok(());
    };
    commands::dispatch(&cx, action)
}
