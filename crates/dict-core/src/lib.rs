//! Core of the csdict dictionary: data model, `.csd` pack format and fast
//! lookup (FST index with prefix and fuzzy search).

pub mod kaikki;
pub mod model;
pub mod normalize;
pub mod pack;
pub mod wordnet;

pub use model::{DictMeta, Entry, Example, Form, Sense};
pub use pack::{Dictionary, Hit, MatchKind, PackBuilder};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("index error: {0}")]
    Fst(#[from] fst::Error),
    #[error("serialization error: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid pack: {0}")]
    Format(String),
}
