//! eve-dogma-e: a Pyfa-faithful EVE dogma engine (Rust port of Pyfa's eos). GPL-3.0-or-later.
pub mod data;
pub mod request;
pub mod eos {
    pub mod capsim;
    pub mod custom;
    pub mod cx;
    pub mod fit;
    pub mod mad;
    pub mod stats;
}
pub mod generated {
    pub mod effects;
}
pub mod api;
pub mod jv;
