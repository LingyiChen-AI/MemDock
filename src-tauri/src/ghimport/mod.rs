//! "Import from GitHub": read someone else's memories / skills from a GitHub
//! link. `link` parses the link, `fetch` talks to GitHub (behind a trait so
//! tests never touch the network), `scan` decides what can be imported.
//! Memory imports become review suggestions; nothing is written directly.
pub mod fetch;
pub mod link;
pub mod scan;
