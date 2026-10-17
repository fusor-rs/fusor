//! Compiler diagnostics, remapped to their HTML. Written to stderr directly: a
//! status verb would break the alignment of rustc's carets.
use crate::error::Result;
use fusor_build::{SourceMap, app::SourceArtifact};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Default)]
pub(crate) struct Diagnostics(BTreeMap<String, (String, SourceMap, bool)>);

impl Diagnostics {
    pub fn register(&mut self, sources: &[SourceArtifact], workspace: &Path) -> Result {
        for file in sources {
            let source = file
                .source
                .strip_prefix(workspace)
                .unwrap_or(&file.source)
                .to_string_lossy()
                .into_owned();
            let map = fs::read_to_string(&file.map)?.parse::<SourceMap>()?;
            self.0.insert(
                file.rust.to_string_lossy().into_owned(),
                (source.clone(), map.clone(), file.external.is_none()),
            );
            if let Some(registration) = &file.registration {
                let map = SourceMap::new(vec![fusor_build::BindingLocation {
                    generated_start: 1,
                    generated_end: fs::read_to_string(&registration.rust)?.lines().count() + 1,
                    line: registration.line,
                    column: registration.column,
                }])?;
                self.0.insert(
                    registration.rust.to_string_lossy().into_owned(),
                    (source.clone(), map.clone(), false),
                );
                if let Ok(relative) = registration.rust.strip_prefix(workspace) {
                    self.0.insert(
                        relative.to_string_lossy().into_owned(),
                        (source.clone(), map, false),
                    );
                }
            }
            if let Ok(relative) = file.rust.strip_prefix(workspace) {
                self.0.insert(
                    relative.to_string_lossy().into_owned(),
                    (source, map, file.external.is_none()),
                );
            }
        }
        Ok(())
    }

    pub fn print(&self, diagnostic: &Value, workspace: &Path) {
        if let Some(rendered) = diagnostic["rendered"].as_str() {
            let mut rendered = rendered.to_owned();
            let mut notes = BTreeSet::new();
            self.locations(diagnostic, &mut notes, &mut rendered);
            eprint!("{rendered}");
            if matches!(diagnostic["code"]["code"].as_str(), Some("E0432" | "E0433")) {
                for (name, capability) in [
                    ("fusor_router", "router"),
                    ("fusor_async", "async"),
                    ("fusor_query", "query"),
                    ("fusor_std::forms", "forms"),
                    ("fusor_std::actions", "actions"),
                ] {
                    if rendered.contains(name) {
                        eprintln!(
                            "  = Fusor capability: run `fusor add {capability}` to add the direct dependency and browser features."
                        );
                    }
                }
            }

            for (source, line, column) in notes {
                eprintln!("  = HTML binding at {source}:{line}:{column}");
                if let Ok(html) = fs::read_to_string(workspace.join(&source)) {
                    if let Some(text) = html.lines().nth(line - 1) {
                        eprintln!("    {line} | {}", text.trim());
                    }
                }
            }
        }
    }

    fn locations(
        &self,
        value: &Value,
        notes: &mut BTreeSet<(String, usize, usize)>,
        rendered: &mut String,
    ) {
        match value {
            Value::Object(fields) => {
                if let (Some(file), Some(line)) =
                    (value["file_name"].as_str(), value["line_start"].as_u64())
                {
                    if let Some((source, map, inline)) = self.0.get(file) {
                        if let Some(location) = map.lookup(line as usize) {
                            notes.insert((source.clone(), location.line, location.column));
                        } else if *inline {
                            // Script lines are copied verbatim. Remap their
                            // locations even when a help span also mentions a
                            // generated component impl in another HTML module.
                            // Generated binding lines keep native coordinates.
                            *rendered = rendered
                                .replace(&format!("{file}:{line}:"), &format!("{source}:{line}:"));
                        }
                    }
                }
                for value in fields.values() {
                    self.locations(value, notes, rendered);
                }
            }
            Value::Array(values) => {
                for value in values {
                    self.locations(value, notes, rendered);
                }
            }
            _ => {}
        }
    }
}
