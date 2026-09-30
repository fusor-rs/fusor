//! Four in a row against an engine that searches in a worker. The rules are
//! one Rust module that both the page and the worker use.
use fusor::{OwnerHandle, Registration, Signal, signal};
use fusor_worker::{CancellationHandle, JobError, TaskContext, TaskResult, Worker, WorkerError};
use gloo_timers::{callback::Interval, future::TimeoutFuture};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, rc::Rc};

const COLUMNS: usize = 7;
const ROWS: usize = 6;
const CELLS: usize = COLUMNS * ROWS;
/// Any score this high is a forced win; sooner wins score higher.
const WIN: i32 = 100_000;
const INFINITY: i32 = 1_000_000;
const CENTER_FIRST: [usize; COLUMNS] = [3, 2, 4, 1, 5, 0, 6];
const TABLE_SIZE: usize = 1 << 20;

// ---- The rules, shared by the page and the worker.

/// A board as two bitboards, as in Pascal Pons’ Connect Four solver. Bit
/// `column * 7 + row` is set in `mask` when that cell holds a disc, and in
/// `current` when the disc belongs to the player about to move.
#[derive(Clone, Copy, Default)]
pub struct Position {
    current: u64,
    mask: u64,
    moves: u32,
}

impl Position {
    pub fn from_moves(moves: &[u8]) -> Self {
        let mut position = Self::default();
        for &column in moves {
            position.play(usize::from(column));
        }
        position
    }

    fn bottom(column: usize) -> u64 {
        1 << (column * (ROWS + 1))
    }

    fn top(column: usize) -> u64 {
        1 << (ROWS - 1 + column * (ROWS + 1))
    }

    fn column(column: usize) -> u64 {
        ((1 << ROWS) - 1) << (column * (ROWS + 1))
    }

    pub fn can_play(&self, column: usize) -> bool {
        self.mask & Self::top(column) == 0
    }

    pub fn play(&mut self, column: usize) {
        self.current ^= self.mask;
        self.mask |= self.mask + Self::bottom(column);
        self.moves += 1;
    }

    fn wins_with(&self, column: usize) -> bool {
        line(self.current | ((self.mask + Self::bottom(column)) & Self::column(column))).is_some()
    }

    /// The four cells of a line that the last player to move completed.
    pub fn winning_line(&self) -> Option<u64> {
        line(self.mask ^ self.current)
    }

    pub fn is_full(&self) -> bool {
        self.moves as usize == CELLS
    }

    /// Unique for each position, and never zero, so zero marks an empty slot.
    fn key(&self) -> u64 {
        self.current + self.mask + 1
    }
}

fn line(stones: u64) -> Option<u64> {
    // Vertical, horizontal and both diagonals. The empty seventh row of
    // each column stops lines from wrapping around the board.
    for shift in [1, 7, 6, 8] {
        let starts = stones & (stones >> shift) & (stones >> (2 * shift)) & (stones >> (3 * shift));
        if starts != 0 {
            let start = starts & starts.wrapping_neg();
            return Some(start | start << shift | start << (2 * shift) | start << (3 * shift));
        }
    }
    None
}

/// Every run of four cells a line could use: 69 on a standard board.
fn windows() -> Vec<u64> {
    let mut windows = Vec::new();
    for column in 0..COLUMNS as i32 {
        for row in 0..ROWS as i32 {
            for (across, up) in [(1, 0), (0, 1), (1, 1), (1, -1)] {
                let cells: Option<u64> = (0..4)
                    .map(|step| {
                        let (c, r) = (column + across * step, row + up * step);
                        ((0..COLUMNS as i32).contains(&c) && (0..ROWS as i32).contains(&r))
                            .then(|| 1u64 << (c as usize * (ROWS + 1) + r as usize))
                    })
                    .sum();
                windows.extend(cells);
            }
        }
    }
    windows
}

// ---- Messages between the page and the engine.

#[derive(Serialize, Deserialize)]
pub struct Request {
    pub moves: Vec<u8>,
    pub budget_ms: u32,
}

/// How the engine currently sees the position. Sent as progress while it
/// thinks, and once more as its final answer.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Thought {
    /// How many moves ahead the last complete search looked.
    pub depth: u32,
    /// How each column scores for the engine, where it has an opinion.
    pub columns: Vec<Option<i32>>,
    pub best: Option<u8>,
    pub positions: u64,
    pub elapsed_ms: u32,
    /// Positions the engine remembers from all its searches so far.
    pub remembered: u32,
}

// ---- Worker code: an engine that keeps its memory between moves.

#[derive(Clone, Copy, Default)]
struct Entry {
    key: u64,
    score: i32,
    depth: u8,
    bound: Bound,
    best: u8,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Bound {
    #[default]
    Exact,
    Lower,
    Upper,
}

pub struct Engine {
    table: Vec<Entry>,
    remembered: u32,
    windows: Vec<u64>,
    positions: u64,
    deadline: f64,
    stopped: bool,
}

#[fusor_worker::worker]
impl Engine {
    pub fn new(_: ()) -> TaskResult<Self> {
        Ok(Self {
            table: vec![Entry::default(); TABLE_SIZE],
            remembered: 0,
            windows: windows(),
            positions: 0,
            deadline: 0.0,
            stopped: false,
        })
    }

    /// Searches one move deeper at a time until the time budget runs out,
    /// reporting how every column looks after each column it finishes.
    pub async fn think(
        &mut self,
        request: Request,
        ctx: TaskContext<Thought>,
    ) -> TaskResult<Thought> {
        let root = Position::from_moves(&request.moves);
        let started = js_sys::Date::now();
        self.deadline = started + f64::from(request.budget_ms);
        self.positions = 0;
        self.stopped = false;
        let mut thought = Thought {
            columns: vec![None; COLUMNS],
            ..Thought::default()
        };
        for depth in 1..=(CELLS as u32 - root.moves) {
            let mut columns = thought.columns.clone();
            for column in self.root_order(&root, &thought) {
                let score = if root.wins_with(column) {
                    WIN - root.moves as i32
                } else {
                    let mut next = root;
                    next.play(column);
                    -self.negamax(next, depth - 1, -INFINITY, INFINITY)
                };
                if self.stopped {
                    break;
                }
                columns[column] = Some(score);
                ctx.report(Thought {
                    columns: columns.clone(),
                    positions: self.positions,
                    elapsed_ms: (js_sys::Date::now() - started) as u32,
                    remembered: self.remembered,
                    ..thought.clone()
                });
            }
            if self.stopped {
                break;
            }
            thought.depth = depth;
            thought.best = (0..COLUMNS)
                .filter(|&column| columns[column].is_some())
                .max_by_key(|&column| (columns[column], -(column as i32 - 3).abs()))
                .map(|column| column as u8);
            thought.columns = columns;
            thought.positions = self.positions;
            thought.elapsed_ms = (js_sys::Date::now() - started) as u32;
            thought.remembered = self.remembered;
            ctx.report(thought.clone());
            let decided = thought
                .columns
                .iter()
                .flatten()
                .all(|score| score.abs() > WIN - 100);
            if decided || js_sys::Date::now() > self.deadline {
                break;
            }
            // An ordinary worker only hears about a cancelled call when it
            // gets a turn, so give it one between depths.
            TimeoutFuture::new(0).await;
            ctx.check_cancelled()?;
        }
        Ok(thought)
    }
}

impl Engine {
    /// Playable columns, the most promising first.
    fn root_order(&self, root: &Position, thought: &Thought) -> Vec<usize> {
        let mut order: Vec<usize> = CENTER_FIRST
            .into_iter()
            .filter(|&column| root.can_play(column))
            .collect();
        order
            .sort_by_key(|&column| std::cmp::Reverse(thought.columns[column].unwrap_or(-INFINITY)));
        order
    }

    /// Alpha-beta search, remembering results in a transposition table.
    fn negamax(&mut self, position: Position, depth: u32, mut alpha: i32, mut beta: i32) -> i32 {
        self.positions += 1;
        if self.positions % 4096 == 0 && js_sys::Date::now() > self.deadline {
            self.stopped = true;
        }
        if self.stopped || position.is_full() {
            return 0;
        }
        if (0..COLUMNS).any(|c| position.can_play(c) && position.wins_with(c)) {
            return WIN - position.moves as i32;
        }
        if depth == 0 {
            return self.evaluate(&position);
        }
        let slot = (position.key().wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 44) as usize;
        let entry = self.table[slot];
        let remembered = (entry.key == position.key()).then_some(entry);
        if let Some(entry) = remembered.filter(|entry| u32::from(entry.depth) >= depth) {
            match entry.bound {
                Bound::Exact => return entry.score,
                Bound::Lower => alpha = alpha.max(entry.score),
                Bound::Upper => beta = beta.min(entry.score),
            }
            if alpha >= beta {
                return entry.score;
            }
        }
        let first = remembered.map(|entry| usize::from(entry.best));
        let original = alpha;
        let (mut best, mut best_column) = (-INFINITY, CENTER_FIRST[0]);
        for column in first
            .into_iter()
            .chain(CENTER_FIRST.into_iter().filter(|&c| Some(c) != first))
        {
            if !position.can_play(column) {
                continue;
            }
            let mut next = position;
            next.play(column);
            let score = -self.negamax(next, depth - 1, -beta, -alpha);
            if self.stopped {
                return 0;
            }
            if score > best {
                (best, best_column) = (score, column);
            }
            alpha = alpha.max(score);
            if alpha >= beta {
                break;
            }
        }
        let bound = if best <= original {
            Bound::Upper
        } else if best >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        if self.table[slot].key == 0 {
            self.remembered += 1;
        }
        self.table[slot] = Entry {
            key: position.key(),
            score: best,
            depth: depth as u8,
            bound,
            best: best_column as u8,
        };
        best
    }

    /// Counts the lines each player could still complete, favoring nearly
    /// finished ones. Positive is good for the player about to move.
    fn evaluate(&self, position: &Position) -> i32 {
        const WEIGHT: [i32; 5] = [0, 1, 4, 16, 64];
        let mine = position.current;
        let theirs = position.mask ^ position.current;
        self.windows
            .iter()
            .map(
                |&window| match ((mine & window).count_ones(), (theirs & window).count_ones()) {
                    (count, 0) => WEIGHT[count as usize],
                    (0, count) => -WEIGHT[count as usize],
                    _ => 0,
                },
            )
            .sum()
    }
}

// ---- Page code: the board, the buttons and the engine’s thinking.

type Client = <Engine as Worker>::Client;

#[derive(Clone, PartialEq)]
struct Cell {
    id: usize,
    you: bool,
    engine: bool,
    winning: bool,
    last: bool,
}

#[derive(Clone, PartialEq)]
struct ColumnView {
    column: usize,
    playable: bool,
    label: String,
    bar: String,
    tone: &'static str,
    best: bool,
    known: bool,
}

#[derive(Default)]
struct Control {
    generation: u32,
    cancel: Option<CancellationHandle>,
    ticker: Option<Interval>,
}

pub struct Game {
    engine: Signal<Option<Client>>,
    failure: Signal<Option<String>>,
    moves: Signal<Vec<u8>>,
    engine_first: Signal<bool>,
    budget: Signal<u32>,
    thought: Signal<Option<Thought>>,
    thinking: Signal<bool>,
    clock: Signal<u32>,
    control: Rc<RefCell<Control>>,
    _cleanup: Registration,
}

impl Game {
    pub fn new(owner: OwnerHandle) -> Self {
        let engine: Signal<Option<Client>> = signal(None);
        let failure = signal(None);
        let starting = fusor_worker::spawn::<Engine>(&owner, ());
        wasm_bindgen_futures::spawn_local({
            let (engine, failure) = (engine.clone(), failure.clone());
            async move {
                match starting.await {
                    Ok(client) => engine.update(|slot| *slot = Some(client)),
                    Err(error) => failure.set(Some(error.to_string())),
                }
            }
        });
        // Leaving the demo stops the thinking clock at once, without waiting
        // for the engine's call to settle.
        let control = Rc::new(RefCell::new(Control::default()));
        let cleanup = owner.on_cleanup({
            let control = control.clone();
            move || control.borrow_mut().ticker = None
        });
        Self {
            engine,
            failure,
            moves: signal(Vec::new()),
            engine_first: signal(false),
            budget: signal(1500),
            thought: signal(None),
            thinking: signal(false),
            clock: signal(0),
            control,
            _cleanup: cleanup,
        }
    }

    fn position(&self) -> Position {
        Position::from_moves(&self.moves.get())
    }

    fn engine_to_move(&self) -> bool {
        (self.moves.with(Vec::len) % 2 == 0) == self.engine_first.get()
    }

    fn finished(&self) -> bool {
        let position = self.position();
        position.winning_line().is_some() || position.is_full()
    }

    fn can_drop(&self, column: usize) -> bool {
        self.engine.with(Option::is_some)
            && !self.thinking.get()
            && !self.engine_to_move()
            && !self.finished()
            && self.position().can_play(column)
    }

    fn drop_disc(&self, column: usize) {
        if !self.can_drop(column) {
            return;
        }
        self.moves.update(|moves| moves.push(column as u8));
        if !self.finished() {
            self.think();
        }
    }

    /// Asks the engine for a move, showing its progress as it deepens.
    fn think(&self) {
        let Some(engine) = self.engine.get() else {
            return;
        };
        let generation = self.stop();
        let request = Request {
            moves: self.moves.get(),
            budget_ms: self.budget.get(),
        };
        self.thought.set(None);
        self.thinking.set(true);
        self.clock.set(0);
        let started = js_sys::Date::now();
        let job = engine.think(request).on_progress({
            let thought = self.thought.clone();
            move |update| thought.set(Some(update))
        });
        {
            let clock = self.clock.clone();
            let mut control = self.control.borrow_mut();
            control.cancel = Some(job.cancellation_handle());
            control.ticker = Some(Interval::new(100, move || {
                clock.set((js_sys::Date::now() - started) as u32)
            }));
        }
        let (moves, thought, thinking, failure, control) = (
            self.moves.clone(),
            self.thought.clone(),
            self.thinking.clone(),
            self.failure.clone(),
            self.control.clone(),
        );
        wasm_bindgen_futures::spawn_local(async move {
            let result = job.await;
            if control.borrow().generation != generation {
                return;
            }
            {
                let mut control = control.borrow_mut();
                control.cancel = None;
                control.ticker = None;
            }
            thinking.set(false);
            let choice = match result {
                Ok(final_thought) => {
                    let best = final_thought.best;
                    thought.set(Some(final_thought));
                    best
                }
                // “Move now” cancels the call: play the best move reported so far.
                Err(JobError::Worker(WorkerError::Cancelled)) => thought.get().and_then(|t| t.best),
                Err(error) => {
                    failure.set(Some(error.to_string()));
                    return;
                }
            };
            let position = Position::from_moves(&moves.get());
            let column = choice
                .map(usize::from)
                .filter(|&column| position.can_play(column))
                .or_else(|| CENTER_FIRST.into_iter().find(|&c| position.can_play(c)));
            if let Some(column) = column {
                moves.update(|moves| moves.push(column as u8));
            }
        });
    }

    fn move_now(&self) {
        if let Some(cancel) = self.control.borrow().cancel.as_ref() {
            cancel.cancel();
        }
    }

    /// Forgets any search in progress and returns the new generation.
    fn stop(&self) -> u32 {
        let generation = {
            let mut control = self.control.borrow_mut();
            control.generation += 1;
            if let Some(cancel) = control.cancel.take() {
                cancel.cancel();
            }
            control.ticker = None;
            control.generation
        };
        self.thinking.set(false);
        generation
    }

    fn new_game(&self, engine_first: bool) {
        self.stop();
        self.thought.set(None);
        self.engine_first.set(engine_first);
        self.moves.set(Vec::new());
        if engine_first {
            self.think();
        }
    }

    /// Takes back your last move and the engine’s reply.
    fn undo(&self) {
        self.stop();
        self.thought.set(None);
        self.moves.update(|moves| {
            moves.pop();
            let engine_first = self.engine_first.get_untracked();
            if (moves.len() % 2 == 0) == engine_first {
                moves.pop();
            }
        });
        if self.moves.with(Vec::is_empty) && self.engine_first.get() {
            self.think();
        }
    }

    fn can_undo(&self) -> bool {
        let played = self.moves.with(Vec::len);
        played > usize::from(self.engine_first.get())
    }

    fn status(&self) -> String {
        if let Some(error) = self.failure.get() {
            return format!("The engine stopped: {error}");
        }
        if self.engine.with(Option::is_none) {
            return "Starting the engine…".into();
        }
        let position = self.position();
        if position.winning_line().is_some() {
            return if self.engine_to_move() {
                "You win! Try a longer think time.".into()
            } else {
                "The engine wins. Undo, or start again.".into()
            };
        }
        if position.is_full() {
            return "A draw. Well played.".into();
        }
        if self.thinking.get() {
            format!(
                "Engine is thinking… {:.1} s",
                f64::from(self.clock.get()) / 1000.0
            )
        } else if self.moves.with(Vec::is_empty) {
            "Your move. Drop a disc in any column.".into()
        } else {
            "Your move.".into()
        }
    }

    fn stats(&self) -> String {
        self.thought
            .get()
            .filter(|thought| thought.depth > 0)
            .map(|thought| {
                format!(
                    "Looked {} moves ahead · {} positions in {:.1} s · remembers {}",
                    thought.depth,
                    compact(thought.positions),
                    f64::from(thought.elapsed_ms) / 1000.0,
                    compact(u64::from(thought.remembered)),
                )
            })
            .unwrap_or_else(|| {
                "The bars show how the engine rates each column while it thinks.".into()
            })
    }

    fn cells(&self) -> Vec<Cell> {
        let moves = self.moves.get();
        let engine_first = self.engine_first.get();
        let position = Position::from_moves(&moves);
        let line = position.winning_line().unwrap_or(0);
        let mut owner = [None; CELLS];
        let mut heights = [0; COLUMNS];
        let mut last = None;
        for (turn, &column) in moves.iter().enumerate() {
            let column = usize::from(column);
            let cell = column * ROWS + heights[column];
            heights[column] += 1;
            owner[cell] = Some((turn % 2 == 0) == engine_first);
            last = Some(cell);
        }
        (0..ROWS)
            .rev()
            .flat_map(|row| (0..COLUMNS).map(move |column| (column, row)))
            .map(|(column, row)| {
                let cell = column * ROWS + row;
                Cell {
                    id: cell,
                    you: owner[cell] == Some(false),
                    engine: owner[cell] == Some(true),
                    winning: line & (1 << (column * (ROWS + 1) + row)) != 0,
                    last: last == Some(cell),
                }
            })
            .collect()
    }

    fn columns(&self) -> Vec<ColumnView> {
        let thought = self.thought.get().unwrap_or_default();
        let position = self.position();
        (0..COLUMNS)
            .map(|column| {
                let score = thought.columns.get(column).copied().flatten();
                let (label, share, tone) = match score {
                    Some(score) if score > WIN - 100 => ("wins".to_owned(), 1.0, "good"),
                    Some(score) if score < -(WIN - 100) => ("loses".to_owned(), 1.0, "bad"),
                    Some(score) => {
                        let share = (f64::from(score) / 40.0).tanh();
                        (
                            format!("{score:+}"),
                            share.abs(),
                            if share >= 0.0 { "good" } else { "bad" },
                        )
                    }
                    None => (String::new(), 0.0, "none"),
                };
                ColumnView {
                    column,
                    playable: self.can_drop(column),
                    label,
                    bar: format!("height: {:.0}%", 8.0 + share * 92.0),
                    tone,
                    best: thought.best == Some(column as u8),
                    known: score.is_some() && position.can_play(column),
                }
            })
            .collect()
    }
}

fn compact(count: u64) -> String {
    match count {
        0..1_000 => count.to_string(),
        1_000..1_000_000 => format!("{:.0}K", count as f64 / 1_000.0),
        _ => format!("{:.1}M", count as f64 / 1_000_000.0),
    }
}

fusor::template!("web/demos/game.html");
