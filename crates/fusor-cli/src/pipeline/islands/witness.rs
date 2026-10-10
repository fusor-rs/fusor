//! A mismatch here would otherwise ship a page that looks fine but whose
//! islands silently fail to attach.
use crate::error::{Error, Result};
use fusor_build::app::ArtifactManifest;
use fusor_islands::{DeliveryManifest, RenderMode, UnitWitness};
use std::process::Command;

pub(crate) fn witness(command: &mut Command, what: &str) -> Result<UnitWitness> {
    let output = command.output().map_err(|error| {
        Error::tooling(format!("could not run {what}: {error}"))
            .remedy("island registration is validated by running the compiled artifacts; FUSOR_NODE selects the Node executable")
    })?;
    if !output.status.success() {
        return Err(Error::compile(format!(
            "{what} failed to report its registrations: {}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        )));
    }
    let witness: UnitWitness = serde_json::from_slice(&output.stdout)?;
    if witness.version != fusor_islands::PROTOCOL_VERSION {
        return Err(Error::project(format!(
            "{what} uses island protocol {} but this CLI speaks {}",
            witness.version,
            fusor_islands::PROTOCOL_VERSION
        ))
        .remedy("keep the framework packages and the CLI on one release"));
    }
    Ok(witness)
}

/// Every browser island needs exactly one native counterpart with the same
/// props and, where it attaches to markup, the same template.
pub(crate) fn validate_pairs(native: &UnitWitness, delivery: &DeliveryManifest) -> Result {
    let browser = || delivery.units.values().flat_map(|unit| &unit.entries);
    let browser_count = browser().count();
    if native.entries.len() != browser_count {
        return Err(Error::compile(format!(
            "the native renderer registered {} islands and the browser units registered {}",
            native.entries.len(),
            browser_count
        )));
    }
    for entry in browser() {
        let mut matching = native
            .entries
            .iter()
            .filter(|native| native.descriptor == entry.descriptor);
        let (Some(native), None) = (matching.next(), matching.next()) else {
            return Err(Error::compile(format!(
                "missing or duplicate native registration for {}",
                entry.descriptor
            )));
        };
        if native.unit != entry.unit
            || native.props_schema != entry.props_schema
            || native.mode != entry.mode
            || (entry.mode == RenderMode::Attach && native.template_hash != entry.template_hash)
        {
            return Err(Error::compile(format!(
                "the native and browser registrations for {} disagree",
                entry.descriptor
            )));
        }
    }
    Ok(())
}

/// A native render cannot execute component JavaScript, so reject it at the
/// authored location.
pub(crate) fn reject_javascript(artifact: &ArtifactManifest) -> Result {
    let Some(module) = artifact.javascript.first() else {
        return Ok(());
    };
    Err(Error::project(format!(
        "{}:{}:{}: component JavaScript modules are not supported in island delivery",
        module.source.display(),
        module.line,
        module.column
    ))
    .remedy("use a browser application for components with JavaScript modules"))
}
