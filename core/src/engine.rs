//! Recognition + position tracking state machine. UI-agnostic and clock-agnostic:
//! the host pushes PCM with a monotonic timestamp and calls `tick(now_ms)`.
//!
//! Listening : mic on, try to identify every `attempt_every` on a growing window
//!             (2 s .. 10 s) until a confident match.
//! Locked    : position = anchor_pos + (now - anchor_clock). Mic is *off* most of
//!             the time (`wants_audio == false`); every `verify_every` a short
//!             window is captured and matched again to correct drift / seeks /
//!             song changes.
//! Paused    : sustained silence while locked freezes the position; sound
//!             coming back resumes it and triggers an immediate verification.

use crate::audio::{rms_dbfs, Resampler};
use crate::catalog::Catalog;
use crate::fingerprint::Fingerprinter;
use crate::index::{match_hashes, MatchResult};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct EngineConfig {
    pub min_window_s: f64,
    pub max_window_s: f64,
    pub attempt_every_s: f64,
    pub verify_every_s: f64,
    pub verify_window_s: f64,
    /// strict single-shot rule: score >= accept_score && score >= accept_ratio * (second + 1)
    pub accept_score: u32,
    pub accept_ratio: f32,
    /// weak rule: must be confirmed by a second attempt agreeing on song and offset
    pub weak_score: u32,
    pub weak_ratio: f32,
    /// verification of the already-locked song (offset known a priori)
    pub verify_score: u32,
    pub silence_dbfs: f32,
    pub pause_after_s: f64,
    pub max_misses: u32,
    pub correct_threshold_ms: f64,
    /// Battery saver: close the mic between verifications (pauses are then only
    /// noticed at the next verification). Default: mic stays open, but only its
    /// level is computed between verifications (pause detection < 2 s).
    pub duty_cycle_mic: bool,
    pub query_density: f32,
    pub query_fanout: f32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            min_window_s: 2.0,
            max_window_s: 10.0,
            attempt_every_s: 0.5,
            verify_every_s: 15.0,
            verify_window_s: 4.0,
            accept_score: 13,
            accept_ratio: 2.2,
            weak_score: u32::MAX, // disabled: confirmations of overlapping windows are not independent (measured)
            weak_ratio: 1.4,
            verify_score: 5,
            silence_dbfs: -60.0,
            pause_after_s: 1.5,
            max_misses: 4,
            correct_threshold_ms: 60.0,
            duty_cycle_mic: false,
            query_density: 2.0,
            query_fanout: 4.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Phase {
    Listening = 0,
    Locked = 1,
    Paused = 2,
}

#[derive(Clone, Copy, Debug)]
pub struct State {
    pub phase: Phase,
    pub song: Option<u32>,
    pub position_ms: f64,
    pub confidence: f32,
    pub wants_audio: bool,
    pub uncertain: bool,
    pub level_dbfs: f32,
    pub last_score: u32,
    pub last_match_us: u64,
    pub matches: u32,
    pub corrections: u32,
    pub last_correction_ms: f64,
}

struct Lock {
    song: u32,
    anchor_pos: f64,
    anchor_clock: f64,
}

pub struct Engine {
    pub cat: Arc<Catalog>,
    fp: Fingerprinter,
    pub cfg: EngineConfig,
    rs: Option<Resampler>,
    rs_tmp: Vec<f32>,
    buf: Vec<f32>,      // analysis-rate samples, contiguous
    buf_end_clock: f64, // clock (ms) of the sample after buf's last
    last_attempt: f64,
    lock: Option<Lock>,
    frozen: Option<f64>,
    next_verify: f64,
    misses: u32,
    last_loud: f64,
    pending: Option<(u32, f64, f64)>, // weak candidate: song, position, at clock
    st: State,
}

impl Engine {
    pub fn new(cat: Arc<Catalog>, cfg: EngineConfig) -> Self {
        let fp = Fingerprinter::new(cat.params.query(cfg.query_density, cfg.query_fanout));
        Engine {
            cat,
            fp,
            cfg,
            rs: None,
            rs_tmp: Vec::new(),
            buf: Vec::new(),
            buf_end_clock: f64::NAN,
            last_attempt: f64::NEG_INFINITY,
            lock: None,
            frozen: None,
            next_verify: 0.0,
            misses: 0,
            last_loud: f64::NEG_INFINITY,
            pending: None,
            st: State {
                phase: Phase::Listening,
                song: None,
                position_ms: 0.0,
                confidence: 0.0,
                wants_audio: true,
                uncertain: false,
                level_dbfs: -120.0,
                last_score: 0,
                last_match_us: 0,
                matches: 0,
                corrections: 0,
                last_correction_ms: 0.0,
            },
        }
    }

    fn sr(&self) -> f64 {
        self.cat.params.sample_rate as f64
    }

    /// Push mono PCM. `clock_end_ms` = monotonic time of the end of this chunk.
    pub fn push_pcm(&mut self, pcm: &[f32], rate: u32, clock_end_ms: f64) {
        if pcm.is_empty() {
            return;
        }
        let out_rate = self.cat.params.sample_rate;
        if self.rs.as_ref().map(|r| r.in_rate) != Some(rate) {
            self.rs = Some(Resampler::new(rate, out_rate));
            self.buf.clear();
        }
        let dur = pcm.len() as f64 * 1000.0 / rate as f64;
        // gap / discontinuity -> window no longer contiguous
        if self.buf_end_clock.is_finite() && (clock_end_ms - dur - self.buf_end_clock).abs() > 250.0
        {
            self.buf.clear();
        }
        self.buf_end_clock = clock_end_ms;
        let lvl = rms_dbfs(pcm);
        self.st.level_dbfs = lvl;
        if lvl > self.cfg.silence_dbfs {
            self.last_loud = clock_end_ms;
        }
        self.rs_tmp.clear();
        self.rs.as_mut().unwrap().push(pcm, &mut self.rs_tmp);
        self.buf.extend_from_slice(&self.rs_tmp);
        let cap = (self.cfg.max_window_s * self.sr()) as usize;
        if self.buf.len() > cap {
            let d = self.buf.len() - cap;
            self.buf.drain(..d);
        }
    }

    pub fn buffered_s(&self) -> f64 {
        self.buf.len() as f64 / self.sr()
    }

    /// Identify the current buffer. Returns (result, clock of buffer start).
    fn attempt(&mut self) -> (MatchResult, f64) {
        let t0 = std::time::Instant::now();
        let h = self.fp.hashes(&self.buf);
        let r = match_hashes(&self.cat.index(), &h, 5);
        self.st.last_match_us = t0.elapsed().as_micros() as u64;
        self.st.last_score = r.best().map(|c| c.score).unwrap_or(0);
        let start_clock = self.buf_end_clock - self.buf.len() as f64 * 1000.0 / self.sr();
        (r, start_clock)
    }

    fn passes(r: &MatchResult, min: u32, ratio: f32) -> bool {
        match r.best() {
            Some(c) => c.score >= min && c.score as f32 >= ratio * (r.second_score() as f32 + 1.0),
            None => false,
        }
    }
    fn accepted(&self, r: &MatchResult) -> bool {
        Self::passes(r, self.cfg.accept_score, self.cfg.accept_ratio)
    }
    fn weak(&self, r: &MatchResult) -> bool {
        Self::passes(r, self.cfg.weak_score, self.cfg.weak_ratio)
    }

    fn offset_ms(&self, frames: i32) -> f64 {
        frames as f64 * self.cat.params.hop as f64 * 1000.0 / self.sr()
    }

    fn position_at(&self, clock: f64) -> f64 {
        match (&self.lock, self.frozen) {
            (_, Some(f)) => f,
            (Some(l), None) => l.anchor_pos + (clock - l.anchor_clock),
            _ => 0.0,
        }
    }

    pub fn tick(&mut self, now: f64) -> State {
        let cfg = self.cfg;
        let silent_window = rms_dbfs(&self.buf) <= cfg.silence_dbfs;
        match self.st.phase {
            Phase::Listening => {
                self.st.wants_audio = true;
                // back off on slow devices / huge catalogs: never spend more than ~1/3 of the time matching
                let interval =
                    (cfg.attempt_every_s * 1000.0).max(3.0 * self.st.last_match_us as f64 / 1000.0);
                if self.buffered_s() >= cfg.min_window_s
                    && now - self.last_attempt >= interval
                    && !silent_window
                {
                    self.last_attempt = now;
                    let (r, start) = self.attempt();
                    let strong = self.accepted(&r);
                    let mut confirm = false;
                    if !strong && self.weak(&r) {
                        let c = r.best().unwrap();
                        let pos = self.offset_ms(c.offset_frames);
                        if let Some((ps, ppos, pclock)) = self.pending {
                            // same song, and the offset advanced consistently with the clock
                            confirm =
                                ps == c.song && ((pos - ppos) - (start - pclock)).abs() < 100.0;
                        }
                        self.pending = Some((c.song, pos, start));
                    } else if !strong {
                        self.pending = None;
                    }
                    if strong || confirm {
                        self.pending = None;
                        let c = r.best().unwrap();
                        self.lock = Some(Lock {
                            song: c.song,
                            anchor_pos: self.offset_ms(c.offset_frames),
                            anchor_clock: start,
                        });
                        self.frozen = None;
                        self.st.phase = Phase::Locked;
                        self.st.song = Some(c.song);
                        self.st.confidence = r.confidence();
                        self.st.uncertain = false;
                        self.st.matches += 1;
                        self.misses = 0;
                        self.next_verify = now + cfg.verify_every_s * 1000.0;
                        self.buf.clear();
                    }
                }
            }
            Phase::Locked => {
                let mic_on = !cfg.duty_cycle_mic
                    || now >= self.next_verify - cfg.verify_window_s * 1000.0 - 300.0;
                self.st.wants_audio = mic_on;
                // pause detection (only meaningful while we hear the mic)
                if mic_on
                    && self.buffered_s() > cfg.pause_after_s
                    && now - self.last_loud > cfg.pause_after_s * 1000.0
                {
                    self.frozen = Some(
                        self.position_at(self.last_loud.max(now - cfg.pause_after_s * 1000.0)),
                    );
                    self.st.phase = Phase::Paused;
                } else if now >= self.next_verify && self.buffered_s() >= cfg.verify_window_s - 0.05
                {
                    let (r, start) = self.attempt();
                    self.buf.clear();
                    let cur = self.lock.as_ref().unwrap().song;
                    let predicted = self.position_at(start);
                    // the locked song at a plausible offset is a strong prior: lenient threshold
                    let same_ok = r
                        .top
                        .iter()
                        .find(|c| c.song == cur)
                        .map(|c| {
                            c.score >= self.cfg.verify_score
                                && (self.offset_ms(c.offset_frames) - predicted).abs() < 5000.0
                        })
                        .unwrap_or(false);
                    let other_ok = self.accepted(&r) && r.best().unwrap().song != cur;
                    if same_ok || other_ok {
                        let c = if other_ok {
                            r.best().unwrap()
                        } else {
                            *r.top.iter().find(|c| c.song == cur).unwrap()
                        };
                        let measured = self.offset_ms(c.offset_frames);
                        if c.song == cur {
                            let err = measured - self.position_at(start);
                            if err.abs() > cfg.correct_threshold_ms {
                                self.lock.as_mut().unwrap().anchor_pos += err;
                                self.st.corrections += 1;
                                self.st.last_correction_ms = err;
                            }
                        } else {
                            self.lock = Some(Lock {
                                song: c.song,
                                anchor_pos: measured,
                                anchor_clock: start,
                            });
                            self.st.song = Some(c.song);
                        }
                        self.st.confidence = r.confidence();
                        self.st.uncertain = false;
                        self.st.matches += 1;
                        self.misses = 0;
                        self.next_verify = now + cfg.verify_every_s * 1000.0;
                    } else {
                        self.misses += 1;
                        self.st.uncertain = true;
                        self.next_verify = now + cfg.verify_window_s * 1000.0;
                        if self.misses >= cfg.max_misses {
                            // keep displaying (position keeps running) but search continuously
                            self.st.phase = Phase::Listening;
                            self.last_attempt = f64::NEG_INFINITY;
                        }
                    }
                }
            }
            Phase::Paused => {
                self.st.wants_audio = true;
                if now - self.last_loud < 300.0 {
                    let f = self.frozen.take().unwrap_or(0.0);
                    if let Some(l) = self.lock.as_mut() {
                        l.anchor_pos = f;
                        l.anchor_clock = self.last_loud;
                    }
                    self.st.phase = Phase::Locked;
                    self.buf.clear();
                    self.next_verify = now + cfg.verify_window_s * 1000.0;
                }
            }
        }
        if self.lock.is_some() {
            self.st.position_ms = self.position_at(now).max(0.0);
            let song = self.lock.as_ref().unwrap().song;
            let dur = self.cat.duration_ms(song) as f64;
            if dur > 0.0 && self.st.position_ms > dur + 1500.0 {
                self.lock = None;
                self.frozen = None;
                self.st.song = None;
                self.st.phase = Phase::Listening;
                self.st.wants_audio = true;
            }
        }
        self.st
    }

    pub fn state(&self) -> State {
        self.st
    }

    pub fn reset(&mut self) {
        let cat = self.cat.clone();
        let cfg = self.cfg;
        *self = Engine::new(cat, cfg);
    }
}
