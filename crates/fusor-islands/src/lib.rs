//! Portable descriptors contain wire types and identity, never browser state or
//! component factories. Browser entries live in independently built packages.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;
mod error;
pub use error::Error;

/// Browser registry shipped with this protocol release.
pub const REGISTRY_JAVASCRIPT: &str = include_str!("../runtime/registry.js");
/// Browser composition runtime shipped with this protocol release.
pub const COMPOSITION_JAVASCRIPT: &str = include_str!("../runtime/composition.js");

pub const PROTOCOL_VERSION: u32 = 1;

/// Attributes on an island's host element. The server writes them; the
/// browser registry and preview activation check them before binding.
pub mod attributes {
    pub const ISLAND: &str = "data-fusor-island";
    pub const UNIT: &str = "data-fusor-unit";
    pub const GENERATION: &str = "data-fusor-generation";
    pub const SCHEMA: &str = "data-fusor-schema";
    pub const HASH: &str = "data-fusor-hash";
    pub const ACTIVATE: &str = "data-fusor-activate";
    pub const PREFETCH: &str = "data-fusor-prefetch";
    /// Marks the host's inert script that carries the props as JSON text.
    pub const PROPS: &str = "data-fusor-props";
    pub const PROPS_SCRIPT: &str = r#"script[type="application/json"][data-fusor-props]"#;
    /// Every attribute that identifies an instance, in the order the server
    /// writes them. The registry keeps its own copy as `metadataNames`.
    pub const METADATA: [&str; 8] = [
        "id", ISLAND, UNIT, GENERATION, SCHEMA, HASH, ACTIVATE, PREFETCH,
    ];
}

/// Implement in a small shared wire-types crate. Changing a wire contract needs
/// a new schema identifier; derives do not prove cross-target compatibility.
pub trait Island: 'static {
    type Props: Serialize + DeserializeOwned;
    const NAME: &'static str;
    const UNIT: &'static str;
    const SCHEMA: &'static str;
    const MODE: RenderMode = RenderMode::Attach;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RenderMode {
    #[default]
    Attach,
    Preview,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Activation {
    #[default]
    Load,
    Visible,
    Idle,
    Interaction,
    Manual,
}
impl Activation {
    pub const ALL: [Self; 5] = [
        Self::Load,
        Self::Visible,
        Self::Idle,
        Self::Interaction,
        Self::Manual,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Visible => "visible",
            Self::Idle => "idle",
            Self::Interaction => "interaction",
            Self::Manual => "manual",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Prefetch {
    #[default]
    None,
    Load,
    Visible,
    Idle,
}
impl Prefetch {
    pub const ALL: [Self; 4] = [Self::None, Self::Load, Self::Visible, Self::Idle];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Load => "load",
            Self::Visible => "visible",
            Self::Idle => "idle",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub unit: String,
    pub descriptor: String,
    pub props_schema: String,
    pub template_hash: String,
    pub mode: RenderMode,
}
impl Entry {
    /// The entry for island `D`, rendered by the component template with this hash.
    pub fn new<D: Island>(template_hash: &str) -> Self {
        Self {
            unit: D::UNIT.into(),
            descriptor: D::NAME.into(),
            props_schema: D::SCHEMA.into(),
            template_hash: template_hash.into(),
            mode: D::MODE,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    pub javascript: String,
    pub wasm: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub entries: Vec<Entry>,
}

/// Public delivery protocol. It is independent of the compiler's private Cargo
/// artifact manifest and the browser's mutable per-instance state.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryManifest {
    pub version: u32,
    pub generation: String,
    pub units: BTreeMap<String, Unit>,
}
impl DeliveryManifest {
    pub fn validate(&self) -> Result<(), Error> {
        if self.version != PROTOCOL_VERSION {
            return Err(Error::Protocol(self.version));
        }
        if self.generation.is_empty() {
            return Err(Error::Manifest {
                identity: "generation".into(),
                reason: "delivery generation cannot be empty",
            });
        }
        let mut descriptors = std::collections::BTreeSet::new();
        for (name, unit) in &self.units {
            if name.is_empty() || unit.entries.is_empty() {
                return Err(Error::Manifest {
                    identity: name.clone(),
                    reason: "delivery unit needs a name and entries",
                });
            }
            for url in std::iter::once(&unit.javascript)
                .chain(std::iter::once(&unit.wasm))
                .chain(&unit.dependencies)
            {
                if !url.starts_with('/')
                    || url.starts_with("//")
                    || url.contains("..")
                    || url.contains(['#', '?', '\\'])
                {
                    return Err(Error::Manifest {
                        identity: url.clone(),
                        reason: "delivery URLs must be immutable, same-origin absolute paths",
                    });
                }
            }
            for entry in &unit.entries {
                if &entry.unit != name {
                    return Err(Error::Manifest {
                        identity: entry.descriptor.clone(),
                        reason: "island entry belongs to a different delivery unit",
                    });
                }
                if entry.descriptor.is_empty()
                    || entry.props_schema.is_empty()
                    || entry.template_hash.is_empty()
                    || !descriptors.insert(&entry.descriptor)
                {
                    return Err(Error::Manifest {
                        identity: entry.descriptor.clone(),
                        reason: "island descriptors must be unique and include schema and template identity",
                    });
                }
            }
        }
        Ok(())
    }
    pub fn entry<D: Island>(&self) -> Result<&Entry, Error> {
        let entry = self
            .units
            .get(D::UNIT)
            .and_then(|unit| {
                unit.entries
                    .iter()
                    .find(|entry| entry.descriptor == D::NAME)
            })
            .ok_or(Error::Unregistered {
                descriptor: D::NAME,
                unit: D::UNIT,
            })?;
        if entry.props_schema != D::SCHEMA || entry.mode != D::MODE {
            return Err(Error::DescriptorMismatch(D::NAME));
        }
        Ok(entry)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitWitness {
    pub version: u32,
    pub entries: Vec<Entry>,
}

/// JSON stays opaque text across JavaScript. This preserves all Rust integer
/// values, including u64 values above JavaScript's Number precision.
pub fn encode<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(value)
}
pub fn decode<T: DeserializeOwned>(value: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(value)
}

#[cfg(feature = "browser")]
pub mod browser;

#[cfg(test)]
mod tests {
    use super::{REGISTRY_JAVASCRIPT, attributes};

    #[test]
    fn the_registry_checks_the_same_host_attributes() {
        let names = attributes::METADATA
            .map(|name| format!("'{name}'"))
            .join(", ");
        assert!(REGISTRY_JAVASCRIPT.contains(&format!("const metadataNames = [{names}];")));
        assert!(REGISTRY_JAVASCRIPT.contains(attributes::PROPS_SCRIPT));
    }
}
