use crate::app::record;
use fusor::Registration;
use fusor::dom::JsValue;
use fusor::prelude::*;

struct CounterLifetime;
impl Drop for CounterLifetime {
    fn drop(&mut self) {
        record(1);
    }
}

pub struct Counter {
    count: Signal<i32>,
    clicks: Signal<i32>,
    initial: i32,
    label: &'static str,
    _cleanup: CounterLifetime,
    _activation: Registration,
}
pub struct CounterInputs {
    pub count: Signal<i32>,
    pub initial: i32,
    pub label: &'static str,
    pub fail: bool,
}
impl FromInputs for Counter {
    type Error = fusor::dom::JsValue;
    type Inputs = CounterInputs;
    fn from_inputs(inputs: Self::Inputs, owner: OwnerHandle) -> Result<Self, JsValue> {
        record(0);
        let cleanup = CounterLifetime;
        let activation = owner.on_activate(|| record(2));
        if inputs.fail {
            return Err(JsValue::from_str("expected construction failure"));
        }
        Ok(Self {
            count: inputs.count,
            clicks: signal(0),
            initial: inputs.initial,
            label: inputs.label,
            _cleanup: cleanup,
            _activation: activation,
        })
    }
}

pub struct Theme;
impl ContextKey for Theme {
    type Value = &'static str;
}

pub struct Panel {
    body: Content,
    title: &'static str,
}
pub struct PanelInputs {
    pub body: Content,
    pub title: &'static str,
}
impl FromInputs for Panel {
    type Error = fusor::dom::JsValue;
    type Inputs = PanelInputs;
    fn from_inputs(inputs: Self::Inputs, owner: OwnerHandle) -> Result<Self, JsValue> {
        owner
            .provide::<Theme>(inputs.title)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(Self {
            body: inputs.body,
            title: "receiver title",
        })
    }
}

pub struct Badge {
    theme: &'static str,
}
pub struct BadgeInputs;
impl FromInputs for Badge {
    type Error = &'static str;
    type Inputs = BadgeInputs;
    fn from_inputs(_: Self::Inputs, owner: OwnerHandle) -> Result<Self, Self::Error> {
        Ok(Self {
            theme: *owner
                .context::<Theme>()
                .ok_or("missing receiving context")?,
        })
    }
}

#[derive(FromInputs)]
pub struct Row {
    #[input]
    count: Signal<i32>,
}

#[derive(FromInputs)]
pub struct Cells {
    #[input]
    count: Signal<i32>,
}

#[derive(FromInputs)]
pub struct FragmentControls {
    #[input]
    pub visible: Signal<bool>,
    #[input]
    pub count: Signal<i32>,
}

#[derive(FromInputs)]
pub struct Empty;

fusor::template!("web/components/widgets.html");

#[derive(FromInputs)]
pub struct Button;
