//! Version-1 CLI output metadata. Missing legacy optional fields have defaults;
//! a present field with the wrong type is corrupt, never a request for a default.
use crate::{
    error::{Error, Result},
    layout,
};
use fusor_build::app::AppConfig;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::{fs, path::Path};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OutputManifest {
    version: u32,
    pub generation: String,
    pub base_path: String,
    #[serde(default)]
    pub history_fallback: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub revision: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub reload_after: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rust_signature: Option<Value>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub javascript: Option<Javascript>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub delivery: Option<Delivery>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Javascript {
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub styles: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Delivery {
    Islands,
}

fn present<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

impl OutputManifest {
    pub fn new(generation: String, config: &AppConfig) -> Self {
        Self {
            version: 1,
            generation,
            base_path: config.base_path.clone(),
            history_fallback: config.history_fallback.clone(),
            revision: None,
            reload_after: None,
            rust_signature: None,
            javascript: None,
            delivery: None,
        }
    }

    fn validate(&self) -> Result {
        if self.version != 1
            || !self.generation.starts_with("g-")
            || !self
                .generation
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'-' || b == b'g')
        {
            return Err(
                Error::project("the output manifest is unsupported or corrupt")
                    .remedy("run `fusor build` to regenerate it"),
            );
        }
        // Artifact-only preview has no Cargo manifest to validate these paths.
        let config = AppConfig {
            base_path: self.base_path.clone(),
            history_fallback: self.history_fallback.clone(),
            ..AppConfig::default()
        };
        config.validate()?;
        Ok(())
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(bytes).map_err(|error| {
            Error::project(format!("the output manifest is corrupt: {error}"))
                .remedy("run `fusor build` to regenerate it")
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn read(directory: &Path) -> Result<Self> {
        Self::read_optional(directory)?.ok_or_else(|| {
            Error::project(format!(
                "{} is missing",
                directory.join(layout::OUTPUT_MANIFEST).display()
            ))
            .remedy("run `fusor build` to generate it")
        })
    }

    pub fn read_optional(directory: &Path) -> Result<Option<Self>> {
        let path = directory.join(layout::OUTPUT_MANIFEST);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::from(error).context(path.display())),
        };
        Self::decode(&bytes)
            .map(Some)
            .map_err(|error| error.context(path.display()))
    }

    pub fn write(&self, directory: &Path) -> Result {
        self.validate()?;
        fs::write(
            directory.join(layout::OUTPUT_MANIFEST),
            serde_json::to_vec_pretty(self)?,
        )?;
        Ok(())
    }

    pub fn revision(&self) -> u64 {
        self.revision.unwrap_or(0)
    }
    pub fn reload_after(&self) -> u64 {
        self.reload_after.unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn version_one_production_dev_javascript_and_islands_round_trip() {
        for fields in [
            json!({}),
            json!({"revision": 2, "reload_after": 1, "rust_signature": {"app": [["token", 1]]}}),
            json!({"javascript": {"inputs": ["/src/module.ts"], "styles": ["module.css"]}}),
            json!({"delivery": "islands", "revision": 0, "reload_after": 0}),
        ] {
            let mut value = json!({"version": 1, "generation": "g-123-4", "base_path": "/", "history_fallback": []});
            value
                .as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            let manifest = OutputManifest::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
            assert_eq!(serde_json::to_value(manifest).unwrap(), value);
        }
    }

    #[test]
    fn missing_legacy_fields_default_but_corrupt_fields_fail() {
        let legacy = json!({"version": 1, "generation": "g-1", "base_path": "/"});
        let manifest = OutputManifest::decode(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(manifest.revision(), 0);
        assert_eq!(manifest.reload_after(), 0);
        assert!(manifest.history_fallback.is_empty());
        assert!(manifest.javascript.is_none());
        for (field, invalid) in [
            ("version", json!(2)),
            ("generation", json!("../g-1")),
            ("base_path", json!(null)),
            ("revision", json!(-1)),
            ("revision", json!(null)),
            ("reload_after", json!("0")),
            ("history_fallback", json!(null)),
            ("javascript", json!({"inputs": [1]})),
            ("javascript", json!({"styles": "style.css"})),
            ("delivery", json!("other")),
        ] {
            let mut value = legacy.clone();
            value[field] = invalid;
            assert!(
                OutputManifest::decode(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{value}"
            );
        }
        for field in ["version", "generation", "base_path"] {
            let mut value = legacy.clone();
            value.as_object_mut().unwrap().remove(field);
            assert!(
                OutputManifest::decode(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{field}"
            );
        }
    }
}
