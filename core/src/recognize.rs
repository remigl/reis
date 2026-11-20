//! One entry point for every use: give audio, get a decision.
//!
//! ```text
//! audio ─┬─ exact engine (landmarks)   → same recording? + position
//!        ├─ semantic engine (embedding) → same kind of sound? (labels of the library)
//!        └─ model head (AudioSet tags)  → what is it, in general? (no library needed)
//!                         ↓
//!   Verdict: Exact > Similar > Tag > Unknown
//! ```

use crate::fingerprint::{Fingerprinter, FpParams};
use crate::index::match_hashes;
use crate::json::{self, Json};
use crate::library::Library;
use crate::nn::Model;
use crate::semantic::{LabelScore, RefScore};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct RecognizerConfig {
    /// exact engine acceptance: score >= exact_min_score && score >= exact_ratio * (second + 1)
    pub exact_min_score: u32,
    pub exact_ratio: f32,
    /// semantic threshold; None = value calibrated when the library was built
    pub semantic_threshold: Option<f32>,
    /// alternatively: accepted rate of unknown sounds, mapped through the library's calibration curve
    pub false_accept: Option<f32>,
    /// minimal AudioSet probability for a general tag
    pub tag_threshold: f32,
    /// below this level the input is reported as silence (dBFS RMS)
    pub silence_dbfs: f32,
    pub top: usize,
    /// query-side density multipliers of the exact engine (see fingerprint.rs)
    pub query_density: f32,
    pub query_fanout: f32,
    pub engines: Vec<String>,
}

impl Default for RecognizerConfig {
    fn default() -> Self {
        RecognizerConfig {
            exact_min_score: 13,
            exact_ratio: 2.2,
            semantic_threshold: None,
            false_accept: None,
            tag_threshold: 0.3,
            silence_dbfs: -65.0,
            top: 5,
            query_density: 2.0,
            query_fanout: 4.0,
            engines: vec!["exact".into(), "semantic".into(), "tags".into()],
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExactHit {
    pub ref_id: u32,
    /// position of the query start inside the reference (seconds)
    pub offset_s: f64,
    pub score: u32,
    pub second: u32,
    pub accepted: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Exact {
        ref_id: u32,
        offset_s: f64,
    },
    Similar {
        label: String,
        score: f32,
    },
    Tag {
        label: String,
        prob: f32,
    },
    /// the input is too quiet to analyse
    Silence,
    Unknown,
}

#[derive(Clone, Debug, Default)]
pub struct Recognition {
    pub exact: Option<ExactHit>,
    pub similar: Vec<(String, f32, usize)>, // label, score, references of that label
    pub neighbours: Vec<RefScore>,
    pub semantic_threshold: f32,
    pub tags: Vec<(String, f32)>,
    /// tags were computed but looked out-of-distribution (many classes saturated)
    pub tags_suppressed: bool,
    pub level_dbfs: f32,
    pub verdict: Option<Verdict>,
    pub timings_ms: Vec<(&'static str, f64)>,
}

pub struct Recognizer {
    pub lib: Arc<Library>,
    pub cfg: RecognizerConfig,
    exact_fp: Option<Fingerprinter>,
    pub model: Option<Model>,
    labels: Vec<u32>,
}

impl Recognizer {
    pub fn new(lib: Arc<Library>, cfg: RecognizerConfig) -> Result<Recognizer, String> {
        let exact_fp = lib
            .params
            .map(|p: FpParams| Fingerprinter::new(p.query(cfg.query_density, cfg.query_fanout)));
        let model = match lib.model() {
            Some(m) => Some(m?),
            None => None,
        };
        let labels = lib.label_ids();
        Ok(Recognizer {
            lib,
            cfg,
            exact_fp,
            model,
            labels,
        })
    }

    /// Use a stand-alone model for general tags when the library has none.
    pub fn with_model(mut self, m: Model) -> Self {
        if self.model.is_none() {
            self.model = Some(m);
        }
        self
    }

    fn on(&self, e: &str) -> bool {
        self.cfg.engines.iter().any(|x| x == e)
    }

    /// Recognize mono audio at any sample rate.
    pub fn recognize(&self, audio: &[f32], rate: u32) -> Recognition {
        let mut r = Recognition {
            level_dbfs: crate::audio::rms_dbfs(audio),
            ..Default::default()
        };
        if r.level_dbfs < self.cfg.silence_dbfs {
            r.verdict = Some(Verdict::Silence);
            return r;
        }
        // exact
        if let (Some(fp), true) = (&self.exact_fp, self.on("exact")) {
            let t = std::time::Instant::now();
            let x = crate::audio::resample(audio, rate, fp.p.sample_rate);
            let m = match_hashes(&self.lib.index(), &fp.hashes(&x), 5);
            if let Some(b) = m.best() {
                let second = m.second_score();
                let accepted = b.score >= self.cfg.exact_min_score
                    && b.score as f32 >= self.cfg.exact_ratio * (second as f32 + 1.0);
                r.exact = Some(ExactHit {
                    ref_id: b.song,
                    offset_s: b.offset_frames as f64 * fp.p.frame_secs(),
                    score: b.score,
                    second,
                    accepted,
                });
            }
            r.timings_ms
                .push(("exact", t.elapsed().as_secs_f64() * 1e3));
        }
        // neural: one forward pass per window serves both the semantic engine and the tags
        if let Some(model) = &self.model {
            let need_sem = self.on("semantic") && self.lib.has_semantic();
            let need_tags = self.on("tags") && !model.labels.is_empty();
            if need_sem || need_tags {
                let t = std::time::Instant::now();
                let x = crate::audio::resample(audio, rate, model.frontend.sample_rate);
                let view = self.lib.semantic();
                let cfg = view.as_ref().map(|v| v.config()).unwrap_or_default();
                if let (true, Some(v)) = (need_sem, &view) {
                    let q = v.query_vectors(model, &x);
                    let rs = v.ref_scores(&q, self.lib.refs.len(), None);
                    let ls: Vec<LabelScore> = v.label_scores(&rs, &self.labels, cfg.top_m);
                    r.semantic_threshold = self
                        .cfg
                        .semantic_threshold
                        .or_else(|| self.cfg.false_accept.and_then(|fa| v.threshold_for(fa)))
                        .unwrap_or(v.threshold());
                    r.similar = ls
                        .iter()
                        .take(self.cfg.top)
                        .map(|l| (self.lib.labels[l.label as usize].clone(), l.score, l.refs))
                        .collect();
                    r.neighbours = rs.into_iter().take(self.cfg.top).collect();
                    r.timings_ms
                        .push(("semantic", t.elapsed().as_secs_f64() * 1e3));
                }
                if need_tags {
                    let t = std::time::Instant::now();
                    let out = model.forward_audio(&x, model.frontend.sample_rate);
                    if let Some(p) = out.get("logits") {
                        // AudioSet taggers answer stationary noise / featureless input with many
                        // saturated classes at once: treat that as "no reliable tag".
                        if p.iter().filter(|&&v| v > 0.9).count() >= 5 {
                            r.tags_suppressed = true;
                        } else {
                            let mut idx: Vec<usize> = (0..p.len()).collect();
                            idx.sort_by(|a, b| p[*b].partial_cmp(&p[*a]).unwrap());
                            r.tags = idx
                                .into_iter()
                                .take(self.cfg.top)
                                .map(|i| (model.labels.get(i).cloned().unwrap_or_default(), p[i]))
                                .collect();
                        }
                    }
                    r.timings_ms.push(("tags", t.elapsed().as_secs_f64() * 1e3));
                }
            }
        }
        r.verdict = Some(self.decide(&r));
        r
    }

    fn decide(&self, r: &Recognition) -> Verdict {
        if let Some(e) = r.exact.as_ref().filter(|e| e.accepted) {
            return Verdict::Exact {
                ref_id: e.ref_id,
                offset_s: e.offset_s,
            };
        }
        if let Some((l, s, _)) = r.similar.first() {
            if *s >= r.semantic_threshold {
                return Verdict::Similar {
                    label: l.clone(),
                    score: *s,
                };
            }
        }
        if let Some((l, p)) = r.tags.first() {
            if *p >= self.cfg.tag_threshold {
                return Verdict::Tag {
                    label: l.clone(),
                    prob: *p,
                };
            }
        }
        Verdict::Unknown
    }

    /// Machine-readable result (one JSON object).
    pub fn to_json(&self, r: &Recognition) -> Json {
        let lib = &self.lib;
        let refj = |id: u32| {
            let rf = &lib.refs[id as usize];
            json::obj(vec![
                ("id", json::n(id as f64)),
                ("name", json::s(&rf.name)),
                ("label", json::s(&rf.label)),
                ("file", json::s(&rf.file)),
            ])
        };
        let verdict = match &r.verdict {
            Some(Verdict::Exact { ref_id, offset_s }) => json::obj(vec![
                ("kind", json::s("exact")),
                ("reference", refj(*ref_id)),
                ("offset_s", json::n((offset_s * 1000.0).round() / 1000.0)),
            ]),
            Some(Verdict::Similar { label, score }) => json::obj(vec![
                ("kind", json::s("similar")),
                ("label", json::s(label)),
                ("score", json::n(round3(*score))),
            ]),
            Some(Verdict::Tag { label, prob }) => json::obj(vec![
                ("kind", json::s("tag")),
                ("label", json::s(label)),
                ("probability", json::n(round3(*prob))),
            ]),
            Some(Verdict::Silence) => json::obj(vec![("kind", json::s("silence"))]),
            _ => json::obj(vec![("kind", json::s("unknown"))]),
        };
        let mut o = vec![("verdict", verdict)];
        if let Some(e) = &r.exact {
            o.push((
                "exact",
                json::obj(vec![
                    ("reference", refj(e.ref_id)),
                    ("offset_s", json::n((e.offset_s * 1000.0).round() / 1000.0)),
                    ("score", json::n(e.score as f64)),
                    ("second", json::n(e.second as f64)),
                    ("accepted", Json::Bool(e.accepted)),
                ]),
            ));
        }
        if !r.similar.is_empty() {
            o.push((
                "similar",
                Json::Arr(
                    r.similar
                        .iter()
                        .map(|(l, s, n)| {
                            json::obj(vec![
                                ("label", json::s(l)),
                                ("score", json::n(round3(*s))),
                                ("references", json::n(*n as f64)),
                            ])
                        })
                        .collect(),
                ),
            ));
            o.push(("semantic_threshold", json::n(round3(r.semantic_threshold))));
            o.push((
                "neighbours",
                Json::Arr(
                    r.neighbours
                        .iter()
                        .map(|n| {
                            let mut j = refj(n.ref_id);
                            if let Json::Obj(v) = &mut j {
                                v.push(("score".into(), json::n(round3(n.score))));
                                v.push(("at_s".into(), json::n(round3(n.at_s))));
                            }
                            j
                        })
                        .collect(),
                ),
            ));
        }
        if !r.tags.is_empty() {
            o.push((
                "tags",
                Json::Arr(
                    r.tags
                        .iter()
                        .map(|(l, p)| {
                            json::obj(vec![
                                ("label", json::s(l)),
                                ("probability", json::n(round3(*p))),
                            ])
                        })
                        .collect(),
                ),
            ));
        }
        if r.tags_suppressed {
            o.push(("tags_suppressed", Json::Bool(true)));
        }
        o.push((
            "level_dbfs",
            json::n((r.level_dbfs as f64 * 10.0).round() / 10.0),
        ));
        o.push((
            "timings_ms",
            Json::Obj(
                r.timings_ms
                    .iter()
                    .map(|(k, v)| (k.to_string(), json::n((v * 10.0).round() / 10.0)))
                    .collect(),
            ),
        ));
        json::obj(o)
    }
}

fn round3(v: f32) -> f64 {
    (v as f64 * 1000.0).round() / 1000.0
}
