//! Progress and diagnostics go to stderr as a short event log: a banner, then
//! `○` for work starting, `✓` for work done and `✗` for failures. What a user
//! might pipe goes to stdout.
use crate::{
    cli::Color,
    error::{Error, Result},
};
use std::{
    fmt::Display,
    io::{IsTerminal, Write},
    time::Duration,
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Verbosity {
    Quiet,
    #[default]
    Normal,
    Verbose,
}

#[derive(Clone, Default)]
pub(crate) struct Reporter {
    verbosity: Verbosity,
    color: bool,
}

impl Reporter {
    pub fn new(verbosity: Verbosity, color: Color) -> Self {
        let color = match color {
            Color::Always => true,
            Color::Never => false,
            // NO_COLOR is honored for `auto` only; an explicit flag wins.
            Color::Auto => {
                std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal()
            }
        };
        Self { verbosity, color }
    }

    pub fn is_quiet(&self) -> bool {
        self.verbosity == Verbosity::Quiet
    }

    pub fn is_verbose(&self) -> bool {
        self.verbosity == Verbosity::Verbose
    }

    /// `▲ fusor 0.1.0 · development`
    pub fn banner(&self, mode: impl Display) {
        if !self.is_quiet() {
            let name = self.paint(BOLD, &format!("▲ fusor {}", env!("CARGO_PKG_VERSION")));
            self.line(&format!("{name} · {mode}"));
        }
    }

    /// `- Local:    http://…`, with any further lines aligned under the value.
    pub fn field(&self, label: &str, value: impl Display) {
        if self.is_quiet() {
            return;
        }
        let label = format!("{label}:");
        let value = value.to_string();
        let mut lines = value.lines();
        self.line(&format!(
            "- {label:<9} {}",
            lines.next().unwrap_or_default()
        ));
        for line in lines {
            self.line(&format!("  {:<9} {line}", ""));
        }
    }

    pub fn blank(&self) {
        if !self.is_quiet() {
            self.line("");
        }
    }

    /// Work starting: `○ Compiling fusor-docs ...`
    pub fn step(&self, detail: impl Display) {
        if !self.is_quiet() {
            self.mark(CYAN, "○", detail);
        }
    }

    /// Work finished: `✓ Compiled fusor-docs in 18.0s`
    pub fn done(&self, detail: impl Display) {
        if !self.is_quiet() {
            self.mark(GREEN, "✓", detail);
        }
    }

    /// Printed even under `--quiet`.
    pub fn fail(&self, detail: impl Display) {
        self.mark(RED, "✗", detail);
    }

    /// Printed even under `--quiet`.
    pub fn warn(&self, detail: impl Display) {
        self.mark(YELLOW, "⚠", detail);
    }

    pub fn note(&self, detail: impl Display) {
        if self.is_verbose() {
            self.mark(DIM, " ", detail);
        }
    }

    /// `GET /docs/installation 200 in 2ms`
    pub fn request(&self, method: impl Display, path: &str, status: u16, elapsed: Duration) {
        if self.is_quiet() {
            return;
        }
        let color = match status {
            200..=299 => GREEN,
            300..=399 => CYAN,
            400..=499 => YELLOW,
            _ => RED,
        };
        let status = self.paint(color, &status.to_string());
        self.line(&format!(
            "{method} {path} {status} in {}",
            elapsed_text(elapsed)
        ));
    }

    /// Data a script consumes, such as generated source. Printed even under
    /// `--quiet`, which silences progress only.
    pub fn result(&self, detail: impl Display) -> Result {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{detail}")
            .map_err(|error| Error::from(error).context("writing command output"))
    }

    /// A server's address, for scripts. The banner already shows it to a person,
    /// so it goes to stdout only when stdout is not a terminal.
    pub fn address(&self, url: &str) -> Result {
        if !std::io::stdout().is_terminal() {
            self.result(url)?;
        }
        Ok(())
    }

    fn mark(&self, color: &str, symbol: &str, detail: impl Display) {
        let detail = detail.to_string();
        let mut lines = detail.lines();
        let symbol = self.paint(color, symbol);
        self.line(&format!("{symbol} {}", lines.next().unwrap_or_default()));
        for line in lines {
            self.line(&format!("  {line}"));
        }
    }

    fn paint(&self, color: &str, text: &str) -> String {
        if self.color {
            format!("{color}{text}{RESET}")
        } else {
            text.to_owned()
        }
    }

    fn line(&self, text: &str) {
        // Progress is best effort: losing stderr must not prevent command results.
        let _ = writeln!(std::io::stderr().lock(), "{text}");
    }
}

/// `520ms`, `4.2s`, `1m 12s`
pub(crate) fn elapsed_text(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    if elapsed < Duration::from_millis(1) {
        "<1ms".into()
    } else if seconds < 1.0 {
        format!("{}ms", elapsed.as_millis())
    } else if seconds < 60.0 {
        format!("{seconds:.1}s")
    } else {
        format!("{}m {}s", elapsed.as_secs() / 60, elapsed.as_secs() % 60)
    }
}

const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[1;32m";
const CYAN: &str = "\x1b[1;36m";
const YELLOW: &str = "\x1b[1;33m";
const RED: &str = "\x1b[1;31m";
const RESET: &str = "\x1b[0m";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_times_read_like_a_person_would_write_them() {
        assert_eq!(elapsed_text(Duration::from_micros(300)), "<1ms");
        assert_eq!(elapsed_text(Duration::from_millis(45)), "45ms");
        assert_eq!(elapsed_text(Duration::from_millis(4_230)), "4.2s");
        assert_eq!(elapsed_text(Duration::from_secs(72)), "1m 12s");
    }
}
