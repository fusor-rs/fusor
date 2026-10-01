use fusor::{JsInputs, Signal, signal};

const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

#[derive(JsInputs)]
pub struct ChartJs {
    #[js]
    values: Signal<Vec<f64>>,
    #[js]
    smooth: Signal<bool>,
    #[js]
    selected: Signal<Option<u32>>,
    status: Signal<String>,
}

impl ChartJs {
    pub fn new() -> Self {
        Self {
            values: signal(vec![42.0, 68.0, 55.0, 91.0, 74.0, 112.0, 96.0]),
            smooth: signal(true),
            selected: signal(None),
            status: signal("Loading Chart.js…".into()),
        }
    }

    fn boost(&self) {
        let index = self.selected.get().unwrap_or(3) as usize;
        self.values.update(|values| values[index] += 12.0);
    }

    fn next_week(&self) {
        self.values.update(|values| {
            values.rotate_left(1);
            for (index, value) in values.iter_mut().enumerate() {
                *value = (*value + if index % 2 == 0 { 9.0 } else { -5.0 }).max(12.0);
            }
        });
    }

    fn select(&self, event: web_sys::Event) {
        if let Ok(index) = fusor::js::event_detail::<u32>(&event) {
            if index < 7 {
                self.selected.set(Some(index));
            }
        }
    }

    fn report(&self, event: web_sys::Event) {
        if let Ok(message) = fusor::js::event_detail::<String>(&event) {
            self.status.set(message);
        }
    }

    fn selection(&self) -> String {
        self.selected.get().map_or_else(
            || "Pick a day in the chart".into(),
            |index| {
                format!(
                    "{} · {} visits",
                    DAYS[index as usize],
                    self.values.get()[index as usize]
                )
            },
        )
    }
}

fusor::template!("web/demos/chartjs.html");

pub struct ChartJsInputs {}

impl fusor::dom::FromInputs for ChartJs {
    type Inputs = ChartJsInputs;
    type Error = fusor::dom::JsValue;

    fn from_inputs(
        _inputs: Self::Inputs,
        _owner: fusor::OwnerHandle,
    ) -> Result<Self, wasm_bindgen::JsValue> {
        Ok(Self::new())
    }
}
