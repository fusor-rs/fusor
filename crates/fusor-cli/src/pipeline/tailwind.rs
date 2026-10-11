//! Compiles the application's Tailwind stylesheet into the generation.
//!
//! Tailwind finds classes by scanning the package for candidate strings, so it
//! sees static `class` attributes and string literals in Rust. A `class:name`
//! binding leaves no class in any file it scans; the compiler lists those
//! names, and a generated entry passes them to Tailwind with `@source inline`.
use super::BuildMode;
use crate::{
    error::{Error, Result},
    layout, toolchain,
    workspace::Project,
};
use fusor_build::app::ArtifactManifest;
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};

/// Digits of the content hash in the stylesheet's file name.
const HASH_LENGTH: usize = 16;

/// Writes the compiled stylesheet into `package` and returns its file name.
/// The name follows the content, so a changed stylesheet gets a new immutable
/// URL and a dev refresh can link it without reloading the page.
pub(crate) fn compile(
    project: &Project,
    artifact: &ArtifactManifest,
    package: &Path,
    mode: BuildMode,
) -> Result<Option<String>> {
    let Some(stylesheet) = &project.config.tailwind else {
        return Ok(None);
    };
    let binary = toolchain::tailwind::resolve()?;
    let work = project
        .target
        .join(layout::TAILWIND_BUILD)
        .join(&project.name);
    fs::create_dir_all(&work)?;
    let input = work.join("input.css");
    fs::write(&input, entry(&project.root.join(stylesheet), artifact))?;
    let mut command = Command::new(&binary);
    // Tailwind scans from its working directory.
    command
        .arg("--input")
        .arg(&input)
        .current_dir(&project.root);
    if mode == BuildMode::Release {
        command.arg("--minify");
    }
    let output = command
        .output()
        .map_err(|error| Error::tooling(format!("could not run {}: {error}", binary.display())))?;
    if !output.status.success() {
        return Err(Error::compile(format!(
            "Tailwind CSS could not compile {}:\n{}",
            stylesheet.display(),
            diagnostic(&String::from_utf8_lossy(&output.stderr))
        )));
    }
    let digest = format!("{:x}", Sha256::digest(&output.stdout));
    let name = format!("tailwind-{}.css", &digest[..HASH_LENGTH]);
    fs::write(package.join(&name), output.stdout)?;
    Ok(Some(name))
}

/// Class names contain no whitespace; the compiler rejects it in `class:name`.
fn entry(stylesheet: &Path, artifact: &ArtifactManifest) -> String {
    // CSS treats a backslash as an escape, and Windows accepts `/`.
    let path = stylesheet.to_string_lossy().replace('\\', "/");
    let mut css = format!("@import {};\n", css_string(&path));
    if !artifact.classes.is_empty() {
        let classes = Vec::from_iter(artifact.classes.iter().map(String::as_str)).join(" ");
        css.push_str(&format!("@source inline({});\n", css_string(&classes)));
    }
    css
}

fn css_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Tailwind always colors its errors and frames them with box drawing, and
/// prints its banner first. Keep only the message lines.
fn diagnostic(stderr: &str) -> String {
    let plain = toolchain::tailwind::strip_ansi(stderr);
    let lines: Vec<&str> = plain
        .lines()
        .map(|line| line.trim_start_matches(['│', '┌', '└']).trim())
        .filter(|line| !line.is_empty() && *line != "Error:" && !line.starts_with('≈'))
        .collect();
    if lines.is_empty() {
        plain.trim().to_owned()
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_keep_the_message_without_color_or_framing() {
        let stderr = "≈ tailwindcss v4.3.3\n\n\u{1b}[31mError:\u{1b}[39m\n\u{1b}[2m┌\u{1b}[22m\n\u{1b}[2m│\u{1b}[22m Error: Cannot apply unknown utility class `nonsense`\n\u{1b}[2m└\u{1b}[22m\n";
        assert_eq!(
            diagnostic(stderr),
            "Error: Cannot apply unknown utility class `nonsense`"
        );
    }
}
