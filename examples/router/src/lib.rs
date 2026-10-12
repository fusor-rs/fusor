mod dashboard;
mod memory;
use dashboard::Dashboard;
use dashboard::Team;
use fusor::FromInputs;
use memory::Memory;
use wasm_bindgen::prelude::*;
struct App {
    selected: fusor::Signal<String>,
}
#[derive(FromInputs)]
struct Home;
#[derive(FromInputs)]
struct NotFound;
#[derive(FromInputs)]
struct Article {
    #[input]
    slug: String,
}
#[wasm_bindgen]
pub fn stop() -> Result<(), JsValue> {
    fusor::dom::application::unmount()
}
fusor::template!("web/index.html");

#[wasm_bindgen]
pub fn navigate(url: &str, replace: bool) -> Result<(), JsValue> {
    use fusor_router::browser::{NavigateOptions, declarative::Navigation};
    let owner = fusor::dom::application::owner().ok_or_else(|| JsValue::from_str("unmounted"))?;
    Navigation::from_owner(&owner)
        .ok_or_else(|| JsValue::from_str("missing router"))?
        .navigate_url(
            url,
            NavigateOptions {
                replace,
                ..Default::default()
            },
        )
}
#[derive(FromInputs)]
struct Broken;
impl fusor::dom::Component for Broken {
    fn mount(self) -> Result<fusor::dom::Scope, JsValue> {
        Self::try_mount_with(|_| Ok(self))
    }
    fn prepare_component(
        parent: Option<&fusor::OwnerHandle>,
        make: fusor::dom::ComponentFactory<'_, Self>,
    ) -> Result<fusor::dom::Scope, JsValue> {
        use fusor::dom::{Scope, document};
        let mut scope = Scope::new(document()?.create_element("div")?);
        scope.prepare_owner(parent);
        let _ = make(scope.owner())?;
        scope.before_commit(|| Err(JsValue::from_str("expected page setup failure")))?;
        Ok(scope)
    }
}

impl fusor::dom::TemplateComponent for Broken {}

impl Default for App {
    fn default() -> Self {
        Self {
            selected: fusor::signal(String::new()),
        }
    }
}

struct BranchRouters {
    signed_in: fusor::Signal<bool>,
    fail: fusor::Signal<bool>,
}

impl BranchRouters {
    fn new() -> Self {
        Self {
            signed_in: fusor::signal(false),
            fail: fusor::signal(false),
        }
    }
}

fusor::template!("web/components/branch-routers.html");

#[wasm_bindgen]
pub fn start_branch_routers() -> Result<(), JsValue> {
    use fusor::dom::{Component, document};
    fusor::dom::application::mount_scope(|| {
        let mut scope =
            BranchRouters::prepare_component(None, Box::new(|_| Ok(BranchRouters::new())))?;
        let main = document()?
            .query_selector("main")?
            .ok_or_else(|| JsValue::from_str("missing application container"))?;
        scope.attach(&main)?;
        Ok(scope)
    })
}

#[wasm_bindgen]
pub fn probe_competing_router() -> Result<(), JsValue> {
    use fusor::dom::Component;
    let _scope = BranchRouters::mount_with(|_| BranchRouters::new())?;
    Ok(())
}
