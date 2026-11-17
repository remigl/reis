//! Reis core — offline music recognition with synchronized lyrics.
//!
//! ```text
//! PCM ─▶ audio::Resampler ─▶ fingerprint (landmarks) ─▶ index::match_hashes
//!                                                        │ song + offset
//!                 engine::Engine (listening / locked / paused, drift correction)
//!                                                        │ position
//!        catalog::Catalog (mmap) ─▶ metadata · lyrics::LyricsView · artwork
//! ```
//! Zero dependencies; the C ABI lives in [`ffi`].

pub mod audio;
pub mod bytes;
pub mod catalog;
pub mod engine;
pub mod ffi;
pub mod fft;
pub mod fingerprint;
pub mod index;
pub mod lyrics;

pub use catalog::{Catalog, CatalogBuilder, SongMeta};
pub use engine::{Engine, EngineConfig, Phase, State};
pub use fingerprint::{Fingerprinter, FpParams};
pub use index::{match_hashes, MatchResult};

/// One-shot identification of mono samples at the catalog's analysis rate.
pub fn identify(cat: &Catalog, fp: &Fingerprinter, samples: &[f32]) -> MatchResult {
    let h = fp.hashes(samples);
    match_hashes(&cat.index(), &h, 5)
}
