mod report;
use fusor::{OwnerHandle, Signal, signal};
use fusor_async::{AsyncBoundary, AsyncValue, browser, fetch};
use report::{MemoryRow, MetricRow, Report};
struct App {
    view: AsyncBoundary,
    report: AsyncValue<(), Report, String>,
    category: Signal<String>,
    percentile: Signal<bool>,
}
impl App {
    fn new(owner: OwnerHandle) -> Self {
        Self {
            view: AsyncBoundary::coherent(),
            report: browser::read(
                &owner,
                || (),
                |(), cancel| async move {
                    let text = fetch::get_text("/benchmarks/results.json", &cancel)
                        .await
                        .map_err(|error| error.to_string())?;
                    let report: Report =
                        serde_json::from_str(&text).map_err(|error| error.to_string())?;
                    // 1: the six-framework protocol; 2: versioned protocols.
                    if !matches!(report.schema, 1 | 2) {
                        return Err("Unsupported report version".into());
                    }
                    Ok(report)
                },
            ),
            category: signal("all".into()),
            percentile: signal(false),
        }
    }
    fn status(&self) -> String {
        use fusor_async::BoundaryStatus;
        match self.view.status() {
            BoundaryStatus::Error(error) | BoundaryStatus::Faulted(error) => {
                format!("The report could not load: {error}")
            }
            BoundaryStatus::Ready => String::new(),
            _ => "Loading the recorded measurements…".into(),
        }
    }
}
#[derive(fusor::FromInputs)]
struct Row {
    #[input]
    item: fusor::Memo<MetricRow>,
    #[input]
    percentile: Signal<bool>,
}
impl Row {
    fn value(&self, index: usize) -> String {
        self.item.get().display(index, self.percentile.get())
    }
    fn operation(&self, index: usize) -> String {
        self.item
            .with(|row| row.operations[index].clone().unwrap_or_default())
    }
    fn best(&self, index: usize) -> bool {
        self.item.get().best(index, self.percentile.get())
    }
    fn present(&self, index: usize) -> bool {
        self.item.with(|row| row.present[index])
    }
}
#[derive(fusor::FromInputs)]
struct Memory {
    #[input]
    item: fusor::Memo<MemoryRow>,
}
fusor::bindings!(app);
include!(env!("FUSOR_MODULE"));
