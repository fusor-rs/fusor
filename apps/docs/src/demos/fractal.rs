//! A Mandelbrot picture painted by every thread in a pool. One streaming task
//! hands tiles to the pool’s compute threads and sends each finished tile back
//! as soon as it is ready; zooming cancels the stream mid-picture.
use fusor::{OwnerHandle, Signal, signal};
use fusor_async::CancellationSource;
use fusor_worker::{
    ComputeContext, JobError, NoError, Pool, ResultStream, StreamSender, TaskContext, TaskResult,
    WorkerError,
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use gloo_timers::future::TimeoutFuture;
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::atomic::{AtomicU8, Ordering},
};
use wasm_bindgen::JsCast;

const WIDTH: usize = 768;
const HEIGHT: usize = 480;
const TILE: usize = 48;
const ACROSS: usize = WIDTH / TILE;
const TILES: usize = ACROSS * (HEIGHT / TILE);
/// Each pixel averages a 2 × 2 grid of samples, for smoother edges.
const SAMPLES: usize = 2;
const BLANK: &str =
    "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

/// Where the picture looks, and how many threads may paint it at once.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    /// Width of the view on the complex plane.
    pub span: f64,
    pub iterations: u32,
    pub painters: u8,
}

/// One painted tile: a small BMP image, and which thread painted it.
#[derive(Serialize, Deserialize)]
pub struct Tile {
    pub index: u16,
    pub painter: u8,
    pub image: String,
}

// ---- Worker code.

/// Streams tiles back as the pool’s compute threads finish them, keeping at
/// most `frame.painters` tiles in progress at once.
#[fusor_worker::task(pool, stream)]
pub async fn paint(
    frame: Frame,
    ctx: TaskContext,
    mut tiles: StreamSender<Tile>,
) -> TaskResult<()> {
    let mut waiting = center_out().into_iter();
    let mut painting = FuturesUnordered::new();
    loop {
        while painting.len() < usize::from(frame.painters.max(1)) {
            let Some(index) = waiting.next() else { break };
            painting.push(ctx.compute(move |cpu: ComputeContext| {
                render(&frame, index, &|| cpu.check_cancelled())
            }));
        }
        let Some(tile) = painting.next().await else {
            return Ok(());
        };
        // The outer `?` is the pool’s error, the inner one the tile’s.
        tiles.send(tile??).await?;
    }
}

/// The same picture on one ordinary worker, for pages without shared memory.
#[fusor_worker::task(stream)]
pub async fn paint_alone(frame: Frame, mut tiles: StreamSender<Tile>) -> TaskResult<()> {
    for index in center_out() {
        tiles.send(render(&frame, index, &|| Ok(()))?).await?;
        // Give a cancelled stream’s message a chance to arrive.
        TimeoutFuture::new(0).await;
    }
    Ok(())
}

/// Tiles nearest the middle first, so the picture grows outwards.
fn center_out() -> Vec<u16> {
    let mut order: Vec<u16> = (0..TILES as u16).collect();
    let distance = |index: &u16| {
        let (column, row) = (usize::from(*index) % ACROSS, usize::from(*index) / ACROSS);
        let dx = (column * TILE + TILE / 2) as f64 - WIDTH as f64 / 2.0;
        let dy = (row * TILE + TILE / 2) as f64 - HEIGHT as f64 / 2.0;
        (dx * dx + dy * dy) as u64
    };
    order.sort_by_key(distance);
    order
}

fn render(
    frame: &Frame,
    index: u16,
    check: &dyn Fn() -> Result<(), WorkerError>,
) -> Result<Tile, WorkerError> {
    let step = frame.span / WIDTH as f64;
    let left = frame.x - frame.span / 2.0;
    let top = frame.y + step * HEIGHT as f64 / 2.0;
    let (column, row) = (usize::from(index) % ACROSS, usize::from(index) / ACROSS);
    let mut pixels = Vec::with_capacity(TILE * TILE * 3);
    for y in 0..TILE {
        check()?;
        for x in 0..TILE {
            let mut color = [0.0; 3];
            for sample in 0..SAMPLES * SAMPLES {
                let fx =
                    (column * TILE + x) as f64 + ((sample % SAMPLES) as f64 + 0.5) / SAMPLES as f64;
                let fy =
                    (row * TILE + y) as f64 + ((sample / SAMPLES) as f64 + 0.5) / SAMPLES as f64;
                let shade = shade(escape(left + fx * step, top - fy * step, frame.iterations));
                for (total, part) in color.iter_mut().zip(shade) {
                    *total += part;
                }
            }
            // BMP stores blue, green, red.
            for part in color.iter().rev() {
                pixels.push((part / (SAMPLES * SAMPLES) as f64 * 255.0) as u8);
            }
        }
    }
    Ok(Tile {
        index,
        painter: painter(),
        image: bmp(&pixels),
    })
}

/// How quickly a point escapes, smoothed so colors blend between bands.
fn escape(cx: f64, cy: f64, limit: u32) -> Option<f64> {
    let (mut x, mut y) = (0.0f64, 0.0f64);
    for iteration in 0..limit {
        let (xx, yy) = (x * x, y * y);
        if xx + yy > 256.0 {
            let log_z = (xx + yy).ln() / 2.0;
            return Some(
                f64::from(iteration) + 1.0
                    - (log_z / std::f64::consts::LN_2).ln() / std::f64::consts::LN_2,
            );
        }
        y = 2.0 * x * y + cy;
        x = xx - yy + cx;
    }
    None
}

fn shade(escaped: Option<f64>) -> [f64; 3] {
    let Some(value) = escaped else {
        return [0.03, 0.04, 0.09];
    };
    let t = value.sqrt() * 0.18;
    let wave = |offset: f64| 0.5 + 0.5 * (std::f64::consts::TAU * (t + offset)).cos();
    [wave(0.0), wave(0.12), wave(0.28)].map(|part| part * part.sqrt())
}

/// Numbers compute threads in the order they first paint.
fn painter() -> u8 {
    static NEXT: AtomicU8 = AtomicU8::new(0);
    thread_local!(static PAINTER: u8 = NEXT.fetch_add(1, Ordering::Relaxed));
    PAINTER.with(|painter| *painter)
}

/// A tile as a data URL: a 24-bit BMP, stored top row first.
fn bmp(pixels: &[u8]) -> String {
    let mut file = Vec::with_capacity(54 + pixels.len());
    file.extend_from_slice(b"BM");
    for value in [54 + pixels.len() as u32, 0, 54, 40, TILE as u32] {
        file.extend_from_slice(&value.to_le_bytes());
    }
    file.extend_from_slice(&(-(TILE as i32)).to_le_bytes());
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&24u16.to_le_bytes());
    for value in [0, pixels.len() as u32, 2835, 2835, 0, 0] {
        file.extend_from_slice(&value.to_le_bytes());
    }
    file.extend_from_slice(pixels);
    format!("data:image/bmp;base64,{}", base64(&file))
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = chunk.iter().enumerate().fold(0u32, |value, (i, &byte)| {
            value | u32::from(byte) << (16 - 8 * i)
        });
        for position in 0..4 {
            out.push(if position <= chunk.len() {
                char::from(ALPHABET[(value >> (18 - 6 * position) & 63) as usize])
            } else {
                '='
            });
        }
    }
    out
}

// ---- Page code.

#[derive(Clone, Copy, PartialEq)]
struct View {
    x: f64,
    y: f64,
    span: f64,
}

const HOME: View = View {
    x: -0.65,
    y: 0.0,
    span: 3.4,
};
const PLACES: [(&str, View); 3] = [
    (
        "Seahorse valley",
        View {
            x: -0.7453,
            y: 0.1127,
            span: 0.0065,
        },
    ),
    (
        "Elephant valley",
        View {
            x: 0.2821,
            y: 0.0101,
            span: 0.016,
        },
    ),
    (
        "Deep spiral",
        View {
            x: -0.743_643_887_037_151,
            y: 0.131_825_904_205_33,
            span: 0.000_014,
        },
    ),
];

#[derive(Clone, PartialEq)]
enum Mode {
    Starting,
    Pool,
    /// No shared memory, so there is no pool: one ordinary worker paints.
    Alone(String),
}

#[derive(Clone, Default, PartialEq)]
struct TileView {
    image: String,
    painter: u8,
    fresh: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct Race {
    one: f64,
    all: f64,
    threads: usize,
}

#[derive(Clone, PartialEq)]
struct PainterCount {
    painter: usize,
    tiles: u32,
    style: String,
}

#[derive(Default)]
struct Painting {
    generation: u32,
    stop: Option<CancellationSource>,
}

#[derive(Clone)]
pub struct Fractal {
    owner: OwnerHandle,
    pool: Rc<RefCell<Option<Pool>>>,
    mode: Signal<Mode>,
    threads: Signal<usize>,
    painters: Signal<usize>,
    view: Signal<View>,
    tiles: Rc<Vec<Signal<TileView>>>,
    painted: Signal<u32>,
    counts: Signal<Vec<u32>>,
    last: Signal<Option<(f64, usize)>>,
    race: Signal<Option<Race>>,
    racing: Signal<bool>,
    stopped: Signal<u32>,
    show_painters: Signal<bool>,
    failure: Signal<Option<String>>,
    painting: Rc<RefCell<Painting>>,
}

impl Fractal {
    pub fn new(owner: OwnerHandle) -> Self {
        let fractal = Self {
            owner,
            pool: Rc::new(RefCell::new(None)),
            mode: signal(Mode::Starting),
            threads: signal(1),
            painters: signal(1),
            view: signal(HOME),
            tiles: Rc::new((0..TILES).map(|_| signal(TileView::default())).collect()),
            painted: signal(0),
            counts: signal(Vec::new()),
            last: signal(None),
            race: signal(None),
            racing: signal(false),
            stopped: signal(0),
            show_painters: signal(false),
            failure: signal(None),
            painting: Rc::new(RefCell::new(Painting::default())),
        };
        let this = fractal.clone();
        wasm_bindgen_futures::spawn_local(async move { this.start().await });
        fractal
    }

    /// Starts one pool thread per core, or explains why there can’t be a pool.
    async fn start(&self) {
        let support = fusor_worker::capabilities();
        if !support.shared_memory {
            self.mode.set(Mode::Alone(
                "This page isn’t cross-origin isolated, so the browser offers no shared memory \
                 and there is no pool. One ordinary worker paints instead."
                    .into(),
            ));
            self.paint_now();
            return;
        }
        let wanted = support.hardware_parallelism.clamp(1, 16);
        match Pool::new(&self.owner).max_threads(wanted).await {
            Ok(pool) => {
                let threads = pool.threads();
                *self.pool.borrow_mut() = Some(pool);
                self.threads.set(threads);
                self.painters.set(threads);
                self.mode.set(Mode::Pool);
                self.paint_now();
            }
            Err(error) => {
                self.mode.set(Mode::Alone(format!(
                    "The pool couldn’t start ({error}). One ordinary worker paints instead."
                )));
                self.paint_now();
            }
        }
    }

    fn paint_now(&self) {
        let this = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            this.paint(this.painters.get_untracked()).await;
        });
    }

    /// Paints the current view, and returns how long it took if it finished.
    async fn paint(&self, painters: usize) -> Option<f64> {
        let (generation, token) = {
            let mut painting = self.painting.borrow_mut();
            painting.generation += 1;
            // Replacing the source cancels the previous picture’s stream.
            let source = CancellationSource::default();
            let token = source.token();
            painting.stop = Some(source);
            (painting.generation, token)
        };
        let unfinished = TILES as u32 - self.painted.get_untracked();
        if self.last.get_untracked().is_some() || unfinished < TILES as u32 {
            self.stopped.set(unfinished);
        }
        self.painted.set(0);
        self.counts.set(vec![0; self.threads.get_untracked()]);
        for tile in self.tiles.iter() {
            tile.update(|tile| tile.fresh = false);
        }
        let view = self.view.get_untracked();
        let frame = Frame {
            x: view.x,
            y: view.y,
            span: view.span,
            iterations: (700.0 + 160.0 * (HOME.span / view.span).log2().max(0.0)) as u32,
            painters: painters as u8,
        };
        let started = js_sys::Date::now();
        let pooled = self
            .pool
            .borrow()
            .as_ref()
            .map(|pool| paint::stream(&self.owner, frame).cancel_on(&token).on(pool));
        let finished = match pooled {
            Some(tiles) => self.receive(tiles, generation).await,
            None => {
                self.receive(
                    paint_alone::stream(&self.owner, frame).cancel_on(&token),
                    generation,
                )
                .await
            }
        };
        if !finished {
            return None;
        }
        let took = js_sys::Date::now() - started;
        self.last.set(Some((took, painters)));
        Some(took)
    }

    async fn receive<M>(
        &self,
        mut tiles: ResultStream<Tile, NoError, (), M>,
        generation: u32,
    ) -> bool {
        while let Some(tile) = tiles.next().await {
            if self.painting.borrow().generation != generation {
                return false;
            }
            match tile {
                Ok(tile) => self.show(tile),
                Err(JobError::Worker(WorkerError::Cancelled | WorkerError::OwnerDisposed)) => {
                    return false;
                }
                Err(error) => {
                    self.failure.set(Some(error.to_string()));
                    return false;
                }
            }
        }
        self.painting.borrow().generation == generation
    }

    fn show(&self, tile: Tile) {
        self.tiles[usize::from(tile.index)].set(TileView {
            image: tile.image,
            painter: tile.painter,
            fresh: true,
        });
        self.painted.update(|count| *count += 1);
        self.counts.update(|counts| {
            let painter = usize::from(tile.painter);
            if counts.len() <= painter {
                counts.resize(painter + 1, 0);
            }
            counts[painter] += 1;
        });
    }

    /// Paints with one thread, then with all of them, and compares.
    fn race(&self) {
        let this = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            this.race.set(None);
            this.racing.set(true);
            let threads = this.threads.get_untracked();
            this.painters.set(1);
            let one = this.paint(1).await;
            this.painters.set(threads);
            let all = match one {
                Some(_) => this.paint(threads).await,
                None => None,
            };
            this.racing.set(false);
            if let (Some(one), Some(all)) = (one, all) {
                this.race.set(Some(Race { one, all, threads }));
            }
        });
    }

    fn choose_painters(&self, painters: usize) {
        self.painters.set(painters);
        self.paint_now();
    }

    fn go(&self, view: View) {
        self.view.set(view);
        self.paint_now();
    }

    fn zoom_in(&self) {
        let view = self.view.get();
        self.go(View {
            span: view.span / 3.0,
            ..view
        });
    }

    fn zoom_out(&self) {
        let view = self.view.get();
        self.go(View {
            span: (view.span * 3.0).min(HOME.span),
            ..view
        });
    }

    fn visit(&self, place: usize) {
        self.go(PLACES[place].1);
    }

    /// Zooms in three times around the point that was clicked.
    fn zoom(&self, index: usize, event: web_sys::Event) {
        let Some(click) = event.dyn_ref::<web_sys::MouseEvent>() else {
            return;
        };
        let Some(target) = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let size = f64::from(target.client_width().max(1));
        let (column, row) = (index % ACROSS, index / ACROSS);
        let px = (column * TILE) as f64 + f64::from(click.offset_x()) / size * TILE as f64;
        let py = (row * TILE) as f64 + f64::from(click.offset_y()) / size * TILE as f64;
        let view = self.view.get();
        let step = view.span / WIDTH as f64;
        self.go(View {
            x: view.x - view.span / 2.0 + px * step,
            y: view.y + step * HEIGHT as f64 / 2.0 - py * step,
            span: view.span / 3.0,
        });
    }

    fn tile_ids(&self) -> Vec<usize> {
        (0..TILES).collect()
    }

    fn tile_image(&self, index: usize) -> String {
        self.tiles[index].with(|tile| {
            if tile.image.is_empty() {
                BLANK.to_owned()
            } else {
                tile.image.clone()
            }
        })
    }

    fn tile_style(&self, index: usize) -> String {
        self.tiles[index].with(|tile| format!("--hue: {}", hue(usize::from(tile.painter))))
    }

    fn tile_stale(&self, index: usize) -> bool {
        self.tiles[index].with(|tile| !tile.fresh)
    }

    fn painter_choices(&self) -> Vec<usize> {
        let threads = self.threads.get();
        let mut choices: Vec<usize> = [1, 2, 4, 8].into_iter().filter(|&n| n < threads).collect();
        choices.push(threads);
        choices
    }

    fn painter_counts(&self) -> Vec<PainterCount> {
        self.counts
            .get()
            .into_iter()
            .enumerate()
            .filter(|(_, tiles)| *tiles > 0)
            .map(|(painter, tiles)| PainterCount {
                painter,
                tiles,
                style: format!("--hue: {}", hue(painter)),
            })
            .collect()
    }

    fn pooled(&self) -> bool {
        self.mode.get() == Mode::Pool
    }

    fn notice(&self) -> String {
        match self.mode.get() {
            Mode::Alone(reason) => reason,
            _ => String::new(),
        }
    }

    fn status(&self) -> String {
        if let Some(error) = self.failure.get() {
            return format!("Painting stopped: {error}");
        }
        let threads = match self.mode.get() {
            Mode::Starting => return "Starting a pool of threads…".into(),
            Mode::Pool => self.painters.get(),
            Mode::Alone(_) => 1,
        };
        let painted = self.painted.get();
        let who = if threads == 1 {
            "1 thread".to_owned()
        } else {
            format!("{threads} threads")
        };
        match self.last.get() {
            Some((took, painters)) if painted == TILES as u32 => {
                let who = if painters == 1 {
                    "1 thread".to_owned()
                } else {
                    format!("{painters} threads")
                };
                format!("Painted {TILES} tiles in {took:.0} ms with {who}")
            }
            _ => format!("Painting with {who} · {painted} of {TILES} tiles"),
        }
    }

    fn race_result(&self) -> String {
        self.race
            .get()
            .map(|race| {
                format!(
                    "1 thread: {:.0} ms · {} threads: {:.0} ms · {:.1}× faster",
                    race.one,
                    race.threads,
                    race.all,
                    race.one / race.all.max(1.0)
                )
            })
            .unwrap_or_default()
    }

    fn stopped_label(&self) -> String {
        match self.stopped.get() {
            0 => String::new(),
            1 => "Zooming stopped 1 tile of the previous view mid-paint.".into(),
            count => format!("Zooming stopped {count} tiles of the previous view mid-paint."),
        }
    }

    fn depth(&self) -> String {
        let zoom = HOME.span / self.view.get().span;
        if zoom < 1.5 {
            String::new()
        } else {
            format!("{zoom:.0}× zoom")
        }
    }

    fn places(&self) -> Vec<(usize, &'static str)> {
        PLACES
            .iter()
            .enumerate()
            .map(|(index, (name, _))| (index, *name))
            .collect()
    }
}

/// Well-separated colors for painters, one per thread.
fn hue(painter: usize) -> usize {
    (painter * 137 + 20) % 360
}

fusor::template!("web/demos/fractal.html");
