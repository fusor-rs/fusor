use super::BuildMode;
use crate::{
    error::{Error, Result},
    layout,
    process::checked,
};
use std::{env, fs, path::Path, process::Command};

pub(crate) fn bindgen(
    binary: &Path,
    wasm: &Path,
    out_dir: &Path,
    out_name: &str,
    mode: BuildMode,
) -> Result {
    let mut command = Command::new(binary);
    command
        .arg(wasm)
        .args(["--target", "web", "--out-name", out_name, "--out-dir"])
        .arg(out_dir);
    // Names cost transfer size. FUSOR_KEEP_WASM_NAMES keeps them in release,
    // for readable stack traces.
    let keep_names = env::var_os("FUSOR_KEEP_WASM_NAMES").is_some_and(|value| value == "1");
    if mode == BuildMode::Release && !keep_names {
        command.arg("--remove-name-section");
    }
    checked(&mut command)
}

/// Opt-in through `FUSOR_WASM_OPT` only: no download, no `PATH` search. A
/// failure removes partial output and leaves the staged module untouched.
pub(crate) fn optimize(path: &Path, mode: BuildMode) -> Result {
    if mode != BuildMode::Release {
        return Ok(());
    }
    let Some(binary) = env::var_os("FUSOR_WASM_OPT") else {
        return Ok(());
    };
    let keep_names = env::var_os("FUSOR_KEEP_WASM_NAMES").is_some_and(|value| value == "1");
    run_wasm_opt(path, Path::new(&binary), keep_names)
}

fn run_wasm_opt(path: &Path, binary: &Path, keep_names: bool) -> Result {
    verify_version(binary)?;
    let output = path.with_extension("optimized.wasm");
    let result = optimize_into(path, &output, binary, keep_names);
    if result.is_err() {
        let _ = fs::remove_file(&output);
    }
    result
}

fn verify_version(binary: &Path) -> Result {
    let version = Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|error| {
            Error::tooling(format!("could not run FUSOR_WASM_OPT: {error}"))
                .remedy("point FUSOR_WASM_OPT at a wasm-opt executable, or unset it")
        })?;
    if !version.status.success() {
        return Err(Error::tooling(
            "FUSOR_WASM_OPT failed to report its version",
        ));
    }
    let version = String::from_utf8_lossy(&version.stdout);
    let expected = format!("wasm-opt version {}", layout::WASM_OPT_VERSION);
    if !version.starts_with(&format!("{expected} ")) && version.trim() != expected {
        return Err(
            Error::tooling(format!("FUSOR_WASM_OPT reports {:?}", version.trim())).remedy(format!(
                "select Binaryen wasm-opt version {}",
                layout::WASM_OPT_VERSION
            )),
        );
    }
    Ok(())
}

fn optimize_into(path: &Path, output: &Path, binary: &Path, keep_names: bool) -> Result {
    let mut command = Command::new(binary);
    command.arg(path).args([
        "-O3",
        "--enable-bulk-memory",
        "--enable-reference-types",
        "--enable-multivalue",
        "--enable-sign-ext",
        "--enable-nontrapping-float-to-int",
    ]);
    if keep_names {
        command.arg("--debuginfo");
    }
    checked(command.arg("-o").arg(output))?;
    // Binaryen validates the module; this rejects a success that wrote
    // nothing or wrote the text format.
    if !fs::read(output)?.starts_with(b"\0asm\x01\0\0\0") {
        return Err(Error::tooling(
            "wasm-opt did not produce a WebAssembly module",
        ));
    }
    fs::rename(output, path)?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_failing_optimizer_preserves_the_staged_module_and_removes_partial_output() {
        let directory = env::temp_dir().join(format!("fusor-wasm-opt-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let _cleanup = crate::transaction::Staging(directory.clone());
        let tool = directory.join("wasm-opt");
        let wasm = directory.join("app.wasm");
        fs::write(&wasm, b"\0asm\x01\0\0\0").unwrap();
        fs::write(
            &tool,
            "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'wasm-opt version 132 (version_132)'; exit 0; fi\nfor arg do last=\"$arg\"; done\nprintf partial > \"$last\"\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(run_wasm_opt(&wasm, &tool, false).is_err());
        assert_eq!(fs::read(&wasm).unwrap(), b"\0asm\x01\0\0\0");
        assert!(!wasm.with_extension("optimized.wasm").exists());
    }
}
