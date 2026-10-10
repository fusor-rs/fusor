//! Synchronous native HTML rendering. Applications resolve their own data and
//! return the resulting string through any HTTP framework or build-time tool.
mod error;
mod html;
pub use error::{Error, InputError, InputErrorDiagnostic};

pub use html::{Html, Writer};

use fusor::{Owner, OwnerHandle};
use fusor_islands::{
    Activation, DeliveryManifest, Entry, Island, Prefetch, RenderMode, UnitWitness, attributes,
};
use std::collections::{BTreeMap, BTreeSet};

pub type Result<T> = std::result::Result<T, Error>;

/// Implemented by `rust:render="server"` / `rust:render="shared"` HTML.
pub trait Render {
    /// Whether this component renders several native roots as a sibling range.
    const FRAGMENT: bool = false;
    const TEMPLATE_HASH: &'static str;
    fn render(&self, context: &mut Context<'_>) -> Result<Html>;
    #[doc(hidden)]
    fn render_with_children(
        &self,
        context: &mut Context<'_>,
        _children: Option<&Children<'_>>,
    ) -> Result<Html> {
        self.render(context)
    }
    /// Compiler fast path. Existing custom renderers keep their Html fallback.
    #[doc(hidden)]
    fn render_into(
        &self,
        context: &mut Context<'_>,
        children: Option<&Children<'_>>,
        writer: &mut Writer,
    ) -> Result<()> {
        let html = self.render_with_children(context, children)?;
        writer.rendered_root(&html);
        Ok(())
    }
}

/// Compiler-owned, borrowed children renderer. It keeps the caller's lexical
/// state while the receiving component decides where to render it. The selector
/// is `None` for default children and `Some(name)` for a named slot.
#[doc(hidden)]
pub type Children<'a> = dyn Fn(&mut Context<'_>, Option<&str>) -> Result<Html> + 'a;

pub struct Context<'a> {
    owner: Owner,
    islands: Option<Islands<'a>>,
    instances: BTreeSet<String>,
    next: u64,
    island_depth: usize,
}
impl Default for Context<'_> {
    fn default() -> Self {
        Self::new()
    }
}
impl<'a> Context<'a> {
    pub fn new() -> Self {
        Self {
            owner: Owner::new(),
            islands: None,
            instances: BTreeSet::new(),
            next: 0,
            island_depth: 0,
        }
    }
    pub fn with_islands(delivery: &'a DeliveryManifest, registry: &'a Registry) -> Result<Self> {
        delivery.validate()?;
        registry.validate(delivery)?;
        Ok(Self {
            islands: Some(Islands { delivery, registry }),
            ..Self::new()
        })
    }
    pub fn owner(&self) -> OwnerHandle {
        self.owner.handle()
    }
    pub fn child<C: Render>(&mut self, make: impl FnOnce(OwnerHandle) -> C) -> Result<Html> {
        self.try_child_with_children(|owner| Ok(make(owner)), None)
    }
    /// Construct and render a child whose input conversion can fail.
    pub fn try_child<C: Render>(
        &mut self,
        make: impl FnOnce(OwnerHandle) -> Result<C>,
    ) -> Result<Html> {
        self.try_child_with_children(make, None)
    }
    #[doc(hidden)]
    pub fn try_child_with_children<C: Render>(
        &mut self,
        make: impl FnOnce(OwnerHandle) -> Result<C>,
        children: Option<&Children<'_>>,
    ) -> Result<Html> {
        self.with_child_state(make, |state, context| {
            state.render_with_children(context, children)
        })
    }
    #[doc(hidden)]
    pub fn try_child_into_with_children<C: Render>(
        &mut self,
        make: impl FnOnce(OwnerHandle) -> Result<C>,
        children: Option<&Children<'_>>,
        writer: &mut Writer,
    ) -> Result<()> {
        self.with_child_state(make, |state, context| {
            state.render_into(context, children, writer)
        })
    }
    fn with_child_state<C, T>(
        &mut self,
        make: impl FnOnce(OwnerHandle) -> Result<C>,
        render: impl FnOnce(&C, &mut Context<'_>) -> Result<T>,
    ) -> Result<T> {
        let owner = Owner::child(&self.owner.handle());
        let state = fusor::coherence::prepare_state(owner.handle(), make)?;
        struct Restore<'a, 'b> {
            context: &'a mut Context<'b>,
            owner: Option<Owner>,
        }
        impl Drop for Restore<'_, '_> {
            fn drop(&mut self) {
                self.context.owner = self.owner.take().expect("previous owner");
            }
        }
        let previous = std::mem::replace(&mut self.owner, owner);
        let guard = Restore {
            context: self,
            owner: Some(previous),
        };
        render(&state, guard.context)
    }
    pub fn island<D: Island>(
        &mut self,
        id: Option<&str>,
        props: &D::Props,
        activation: Activation,
        prefetch: Prefetch,
    ) -> Result<Html> {
        let prepared = self.prepare_island::<D>(id, props, activation, prefetch)?;
        let mut html = Writer::new();
        html.open("div");
        prepared.attributes(&mut html);
        html.end_open();
        prepared.contents(&mut html);
        html.close("div");
        Ok(html.finish())
    }

    #[doc(hidden)]
    pub fn prepare_island<D: Island>(
        &mut self,
        id: Option<&str>,
        props: &D::Props,
        activation: Activation,
        prefetch: Prefetch,
    ) -> Result<PreparedIsland> {
        if self.island_depth != 0 {
            return Err("nested independent islands are unsupported".into());
        }
        let islands = self
            .islands
            .as_ref()
            .ok_or("islands require a delivery manifest and server registrations")?;
        let delivery = islands.delivery;
        let entry = delivery.entry::<D>()?;
        let registry = islands.registry;
        let registration = registry
            .entries
            .get(D::NAME)
            .ok_or("missing native island renderer")?;
        let next = self
            .next
            .checked_add(1)
            .ok_or("island instance counter overflow")?;
        let id = id
            .map(str::to_owned)
            .unwrap_or_else(|| format!("fusor-island-{next}"));
        if id.is_empty() || self.instances.contains(&id) {
            return Err("island instance IDs must be nonempty and unique".into());
        }
        let props = fusor_islands::encode(props)?;
        let initial = self.render_island(&registration.render, &props)?;
        if D::MODE == RenderMode::Preview && initial.is_editable() {
            return Err("replaceable island previews cannot contain editable controls".into());
        }
        self.next = next;
        self.instances.insert(id.clone());
        Ok(PreparedIsland {
            id,
            descriptor: D::NAME,
            unit: D::UNIT,
            schema: D::SCHEMA,
            generation: delivery.generation.clone(),
            hash: entry.template_hash.clone(),
            activation,
            prefetch,
            initial,
            props,
        })
    }
    fn render_island(&mut self, render: &ServerFactory, props: &str) -> Result<Html> {
        struct Rendering<'a, 'b>(&'a mut Context<'b>);
        impl Drop for Rendering<'_, '_> {
            fn drop(&mut self) {
                self.0.island_depth = 0;
            }
        }
        self.island_depth = 1;
        let guard = Rendering(self);
        render(props, guard.0)
    }
}

struct Islands<'a> {
    delivery: &'a DeliveryManifest,
    registry: &'a Registry,
}

#[doc(hidden)]
pub struct PreparedIsland {
    id: String,
    descriptor: &'static str,
    unit: &'static str,
    schema: &'static str,
    generation: String,
    hash: String,
    activation: Activation,
    prefetch: Prefetch,
    initial: Html,
    props: String,
}
impl PreparedIsland {
    pub fn attributes(&self, html: &mut Writer) {
        let values = [
            self.id.as_str(),
            self.descriptor,
            self.unit,
            self.generation.as_str(),
            self.schema,
            self.hash.as_str(),
            self.activation.as_str(),
            self.prefetch.as_str(),
        ];
        for (name, value) in attributes::METADATA.into_iter().zip(values) {
            html.attr(name, value);
        }
    }
    pub fn contents(&self, html: &mut Writer) {
        html.child(&self.initial);
        html.open("script");
        html.attr("type", "application/json");
        html.attr(attributes::PROPS, "");
        html.end_open();
        html.inert_json(&self.props);
        html.close("script");
    }
}

type ServerFactory = dyn Fn(&str, &mut Context<'_>) -> Result<Html>;
struct Registration {
    entry: Entry,
    render: Box<ServerFactory>,
}
#[derive(Default)]
pub struct Registry {
    entries: BTreeMap<String, Registration>,
}
impl Registry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register<D: Island, C: Render + 'static>(
        &mut self,
        make: impl Fn(D::Props) -> C + 'static,
    ) -> Result<()> {
        if C::FRAGMENT {
            return Err("island entries require one native root element; wrap the fragment in a single-root component".into());
        }
        if self.entries.contains_key(D::NAME) {
            return Err(format!("duplicate island {}", D::NAME).into());
        }
        self.entries.insert(
            D::NAME.into(),
            Registration {
                entry: Entry::new::<D>(C::TEMPLATE_HASH),
                render: Box::new(move |props, context| {
                    let props = fusor_islands::decode(props)?;
                    fusor::coherence::prepare_state(context.owner(), |_| make(props))
                        .render(context)
                }),
            },
        );
        Ok(())
    }
    pub fn witness(&self) -> UnitWitness {
        UnitWitness {
            version: fusor_islands::PROTOCOL_VERSION,
            entries: self
                .entries
                .values()
                .map(|registration| registration.entry.clone())
                .collect(),
        }
    }
    pub fn validate(&self, manifest: &DeliveryManifest) -> Result<()> {
        let mut seen = BTreeSet::new();
        for (unit_name, unit) in &manifest.units {
            for entry in &unit.entries {
                let registration = self.entries.get(&entry.descriptor).ok_or_else(|| {
                    Error::from(format!("no native registration for {}", entry.descriptor))
                })?;
                if &registration.entry.unit != unit_name
                    || registration.entry.props_schema != entry.props_schema
                    || registration.entry.mode != entry.mode
                    || entry.mode == RenderMode::Attach
                        && registration.entry.template_hash != entry.template_hash
                {
                    return Err(format!(
                        "native/browser registration mismatch: {}",
                        entry.descriptor
                    )
                    .into());
                }
                seen.insert(&entry.descriptor);
            }
        }
        if seen.len() != self.entries.len() {
            return Err("a native island has no browser delivery entry".into());
        }
        Ok(())
    }
}
