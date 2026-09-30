//! A million books, generated once inside a stateful worker and kept there.
//! Every keystroke sends only the query, and a search you have already typed
//! past is cancelled before it runs.
use fusor::{OwnerHandle, Signal, signal};
use fusor_async::{Resource, browser::resource};
use fusor_worker::{JobError, NoError, TaskContext, TaskResult, Worker};
use serde::{Deserialize, Serialize};

const BOOKS: u32 = 1_000_000;
const SHOWN: usize = 30;
const FIRST_YEAR: u16 = 1850;
const DECADES: usize = 18;
const GENRES: [&str; 8] = [
    "Mystery",
    "Science fiction",
    "Fantasy",
    "Romance",
    "History",
    "Poetry",
    "Travel",
    "Cooking",
];
const SUGGESTIONS: [&str; 5] = ["moon", "kingdom", "silver river", "lisbon", "ada"];

// Each book is eight word numbers into this vocabulary, which keeps a million
// books small and makes scanning all of them fast.
const SMALL: &[&str] = &["the", "of", "a", "in", "and"];
const ADJECTIVES: &[&str] = &[
    "silent",
    "hidden",
    "golden",
    "broken",
    "last",
    "secret",
    "burning",
    "frozen",
    "quiet",
    "wild",
    "lost",
    "crimson",
    "distant",
    "electric",
    "forgotten",
    "gentle",
    "hollow",
    "iron",
    "lonely",
    "midnight",
    "northern",
    "painted",
    "restless",
    "salt",
    "shining",
    "silver",
    "stolen",
    "sudden",
    "tender",
    "velvet",
    "wandering",
    "bright",
    "deep",
    "empty",
    "glass",
    "green",
    "paper",
    "starlit",
    "stone",
    "summer",
];
/// Seven nouns for each genre, in `GENRES` order. A book’s genre usually
/// follows its noun, so searching “moon” turns up mostly science fiction.
const NOUNS: &[&str] = &[
    "letter", "key", "clock", "mirror", "shadow", "stranger", "archive", "moon", "planet", "orbit",
    "engine", "machine", "signal", "star", "kingdom", "queen", "dragon", "tower", "lantern",
    "wolf", "fox", "promise", "garden", "song", "orchard", "voice", "meadow", "window", "city",
    "empire", "bridge", "harbor", "market", "road", "station", "river", "sky", "night", "feather",
    "thread", "hour", "field", "island", "map", "compass", "ocean", "journey", "mountain", "sea",
    "harvest", "kitchen", "bread", "honey", "table", "feast", "spice",
];
const NOUNS_PER_GENRE: usize = 7;
const PLACES: &[&str] = &[
    "lisbon",
    "kyoto",
    "reykjavik",
    "marrakesh",
    "oslo",
    "havana",
    "cairo",
    "lima",
    "prague",
    "hanoi",
    "dublin",
    "nairobi",
    "quebec",
    "seville",
    "tbilisi",
    "valparaiso",
    "bergen",
    "zanzibar",
    "krakow",
    "montreal",
    "tangier",
    "porto",
    "kyiv",
    "istanbul",
];
const FIRST: &[&str] = &[
    "ada", "alan", "amara", "beatrix", "carlos", "chen", "dara", "elena", "emeka", "farah",
    "grace", "hana", "hugo", "ines", "ivan", "jonas", "kai", "leila", "lucia", "mateo", "mei",
    "nadia", "nikolai", "noor", "omar", "priya", "rafael", "rosa", "sana", "sofia", "tariq",
    "thea", "tomas", "uma", "vera", "wren", "yara", "yusuf", "zara", "zoe",
];
const LAST: &[&str] = &[
    "okafor",
    "lindqvist",
    "moreau",
    "tanaka",
    "silva",
    "novak",
    "haddad",
    "kowalski",
    "fernandez",
    "nakamura",
    "osei",
    "petrov",
    "rossi",
    "schmidt",
    "quinn",
    "abara",
    "bianchi",
    "castillo",
    "dubois",
    "eriksen",
    "fontaine",
    "garcia",
    "hartmann",
    "ivanova",
    "jensen",
    "kim",
    "laurent",
    "mendes",
    "nguyen",
    "oyelaran",
    "park",
    "reyes",
    "sato",
    "torres",
    "ueda",
    "varga",
    "weber",
    "xu",
    "yamamoto",
    "zielinski",
];
const LISTS: [&[&str]; 6] = [SMALL, ADJECTIVES, NOUNS, PLACES, FIRST, LAST];
const EMPTY: u8 = u8::MAX;

/// What the page sends for each keystroke.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Query {
    pub text: String,
    pub genre: Option<u8>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Book {
    pub id: u32,
    pub title: String,
    pub author: String,
    pub year: u16,
    pub genre: u8,
}

/// What the worker sends back: the first matches and counts over all of them.
#[derive(Serialize, Deserialize)]
pub struct Answer {
    pub matches: u32,
    pub scanned: u32,
    pub took_ms: f64,
    pub kept_bytes: u64,
    pub genres: Vec<u32>,
    pub decades: Vec<u32>,
    pub books: Vec<Book>,
}

// ---- Worker code: the library lives in the worker for as long as the demo.

pub struct Library {
    words: Vec<[u8; 8]>,
    years: Vec<u16>,
    genres: Vec<u8>,
}

#[fusor_worker::worker]
impl Library {
    /// Generates the books, reporting how many are ready so far.
    pub fn new(count: u32, ctx: TaskContext<u32>) -> TaskResult<Self> {
        let mut random = Random(0x2545_f491_4f6c_dd1d);
        let mut library = Library {
            words: Vec::with_capacity(count as usize),
            years: Vec::with_capacity(count as usize),
            genres: Vec::with_capacity(count as usize),
        };
        for made in 0..count {
            if made % 50_000 == 0 {
                ctx.report(made);
            }
            let (words, genre) = random.book();
            library.words.push(words);
            library.years.push(random.year(genre));
            library.genres.push(genre);
        }
        Ok(library)
    }

    /// Scans every book for the query, and counts the matches by genre and decade.
    pub fn search(&mut self, query: Query) -> TaskResult<Answer> {
        let started = js_sys::Date::now();
        // For each word typed, which vocabulary words contain it.
        let terms: Vec<[bool; 256]> = query
            .text
            .split_whitespace()
            .map(|term| {
                let term = term.to_ascii_lowercase();
                std::array::from_fn(|id| id != EMPTY as usize && word(id as u8).contains(&term))
            })
            .collect();
        let mut answer = Answer {
            matches: 0,
            scanned: self.words.len() as u32,
            took_ms: 0.0,
            kept_bytes: (self.words.len() * 8 + self.years.len() * 2 + self.genres.len()) as u64,
            genres: vec![0; GENRES.len()],
            decades: vec![0; DECADES],
            books: Vec::new(),
        };
        for (id, words) in self.words.iter().enumerate() {
            if !terms
                .iter()
                .all(|term| words.iter().any(|&word| term[word as usize]))
            {
                continue;
            }
            // Genre counts ignore the genre filter, so every bar stays useful.
            let genre = self.genres[id];
            answer.genres[genre as usize] += 1;
            if query.genre.is_some_and(|chosen| chosen != genre) {
                continue;
            }
            answer.matches += 1;
            answer.decades[usize::from((self.years[id] - FIRST_YEAR) / 10)] += 1;
            if answer.books.len() < SHOWN {
                answer.books.push(self.book(id));
            }
        }
        answer.took_ms = js_sys::Date::now() - started;
        Ok(answer)
    }
}

impl Library {
    fn book(&self, id: usize) -> Book {
        let words = &self.words[id];
        let title = words[..6]
            .iter()
            .take_while(|&&id| id != EMPTY)
            .enumerate()
            .map(|(position, &id)| {
                if position > 0 && id < SMALL.len() as u8 {
                    word(id).to_owned()
                } else {
                    capitalized(word(id))
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        Book {
            id: id as u32,
            title,
            author: format!(
                "{} {}",
                capitalized(word(words[6])),
                capitalized(word(words[7]))
            ),
            year: self.years[id],
            genre: self.genres[id],
        }
    }
}

fn word(id: u8) -> &'static str {
    let mut id = usize::from(id);
    for list in LISTS {
        if id < list.len() {
            return list[id];
        }
        id -= list.len();
    }
    ""
}

fn capitalized(word: &str) -> String {
    let mut letters = word.chars();
    letters
        .next()
        .map(|first| first.to_ascii_uppercase().to_string() + letters.as_str())
        .unwrap_or_default()
}

/// A small, repeatable generator, so every visitor gets the same library.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, limit: u32) -> u32 {
        (self.next() >> 33) as u32 % limit
    }

    fn pick(&mut self, list: usize) -> u8 {
        let start: usize = LISTS[..list].iter().map(|list| list.len()).sum();
        (start + self.below(LISTS[list].len() as u32) as usize) as u8
    }

    /// A book’s words and its genre.
    fn book(&mut self) -> ([u8; 8], u8) {
        let (the, of, a, in_, and) = (0, 1, 2, 3, 4);
        let (adjective, noun, place) = (1, 2, 3);
        let subject = self.pick(noun);
        let mut words = [EMPTY; 8];
        let title: &[u8] = match self.below(6) {
            0 => &[the, self.pick(adjective), subject],
            1 | 2 => &[subject, of, the, self.pick(adjective), self.pick(noun)],
            3 => &[a, subject, in_, self.pick(place)],
            4 => &[the, subject, of, self.pick(place)],
            _ => &[
                the,
                self.pick(adjective),
                subject,
                and,
                the,
                self.pick(noun),
            ],
        };
        words[..title.len()].copy_from_slice(title);
        words[6] = self.pick(4);
        words[7] = self.pick(5);
        let nouns_start = SMALL.len() + ADJECTIVES.len();
        let genre = if self.below(10) < 6 {
            (usize::from(subject) - nouns_start) / NOUNS_PER_GENRE
        } else {
            self.below(GENRES.len() as u32) as usize
        };
        (words, genre as u8)
    }

    /// Science fiction and cooking start later; poetry and history reach
    /// evenly back to 1850; everything else leans recent, as on a real shelf.
    fn year(&mut self, genre: u8) -> u16 {
        let (from, recent) = match genre {
            1 => (1895, true),
            7 => (1905, true),
            4 | 5 => (FIRST_YEAR, false),
            _ => (FIRST_YEAR, true),
        };
        let span = u32::from(2025 - from) + 1;
        let offset = if recent {
            self.below(span).max(self.below(span))
        } else {
            self.below(span)
        };
        from + offset as u16
    }
}

// ---- Page code: the search box, the facets and the results.

type Client = <Library as Worker>::Client;

/// One finished search, as the page shows it.
pub struct Answered {
    answer: Answer,
    round_trip: f64,
    sent: usize,
}

#[derive(Clone, PartialEq)]
struct GenreRow {
    id: u8,
    name: &'static str,
    count: String,
    width: String,
    selected: bool,
}

#[derive(Clone, PartialEq)]
struct DecadeBar {
    id: usize,
    label: String,
    height: String,
    title: String,
}

#[derive(Clone, PartialEq)]
struct Part {
    id: usize,
    text: String,
    hit: bool,
}

#[derive(Clone, PartialEq)]
struct BookRow {
    id: u32,
    title: Vec<Part>,
    author: String,
    year: u16,
    genre: &'static str,
}

pub struct Search {
    text: Signal<String>,
    genre: Signal<Option<u8>>,
    built: Signal<u32>,
    ready_in: Signal<Option<f64>>,
    failure: Signal<Option<String>>,
    asked: Signal<u32>,
    answered: Signal<u32>,
    results: Resource<Query, Answered, JobError<NoError>>,
}

impl Search {
    pub fn new(owner: OwnerHandle) -> Self {
        let text = signal(String::new());
        let genre = signal(None);
        let built = signal(0);
        let ready_in = signal(None);
        let failure = signal(None);
        let asked = signal(0);
        let answered = signal(0);
        let client: Signal<Option<Client>> = signal(None);

        // Start the worker. Its constructor builds the library and reports progress.
        let starting = fusor_worker::spawn::<Library>(&owner, BOOKS).on_progress({
            let built = built.clone();
            move |made| built.set(made)
        });
        wasm_bindgen_futures::spawn_local({
            let (client, ready_in, failure) = (client.clone(), ready_in.clone(), failure.clone());
            async move {
                let started = js_sys::Date::now();
                match starting.await {
                    Ok(library) => {
                        ready_in.set(Some(js_sys::Date::now() - started));
                        client.update(|slot| *slot = Some(library));
                    }
                    Err(error) => failure.set(Some(error.to_string())),
                }
            }
        });

        // Each new query calls the worker. When the query changes again, the
        // Resource cancels the previous call, so a stale answer never shows.
        let results = resource(
            &owner,
            {
                let (client, text, genre) = (client.clone(), text.clone(), genre.clone());
                move || {
                    client.with(Option::is_some).then(|| Query {
                        text: text.get(),
                        genre: genre.get(),
                    })
                }
            },
            {
                let (asked, answered) = (asked.clone(), answered.clone());
                move |query: Query, cancel| {
                    asked.update(|count| *count += 1);
                    let sent = serde_json::to_string(&query).map_or(0, |json| json.len());
                    let search = client
                        .with_untracked(|library| library.clone())
                        .map(|library| library.search(query).cancel_on(&cancel));
                    let answered = answered.clone();
                    async move {
                        let started = js_sys::Date::now();
                        let answer = search
                            .expect("queries start once the library is ready")
                            .await?;
                        answered.update(|count| *count += 1);
                        Ok(Answered {
                            answer,
                            round_trip: js_sys::Date::now() - started,
                            sent,
                        })
                    }
                }
            },
        );
        Self {
            text,
            genre,
            built,
            ready_in,
            failure,
            asked,
            answered,
            results,
        }
    }

    fn ready(&self) -> bool {
        self.ready_in.get().is_some()
    }

    fn building(&self) -> String {
        format!(
            "width: {}%",
            u64::from(self.built.get()) * 100 / u64::from(BOOKS)
        )
    }

    fn built_label(&self) -> String {
        format!("{} of {} books", grouped(self.built.get()), grouped(BOOKS))
    }

    fn failure_text(&self) -> String {
        self.failure.get().unwrap_or_default()
    }

    fn latest<R>(&self, read: impl FnOnce(&Answered) -> R) -> Option<R> {
        self.results
            .with(|state| state.data().map(|data| read(&data.value)))
    }

    fn summary(&self) -> String {
        self.latest(|done| match done.answer.matches {
            1 => "1 book".to_owned(),
            count => format!("{} books", grouped(count)),
        })
        .unwrap_or_else(|| "Searching…".into())
    }

    fn timing(&self) -> String {
        self.latest(|done| {
            format!(
                "scanned all {} in {}, inside the worker",
                grouped(done.answer.scanned),
                milliseconds(done.answer.took_ms),
            )
        })
        .unwrap_or_default()
    }

    fn round_trip(&self) -> String {
        self.latest(|done| milliseconds(done.round_trip))
            .unwrap_or_else(|| "…".into())
    }

    fn kept(&self) -> String {
        self.latest(|done| format!("{:.1} MB", done.answer.kept_bytes as f64 / 1_000_000.0))
            .unwrap_or_else(|| "…".into())
    }

    fn sent(&self) -> String {
        self.latest(|done| format!("{} bytes", done.sent))
            .unwrap_or_else(|| "…".into())
    }

    fn footnote(&self) -> String {
        let Some(took) = self.ready_in.get() else {
            return String::new();
        };
        let built = format!("The worker built the library in {}.", milliseconds(took));
        match self.skipped() {
            0 => built,
            1 => format!("{built} 1 search was skipped because you had already typed past it."),
            count => format!(
                "{built} {count} searches were skipped because you had already typed past them."
            ),
        }
    }

    /// Searches that were cancelled because a newer one replaced them.
    fn skipped(&self) -> u32 {
        let running = u32::from(self.results.with(|state| state.is_loading()));
        (self.asked.get())
            .saturating_sub(self.answered.get())
            .saturating_sub(running)
    }

    fn busy(&self) -> bool {
        self.results.with(|state| state.is_loading())
    }

    fn choose(&self, text: &str) {
        self.text.set(text.to_owned());
    }

    fn toggle_genre(&self, id: u8) {
        self.genre.update(|genre| {
            *genre = if *genre == Some(id) { None } else { Some(id) };
        });
    }

    fn suggestions(&self) -> Vec<&'static str> {
        SUGGESTIONS.to_vec()
    }

    fn genre_rows(&self) -> Vec<GenreRow> {
        let chosen = self.genre.get();
        let counts = self
            .latest(|done| done.answer.genres.clone())
            .unwrap_or_else(|| vec![0; GENRES.len()]);
        let most = counts.iter().copied().max().unwrap_or(0).max(1);
        GENRES
            .iter()
            .zip(counts)
            .enumerate()
            .map(|(id, (name, count))| GenreRow {
                id: id as u8,
                name,
                count: grouped(count),
                width: format!("width: {}%", u64::from(count) * 100 / u64::from(most)),
                selected: chosen == Some(id as u8),
            })
            .collect()
    }

    fn decade_bars(&self) -> Vec<DecadeBar> {
        let counts = self
            .latest(|done| done.answer.decades.clone())
            .unwrap_or_else(|| vec![0; DECADES]);
        let most = counts.iter().copied().max().unwrap_or(0).max(1);
        counts
            .into_iter()
            .enumerate()
            .map(|(decade, count)| {
                let year = FIRST_YEAR as usize + decade * 10;
                DecadeBar {
                    id: decade,
                    label: if decade % 4 == 0 {
                        format!("{year}")
                    } else {
                        String::new()
                    },
                    height: format!(
                        "height: {}%",
                        (u64::from(count) * 100 / u64::from(most)).max(2)
                    ),
                    title: format!("{year}s: {} books", grouped(count)),
                }
            })
            .collect()
    }

    fn books(&self) -> Vec<BookRow> {
        let text = self.text.get().to_ascii_lowercase();
        let terms: Vec<&str> = text.split_whitespace().collect();
        self.latest(|done| {
            done.answer
                .books
                .iter()
                .map(|book| BookRow {
                    id: book.id,
                    title: highlighted(&book.title, &terms),
                    author: book.author.clone(),
                    year: book.year,
                    genre: GENRES[usize::from(book.genre)],
                })
                .collect()
        })
        .unwrap_or_default()
    }
}

/// Splits `text` into runs, marking the ones that match a typed word.
fn highlighted(text: &str, terms: &[&str]) -> Vec<Part> {
    let lower = text.to_ascii_lowercase();
    let mut hit = vec![false; text.len()];
    for term in terms {
        for (start, _) in lower.match_indices(term) {
            hit[start..start + term.len()].fill(true);
        }
    }
    let mut parts: Vec<Part> = Vec::new();
    for (index, character) in text.char_indices() {
        match parts.last_mut() {
            Some(part) if part.hit == hit[index] => part.text.push(character),
            _ => parts.push(Part {
                id: parts.len(),
                text: character.to_string(),
                hit: hit[index],
            }),
        }
    }
    parts
}

fn grouped(number: u32) -> String {
    let digits = number.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn milliseconds(ms: f64) -> String {
    if ms < 1.0 {
        "under 1 ms".into()
    } else {
        format!("{ms:.0} ms")
    }
}

fusor::template!("web/demos/search.html");
