use crate::code::{self, CodeFile};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Example {
    Counter,
    Search,
    KeyedList,
    AsyncData,
}

impl Example {
    pub fn source(self, rust: bool) -> &'static CodeFile {
        match (self, rust) {
            (Self::Counter, false) => &code::COUNTER_HTML,
            (Self::Counter, true) => &code::COUNTER_RS,
            (Self::Search, false) => &code::SEARCH_HTML,
            (Self::Search, true) => &code::SEARCH_RS,
            (Self::KeyedList, false) => &code::KEYED_LIST_HTML,
            (Self::KeyedList, true) => &code::KEYED_LIST_RS,
            (Self::AsyncData, false) => &code::ASYNC_DATA_HTML,
            (Self::AsyncData, true) => &code::ASYNC_DATA_RS,
        }
    }

    pub fn page_source(self, rust: bool) -> &'static CodeFile {
        match (self, rust) {
            (Self::Counter, false) => &code::COUNTER_PAGE_HTML,
            (Self::Counter, true) => &code::COUNTER_PAGE_RS,
            (Self::Search, false) => &code::SEARCH_PAGE_HTML,
            (Self::Search, true) => &code::SEARCH_PAGE_RS,
            (Self::KeyedList, false) => &code::KEYED_LIST_PAGE_HTML,
            (Self::KeyedList, true) => &code::KEYED_LIST_PAGE_RS,
            (Self::AsyncData, false) => &code::ASYNC_DATA_PAGE_HTML,
            (Self::AsyncData, true) => &code::ASYNC_DATA_PAGE_RS,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Counter => {
                "The buttons call methods on the Rust struct. The output reads the count, so only its text updates."
            }
            Self::Search => {
                "bind keeps the input in a Rust String. A Rust method filters the guides as you type."
            }
            Self::KeyedList => {
                "<ForEach> repeats the row HTML for each id in a Rust Vec. Keys keep each row’s DOM, and its note, when the order changes."
            }
            Self::AsyncData => {
                "Each field loads separately. <Async> keeps the previous issue visible until both reads finish, then shows them together."
            }
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Self::Counter => "<Counter>",
            Self::Search => "<LiveSearch>",
            Self::KeyedList => "<KeyedList>",
            Self::AsyncData => "<AsyncData>",
        }
    }

    pub fn guide(self) -> &'static str {
        match self {
            Self::Counter => "/docs/html-and-rust",
            Self::Search => "/docs/reactivity",
            Self::KeyedList => "/docs/components/for-each",
            Self::AsyncData => "/docs/coherent-async",
        }
    }
}
