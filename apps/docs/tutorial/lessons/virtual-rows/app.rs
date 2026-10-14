use fusor::prelude::*;
use fusor_components::{Scroller, VirtualRows};

#[derive(Clone, PartialEq)]
struct Day {
    date: String,
    photos: Vec<String>,
}

const PHOTOS_PER_LINE: usize = 4;
const LINE_HEIGHT: f64 = 170.0;
const HEADING_HEIGHT: f64 = 40.0;
const OVERSCAN_ROWS: usize = 3;

struct App {
    days: Signal<Vec<Day>>,
    rows: VirtualRows<Day>,
}

impl App {
    fn new() -> Self {
        let days = signal(sample_days());
        let rows = VirtualRows::new(
            {
                let days = days.clone();
                move || days.get()
            },
            |day| HEADING_HEIGHT + LINE_HEIGHT * day.photos.len().div_ceil(PHOTOS_PER_LINE) as f64,
            Scroller::Page,
            OVERSCAN_ROWS,
        );
        Self { days, rows }
    }

    fn jump_to(&self, month: &str) {
        let first = self
            .days
            .with(|days| days.iter().position(|day| day.date.starts_with(month)));
        if let Some(index) = first {
            self.rows.scroll_to(index);
        }
    }
}

/// One row per day, each with one to twelve photos.
fn sample_days() -> Vec<Day> {
    (0..1000)
        .map(|day| Day {
            date: format!(
                "{:04}-{:02}-{:02}",
                2022 + day / 360,
                day % 360 / 30 + 1,
                day % 30 + 1
            ),
            photos: (0..day % 12 + 1)
                .map(|photo| format!("/photos/{day}-{photo}.jpg"))
                .collect(),
        })
        .collect()
}

fusor::template!("web/index.html");
