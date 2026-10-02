//! The native-module bridge used by generated HTML bindings.

/// Associate package-relative HTML with this ordinary Rust module.
///
/// The include path below is a contract with `fusor-build`, which writes the
/// file and, in `app::includes`, exempts this shape from disabling development
/// refresh. Change them together.
///
/// `compile_app()` compiles the default entry (`web/index.html`), reusable
/// templates discovered under `web/components/`, and configured sources before
/// macro expansion. HTML for this mode contains no Rust script. Generated impls
/// stay in this lexical scope, including access to private state and imports.
/// Call once per HTML file. Multiple component declarations in one file share
/// this module. Use `/` separators and the exact configured package-relative path.
#[macro_export]
macro_rules! template {
    (backend = $backend:literal, $path:literal) => {
        include!(concat!(
            env!("OUT_DIR"),
            "/fusor_backends/",
            $backend,
            "/",
            $path,
            ".rs"
        ));
    };
    ($path:literal) => {
        include!(concat!(env!("OUT_DIR"), "/fusor_templates/", $path, ".rs"));
    };
}

/// Include an HTML module's generated bindings in the ordinary Rust module
/// declared by its script's `rust:module` attribute. The argument is the entry
/// registration (`app`) or a component registration from Cargo metadata.
///
/// As with [`template!`], the include path below is a contract with
/// `fusor-build`; change them together.
///
/// The authored Rust file remains in Cargo's normal module tree. Only generated
/// implementations are included here; private fields retain their visibility.
#[macro_export]
macro_rules! bindings {
    ($name:ident) => {
        const __FUSOR_BINDINGS_ORIGIN: (&str, &str) = (file!(), env!("CARGO_MANIFEST_DIR"));
        include!(concat!(
            env!("OUT_DIR"),
            "/fusor_",
            stringify!($name),
            ".rs"
        ));
    };
}

/// Compare native `file!()` origins with the explicitly registered source.
/// Paths may contain native separators and `#[path]`'s lexical `..` components.
#[doc(hidden)]
pub const fn source_matches(origin: (&str, &str), expected: &str) -> bool {
    let file = native_path(origin.0.as_bytes());
    let manifest = native_path(origin.1.as_bytes());
    let expected = PathParts(b"", native_path(expected.as_bytes()));
    if same_path(PathParts(b"", file), expected) {
        return true;
    }
    // Cargo's file!() is either absolute or relative to the workspace/package
    // invocation directory. Only ancestors of this package are candidates;
    // accepting arbitrary suffixes would allow a different nested src/app.rs.
    let mut end = manifest.len();
    loop {
        if same_path(PathParts(manifest.split_at(end).0, file), expected) {
            return true;
        }
        if end == 0 {
            return false;
        }
        end -= 1;
        while end > 0 && !separator(manifest[end]) {
            end -= 1;
        }
    }
}

// std::fs::canonicalize produces verbatim Windows paths; Cargo's source and
// manifest paths generally do not include this prefix.
const fn native_path(path: &[u8]) -> &[u8] {
    if path.len() >= 4
        && separator(path[0])
        && separator(path[1])
        && path[2] == b'?'
        && separator(path[3])
    {
        let path = path.split_at(4).1;
        if path.len() >= 4
            && path[0] == b'U'
            && path[1] == b'N'
            && path[2] == b'C'
            && separator(path[3])
        {
            return path.split_at(4).1;
        }
        return path;
    }
    path
}

const fn separator(byte: u8) -> bool {
    byte == b'/' || byte == b'\\'
}

#[derive(Clone, Copy)]
struct PathParts<'a>(&'a [u8], &'a [u8]);
impl PathParts<'_> {
    const fn len(self) -> usize {
        self.0.len() + 1 + self.1.len()
    }
    const fn at(self, index: usize) -> u8 {
        if index < self.0.len() {
            self.0[index]
        } else if index == self.0.len() {
            b'/'
        } else {
            self.1[index - self.0.len() - 1]
        }
    }
}

const fn previous(path: PathParts<'_>, mut end: usize) -> Option<(usize, usize)> {
    let mut parents = 0;
    while end > 0 {
        while end > 0 && separator(path.at(end - 1)) {
            end -= 1;
        }
        if end == 0 {
            break;
        }
        let mut start = end;
        while start > 0 && !separator(path.at(start - 1)) {
            start -= 1;
        }
        if end - start == 2 && path.at(start) == b'.' && path.at(start + 1) == b'.' {
            parents += 1;
        } else if end - start == 1 && path.at(start) == b'.' {
        } else if parents > 0 {
            parents -= 1;
        } else {
            return Some((start, end));
        }
        end = start;
    }
    None
}

const fn same_path(left: PathParts<'_>, right: PathParts<'_>) -> bool {
    let mut a = left.len();
    let mut b = right.len();
    loop {
        match (previous(left, a), previous(right, b)) {
            (None, None) => return true,
            (Some((start_a, end_a)), Some((start_b, end_b))) => {
                if end_a - start_a != end_b - start_b {
                    return false;
                }
                let mut index = 0;
                while index < end_a - start_a {
                    if left.at(start_a + index) != right.at(start_b + index) {
                        return false;
                    }
                    index += 1;
                }
                a = start_a;
                b = start_b;
            }
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::source_matches;

    #[test]
    fn source_identity_handles_cargo_and_native_module_paths() {
        for (origin, expected) in [
            (("src/app.rs", "/app"), "/app/src/app.rs"),
            (("/app/src/app.rs", "/app"), "/app/src/app.rs"),
            (
                ("examples/app/src/app.rs", "/workspace/examples/app"),
                "/workspace/examples/app/web/../src/app.rs",
            ),
            (("src/../web/page.rs", "/app"), "/app/web/page.rs"),
            (("src\\app.rs", "C:\\app"), "C:/app/src/app.rs"),
            (("src\\app.rs", "C:\\app"), "\\\\?\\C:\\app\\src\\app.rs"),
            (
                ("src\\app.rs", "\\\\server\\app"),
                "\\\\?\\UNC\\server\\app\\src\\app.rs",
            ),
            (
                ("./src/nested/../../web/page.rs", "/app"),
                "/app/web/page.rs",
            ),
        ] {
            assert!(source_matches(origin, expected));
            assert!(!source_matches(origin, "/another/page.rs"));
            assert!(!source_matches(origin, "/app/other/src/app.rs"));
        }
    }
}
