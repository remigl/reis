//! Reis core — build fingerprint libraries from reference sounds and recognize them, offline.
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
pub mod decode;
pub mod engine;
pub mod ffi;
pub mod fft;
pub mod fingerprint;
pub mod index;
pub mod json;
pub mod library;
pub mod lyrics;
pub mod nn;
pub mod recognize;
pub mod semantic;

pub use engine::{Engine, EngineConfig, Phase, State};
pub use fingerprint::{Fingerprinter, FpParams};
pub use index::{match_hashes, MatchResult};
pub use library::{Library, LibraryBuilder, LibraryConfig, SourceItem};
pub use recognize::{Recognition, Recognizer, RecognizerConfig, Verdict};

/// One-shot exact identification of mono samples at the exact engine's rate.
pub fn identify(lib: &Library, fp: &Fingerprinter, samples: &[f32]) -> MatchResult {
    let h = fp.hashes(samples);
    match_hashes(&lib.index(), &h, 5)
}
