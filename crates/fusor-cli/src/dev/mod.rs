//! A failed edit never replaces a working site: the last successful build stays
//! served and the error goes to the terminal.
pub(crate) mod http;
pub(crate) mod proxy;
pub(crate) mod refresh;
pub(crate) mod server;
pub(crate) mod sources;
pub(crate) mod watch;
