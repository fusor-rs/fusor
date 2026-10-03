use serde::Deserialize;
use std::collections::BTreeMap;
/// Column order. A report measured under an earlier protocol lacks later
/// frameworks; their columns are hidden rather than shown as failures.
pub const FRAMEWORKS: [(&str, &str); 7] = [
    ("fusor", "fusor"),
    ("react", "React"),
    ("svelte", "Svelte"),
    ("solid", "SolidJS"),
    ("vue", "Vue"),
    ("preact", "Preact"),
    ("leptos", "Leptos"),
];

#[derive(Deserialize)]
pub struct Report {
    pub schema: u32,
    #[serde(rename = "generatedAt")]
    pub generated_at: String,
    pub environment: Environment,
    pub versions: BTreeMap<String, String>,
    pub results: Vec<ResultRow>,
    pub bundles: Vec<Bundle>,
    pub memory: Vec<MemoryRecord>,
    pub errors: Vec<serde_json::Value>,
}
#[derive(Deserialize)]
pub struct Environment {
    pub browser: String,
    pub os: String,
    pub cpu: String,
    pub samples: usize,
    pub warmups: usize,
}
#[derive(Deserialize)]
pub struct ResultRow {
    pub framework: String,
    pub id: String,
    pub label: String,
    pub category: String,
    pub status: String,
    pub median: Option<f64>,
    pub p95: Option<f64>,
    pub mutations: Option<Mutations>,
}
#[derive(Deserialize)]
pub struct Mutations {
    pub inserted: u32,
    pub removed: u32,
    pub moves: u32,
}
#[derive(Deserialize)]
pub struct Bundle {
    pub framework: String,
    pub fixture: String,
    pub gzip: usize,
    pub raw: usize,
}
#[derive(Clone, PartialEq)]
pub struct MetricRow {
    pub id: String,
    pub label: String,
    pub category: String,
    pub unit: &'static str,
    pub values: [Option<(f64, f64)>; 7],
    pub detail: String,
    pub operations: [Option<String>; 7],
    pub present: [bool; 7],
}
impl MetricRow {
    pub fn display(&self, index: usize, p95: bool) -> String {
        let Some((median, tail)) = self.values[index] else {
            return "—".into();
        };
        let value = if p95 { tail } else { median };
        if self.unit == "ms" && (value * 100.0).round() < 10.0 {
            "<0.10".into()
        } else {
            format!("{value:.2}")
        }
    }
    pub fn best(&self, index: usize, p95: bool) -> bool {
        if self.unit != "ms" {
            return false;
        }
        let values: Vec<f64> = self
            .values
            .iter()
            .filter_map(|value| value.map(|(median, tail)| if p95 { tail } else { median }))
            .collect();
        let Some((median, tail)) = self.values[index] else {
            return false;
        };
        let value = if p95 { tail } else { median };
        values
            .iter()
            .copied()
            .reduce(f64::min)
            .is_some_and(|min| (value - min).abs() < 0.05)
    }
}
impl Report {
    pub fn date(&self) -> &str {
        self.generated_at.get(..10).unwrap_or(&self.generated_at)
    }
    pub fn version(&self, index: usize) -> &str {
        let key = match FRAMEWORKS[index].0 {
            "solid" => "solid-js",
            framework => framework,
        };
        self.versions
            .get(key)
            .map(String::as_str)
            .unwrap_or("unknown")
    }
    /// Whether this report measured the framework in column `index`.
    pub fn includes(&self, index: usize) -> bool {
        self.results
            .iter()
            .any(|result| result.framework == FRAMEWORKS[index].0)
    }
    pub fn framework_count(&self) -> usize {
        (0..FRAMEWORKS.len())
            .filter(|index| self.includes(*index))
            .count()
    }
    pub fn rows(&self, category: &str) -> Vec<MetricRow> {
        let present = std::array::from_fn(|index| self.includes(index));
        let mut rows: Vec<MetricRow> = vec![];
        for result in &self.results {
            if category != "all" && category != result.category {
                continue;
            }
            let Some(index) = FRAMEWORKS
                .iter()
                .position(|(id, _)| *id == result.framework)
            else {
                continue;
            };
            let row = if let Some(index) = rows.iter().position(|row| row.id == result.id) {
                &mut rows[index]
            } else {
                rows.push(MetricRow {
                    id: result.id.clone(),
                    label: result.label.clone(),
                    category: result.category.clone(),
                    unit: "ms",
                    values: [None; 7],
                    detail: String::new(),
                    operations: std::array::from_fn(|_| None),
                    present,
                });
                rows.last_mut().expect("the metric row was just appended")
            };
            if result.status == "measured" {
                if let (Some(median), Some(p95)) = (result.median, result.p95) {
                    row.values[index] = Some((median, p95));
                }
            }
            if let Some(mutations) = &result.mutations {
                row.detail = "Retained row identities checked.".into();
                row.operations[index] = Some(match result.id.as_str() {
                    "insert" => format!("{} inserted", mutations.inserted),
                    "delete" => format!("{} removed", mutations.removed),
                    _ => format!("{} moves", mutations.moves),
                });
            }
        }
        if category == "all" || category == "bundles" {
            for fixture in ["hello", "todo", "benchmark workload"] {
                let name = match fixture {
                    "hello" => "Hello World",
                    "todo" => "Task application",
                    _ => "Benchmark application",
                };
                let mut row = MetricRow {
                    id: format!("bundle-{fixture}"), label: format!("{name} · gzip"),
                    category: "bundles".into(), unit: "KiB", values: [None; 7],
                    operations: std::array::from_fn(|_| None),
                    present,
                    detail: "Sum of separately gzipped HTML, JavaScript, and Wasm files. No CDN or shared warm cache.".into(),
                };
                for bundle in self
                    .bundles
                    .iter()
                    .filter(|bundle| bundle.fixture == fixture)
                {
                    if let Some(index) = FRAMEWORKS
                        .iter()
                        .position(|(id, _)| *id == bundle.framework)
                    {
                        let kib = bundle.gzip as f64 / 1024.0;
                        row.values[index] = Some((kib, kib));
                    }
                }
                rows.push(row);
            }
        }
        rows
    }
    pub fn measured(&self) -> usize {
        self.results
            .iter()
            .filter(|result| result.status == "measured")
            .count()
    }
    pub fn memory_rows(&self) -> Vec<MemoryRow> {
        FRAMEWORKS
            .iter()
            .filter_map(|(id, name)| {
                self.memory
                    .iter()
                    .find(|record| record.framework == *id)
                    .map(|record| {
                        let baseline = record.baseline.heap.used_size;
                        let last = record.cycles.last();
                        let last_nodes =
                            last.map_or(record.baseline.dom.nodes, |value| value.dom.nodes);
                        MemoryRow {
                            framework: (*name).into(),
                            ten: record
                                .live
                                .iter()
                                .find(|point| point.n == 10000)
                                .map_or("—".into(), |point| mib(point.heap.used_size - baseline)),
                            hundred: record
                                .live
                                .iter()
                                .find(|point| point.n == 100000)
                                .map_or("—".into(), |point| mib(point.heap.used_size - baseline)),
                            retained: last
                                .map_or("—".into(), |point| mib(point.heap.used_size - baseline)),
                            wasm: last.map_or("—".into(), |point| {
                                if point.wasm_capacity == 0.0 {
                                    "—".into()
                                } else {
                                    mib(point.wasm_capacity)
                                }
                            }),
                            nodes: format!("{:+}", last_nodes - record.baseline.dom.nodes),
                        }
                    })
            })
            .collect()
    }
    pub fn total_raw(&self) -> usize {
        self.bundles.iter().map(|bundle| bundle.raw).sum()
    }
}
fn mib(bytes: f64) -> String {
    format!("{:.2}", bytes / (1024.0 * 1024.0))
}
#[derive(Deserialize)]
pub struct MemoryRecord {
    framework: String,
    baseline: MemoryPoint,
    live: Vec<MemoryPoint>,
    cycles: Vec<MemoryPoint>,
}
#[derive(Deserialize)]
pub struct MemoryPoint {
    #[serde(default)]
    n: usize,
    heap: Heap,
    dom: Dom,
    #[serde(rename = "wasmCapacity")]
    wasm_capacity: f64,
}
#[derive(Deserialize)]
pub struct Heap {
    #[serde(rename = "usedSize")]
    used_size: f64,
}
#[derive(Deserialize)]
pub struct Dom {
    nodes: i64,
}
#[derive(Clone, PartialEq)]
pub struct MemoryRow {
    pub framework: String,
    pub ten: String,
    pub hundred: String,
    pub retained: String,
    pub wasm: String,
    pub nodes: String,
}
