//! Compiler-owned child fragments. Factories capture lexical state; placement
//! supplies the lifetime. The temporary staging element is never mounted.
use super::{JsValue, MountPoint, Scope, hydration};

pub type Children = crate::render::Children<Scope, JsValue>;

impl Scope {
    #[doc(hidden)]
    pub fn children_at(&mut self, target: &MountPoint, children: &Children) -> Result<(), JsValue> {
        let child = hydration::with_range(self.is_hydrating().then(|| target.clone()), || {
            children.prepare(&self.owner())
        })?;
        if let Some(child) = child {
            if !self.is_hydrating() {
                child.attach_fragment(target)?;
            }
            child.finish_prepare()?;
            child.try_commit()?;
            self.children.push(child);
        } else if self.is_hydrating() && !target.is_empty()? {
            return Err(JsValue::from_str(
                "server children differ from browser children",
            ));
        }
        Ok(())
    }
}
