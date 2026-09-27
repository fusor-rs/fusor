use fusor::{Signal, signal};

struct App {
    name: Signal<String>,
    seats: Signal<u32>,
    volume: Signal<f64>,
    newsletter: Signal<bool>,
    toppings: Signal<Vec<String>>,
    size: Signal<String>,
    delivery: Signal<String>,
    days: Signal<Vec<String>>,
    notes: Signal<String>,
}

impl App {
    fn new() -> Self {
        Self {
            name: signal("Ada".into()),
            seats: signal(2),
            volume: signal(0.5),
            newsletter: signal(false),
            toppings: signal(Vec::new()),
            size: signal("small".into()),
            delivery: signal("standard".into()),
            days: signal(vec!["Tue".into()]),
            notes: signal(String::new()),
        }
    }
}

fusor::template!("web/index.html");
