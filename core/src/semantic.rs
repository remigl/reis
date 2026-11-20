//! Semantic engine: "is this the same *kind* of sound?"
//!
//! Each reference is cut into windows; each window becomes an L2-normalised
//! embedding from a pre-trained audio network (EfficientAT by default, see
//! [`crate::nn`]). Vectors are stored as int8 + one f32 scale (≈ 1 byte per
//! dimension). A query is embedded the same way and compared with every
//! reference window (exact cosine search — fast enough up to ~10⁵–10⁶
//! windows on a phone; an approximate index can be added behind the same API).
//!
//! Scores
//! * reference score  = best cosine between any query window and any window of the reference;
//! * label score      = mean of the `top_m` best reference scores of that label
//!   (a k-NN vote that tolerates outliers in the library);
//! * accepted when the best label score ≥ the threshold calibrated on the library
//!   itself at build time (leave-one-out, see [`calibrate`]).

use crate::bytes::*;
use crate::json::{self, Json};
use crate::nn::{l2_normalize, Model};

#[derive(Clone, Debug)]
pub struct SemanticConfig {
    /// window length in seconds (EfficientAT accepts any length; 3 s is a good compromise)
    pub window_s: f32,
    pub hop_s: f32,
    /// at most this many windows per reference (long files are sampled evenly)
    pub max_windows: usize,
    /// network output used as the embedding ("embedding" = penultimate features, "logits" = class scores)
    pub output: String,
    /// label score = mean of the best `top_m` references of that label
    pub top_m: usize,
    /// windows quieter than (loudest window − this many dB) are ignored:
    /// silence looks the same in every category and would create false matches
    pub silence_rel_db: f32,
}

impl Default for SemanticConfig {
    fn default() -> Self {
        SemanticConfig {
            window_s: 3.0,
            hop_s: 1.5,
            max_windows: 32,
            output: "embedding".into(),
            top_m: 3,
            silence_rel_db: 30.0,
        }
    }
}

/// Embeddings of `x` (mono, model sample rate): (start time s, vector) per window.
pub fn embed_windows(model: &Model, x: &[f32], cfg: &SemanticConfig) -> Vec<(f32, Vec<f32>)> {
    let sr = model.frontend.sample_rate as f32;
    let (mel, frames) = model.logmel(x);
    let nm = model.frontend.n_mels;
    let fps = sr / model.frontend.hop as f32;
    let win = ((cfg.window_s * fps) as usize).max(16);
    let hop = ((cfg.hop_s * fps) as usize).max(1);
    let mut starts: Vec<usize> = if frames <= win {
        vec![0]
    } else {
        (0..=frames - win).step_by(hop).collect()
    };
    if frames > win && *starts.last().unwrap() + win < frames {
        starts.push(frames - win); // cover the tail
    }
    // drop near-silent windows (relative to the loudest one; absolute floor -70 dBFS)
    let hop_samples = model.frontend.hop;
    let energy: Vec<f32> = starts
        .iter()
        .map(|&s| {
            let a = (s * hop_samples).min(x.len());
            let b = ((s + win) * hop_samples).min(x.len());
            crate::audio::rms_dbfs(&x[a..b.max(a)])
        })
        .collect();
    let loudest = energy.iter().cloned().fold(f32::MIN, f32::max);
    let keep: Vec<usize> = starts
        .iter()
        .zip(&energy)
        .filter(|(_, &e)| e >= (loudest - cfg.silence_rel_db).max(-70.0))
        .map(|(s, _)| *s)
        .collect();
    if !keep.is_empty() {
        starts = keep;
    } else if let Some(i) = energy
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|v| v.0)
    {
        starts = vec![starts[i]];
    }
    if starts.len() > cfg.max_windows {
        let n = starts.len();
        starts = (0..cfg.max_windows)
            .map(|i| starts[i * (n - 1) / (cfg.max_windows - 1).max(1)])
            .collect();
        starts.dedup();
    }
    starts
        .into_iter()
        .map(|s| {
            let len = win.min(frames - s);
            let mut sub = vec![0f32; nm * len];
            for m in 0..nm {
                sub[m * len..(m + 1) * len]
                    .copy_from_slice(&mel[m * frames + s..m * frames + s + len]);
            }
            let mut v = embedding_of(model, &sub, len, &cfg.output);
            l2_normalize(&mut v);
            (s as f32 / fps, v)
        })
        .collect()
}

fn embedding_of(model: &Model, mel: &[f32], frames: usize, output: &str) -> Vec<f32> {
    let out = model.forward_mel(mel, frames.max(1));
    match output {
        // class probabilities -> log-odds: a "semantic" space aligned with AudioSet labels
        "logits" => out["logits"]
            .iter()
            .map(|p| (p.clamp(1e-6, 1.0 - 1e-6) / (1.0 - p.clamp(1e-6, 1.0 - 1e-6))).ln())
            .collect(),
        "concat" => {
            let mut e = out["embedding"].clone();
            l2_normalize(&mut e);
            let mut l: Vec<f32> = out["logits"]
                .iter()
                .map(|p| (p.clamp(1e-6, 1.0 - 1e-6) / (1.0 - p.clamp(1e-6, 1.0 - 1e-6))).ln())
                .collect();
            l2_normalize(&mut l);
            e.extend(l);
            e
        }
        _ => out["embedding"].clone(),
    }
}

#[derive(Default)]
pub struct SemanticBuilder {
    pub dim: usize,
    raw: Vec<f32>,
    q: Vec<i8>,
    scales: Vec<f32>,
    refs: Vec<u32>,
    times: Vec<f32>,
    pub center: Vec<f32>,
}

/// Subtract the library mean and L2-normalise: network features are all
/// positive (post-activation averages), so raw cosines are crowded near 1;
/// centring spreads them and makes thresholds meaningful.
pub fn center_and_normalize(v: &mut [f32], center: &[f32]) {
    if center.len() == v.len() {
        for (x, c) in v.iter_mut().zip(center) {
            *x -= c;
        }
    }
    l2_normalize(v);
}

impl SemanticBuilder {
    pub fn add(&mut self, ref_id: u32, windows: &[(f32, Vec<f32>)]) {
        for (t, v) in windows {
            if self.dim == 0 {
                self.dim = v.len();
            }
            assert_eq!(v.len(), self.dim);
            self.raw.extend_from_slice(v);
            self.refs.push(ref_id);
            self.times.push(*t);
        }
    }

    /// Centre on the library mean and quantise to int8 (call once, after all `add`).
    pub fn quantize(&mut self) {
        let (n, d) = (self.refs.len(), self.dim);
        let mut c = vec![0f64; d];
        for i in 0..n {
            for k in 0..d {
                c[k] += self.raw[i * d + k] as f64;
            }
        }
        self.center = c.iter().map(|v| (v / n.max(1) as f64) as f32).collect();
        self.q.clear();
        self.scales.clear();
        for i in 0..n {
            let mut v = self.raw[i * d..(i + 1) * d].to_vec();
            center_and_normalize(&mut v, &self.center);
            let m = v.iter().fold(0f32, |a, x| a.max(x.abs())).max(1e-12);
            let s = m / 127.0;
            self.q
                .extend(v.iter().map(|x| (x / s).round().clamp(-127.0, 127.0) as i8));
            self.scales.push(s);
        }
        self.raw = Vec::new();
    }
    pub fn len(&self) -> usize {
        self.refs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.refs.is_empty()
    }

    /// Serialize: u32 header_len | JSON | pad8 | i8 vectors | pad4 | f32 scales | u32 refs | f32 times
    pub fn serialize(&self, header: Json) -> Vec<u8> {
        let mut header = header;
        if let Json::Obj(o) = &mut header {
            o.retain(|(k, _)| k != "center");
            o.push((
                "center".into(),
                Json::Arr(self.center.iter().map(|&v| json::n(v as f64)).collect()),
            ));
        }
        let hj = header.to_string_compact().into_bytes();
        let mut out = Vec::new();
        out.extend_from_slice(&(hj.len() as u32).to_le_bytes());
        out.extend_from_slice(&hj);
        while out.len() % 8 != 0 {
            out.push(0);
        }
        out.extend(self.q.iter().map(|&v| v as u8));
        while out.len() % 4 != 0 {
            out.push(0);
        }
        for s in &self.scales {
            out.extend_from_slice(&s.to_le_bytes());
        }
        for r in &self.refs {
            out.extend_from_slice(&r.to_le_bytes());
        }
        for t in &self.times {
            out.extend_from_slice(&t.to_le_bytes());
        }
        out
    }
}

/// Zero-copy view over a serialized semantic section.
pub struct SemanticView<'a> {
    pub header: Json,
    pub center: Vec<f32>,
    pub dim: usize,
    pub n: usize,
    q: &'a [u8],
    scales: &'a [u8],
    refs: &'a [u8],
    times: &'a [u8],
}

#[derive(Clone, Debug)]
pub struct RefScore {
    pub ref_id: u32,
    pub score: f32,
    /// start (s) of the best-matching window inside the reference
    pub at_s: f32,
}

#[derive(Clone, Debug)]
pub struct LabelScore {
    pub label: u32,
    pub score: f32,
    pub refs: usize,
}

impl<'a> SemanticView<'a> {
    pub fn new(b: &'a [u8]) -> Result<SemanticView<'a>, String> {
        let hl = u32_at(b, 0) as usize;
        let header =
            Json::parse(std::str::from_utf8(&b[4..4 + hl]).map_err(|_| "semantic header")?)?;
        let dim = header.get_usize("dim").ok_or("semantic dim")?;
        let n = header.get_usize("vectors").ok_or("semantic vectors")?;
        let mut p = 4 + hl;
        p += (8 - p % 8) % 8;
        let q = &b[p..p + n * dim];
        p += n * dim;
        p += (4 - p % 4) % 4;
        let scales = &b[p..p + 4 * n];
        let refs = &b[p + 4 * n..p + 8 * n];
        let times = &b[p + 8 * n..p + 12 * n];
        let center = header
            .get("center")
            .and_then(|c| c.arr())
            .map(|a| a.iter().filter_map(|v| v.num()).map(|v| v as f32).collect())
            .unwrap_or_default();
        Ok(SemanticView {
            header,
            center,
            dim,
            n,
            q,
            scales,
            refs,
            times,
        })
    }
    pub fn config(&self) -> SemanticConfig {
        let d = SemanticConfig::default();
        let h = &self.header;
        SemanticConfig {
            window_s: h
                .get_num("window_s")
                .map(|v| v as f32)
                .unwrap_or(d.window_s),
            hop_s: h.get_num("hop_s").map(|v| v as f32).unwrap_or(d.hop_s),
            max_windows: h.get_usize("max_windows").unwrap_or(d.max_windows),
            output: h.get_str("output").unwrap_or("embedding").to_string(),
            top_m: h.get_usize("top_m").unwrap_or(d.top_m),
            silence_rel_db: h
                .get_num("silence_rel_db")
                .map(|v| v as f32)
                .unwrap_or(d.silence_rel_db),
        }
    }
    /// Threshold for a chosen rate of accepted unknown sounds (interpolated on the calibration curve).
    pub fn threshold_for(&self, false_accept: f32) -> Option<f32> {
        let c = self.header.get("calibration")?.get("curve")?.arr()?;
        let pts: Vec<(f32, f32)> = c
            .iter()
            .filter_map(|p| {
                Some((
                    p.get_num("false_accept")? as f32,
                    p.get_num("threshold")? as f32,
                ))
            })
            .collect();
        if pts.is_empty() {
            return None;
        }
        if false_accept <= pts[0].0 {
            return Some(pts[0].1);
        }
        for w in pts.windows(2) {
            if false_accept <= w[1].0 {
                let f = (false_accept - w[0].0) / (w[1].0 - w[0].0);
                return Some(w[0].1 + f * (w[1].1 - w[0].1));
            }
        }
        Some(pts.last().unwrap().1)
    }
    pub fn threshold(&self) -> f32 {
        self.header
            .get("calibration")
            .and_then(|c| c.get_num("threshold"))
            .unwrap_or(0.5) as f32
    }
    #[inline]
    pub fn ref_of(&self, i: usize) -> u32 {
        u32_at(self.refs, 4 * i)
    }
    #[inline]
    fn time_of(&self, i: usize) -> f32 {
        f32::from_le_bytes(self.times[4 * i..4 * i + 4].try_into().unwrap())
    }
    #[inline]
    fn dot(&self, i: usize, v: &[f32]) -> f32 {
        let s = f32::from_le_bytes(self.scales[4 * i..4 * i + 4].try_into().unwrap());
        let row = &self.q[i * self.dim..(i + 1) * self.dim];
        let mut acc = 0f32;
        for k in 0..self.dim {
            acc += (row[k] as i8) as f32 * v[k];
        }
        acc * s
    }
    pub fn vector(&self, i: usize) -> Vec<f32> {
        let s = f32::from_le_bytes(self.scales[4 * i..4 * i + 4].try_into().unwrap());
        self.q[i * self.dim..(i + 1) * self.dim]
            .iter()
            .map(|&b| (b as i8) as f32 * s)
            .collect()
    }

    /// Embed query audio (model rate) into centred, normalised window vectors.
    pub fn query_vectors(&self, model: &Model, x: &[f32]) -> Vec<Vec<f32>> {
        let cfg = self.config();
        embed_windows(model, x, &cfg)
            .into_iter()
            .map(|(_, mut v)| {
                center_and_normalize(&mut v, &self.center);
                v
            })
            .collect()
    }

    /// Best score per reference for a set of query windows. `skip_ref` excludes one reference (leave-one-out).
    pub fn ref_scores(
        &self,
        query: &[Vec<f32>],
        n_refs: usize,
        skip_ref: Option<u32>,
    ) -> Vec<RefScore> {
        let mut best = vec![
            RefScore {
                ref_id: 0,
                score: f32::NEG_INFINITY,
                at_s: 0.0
            };
            n_refs
        ];
        for i in 0..self.n {
            let r = self.ref_of(i);
            if Some(r) == skip_ref {
                continue;
            }
            for qv in query {
                let s = self.dot(i, qv);
                let b = &mut best[r as usize];
                if s > b.score {
                    *b = RefScore {
                        ref_id: r,
                        score: s,
                        at_s: self.time_of(i),
                    };
                }
            }
        }
        let mut v: Vec<RefScore> = best.into_iter().filter(|r| r.score.is_finite()).collect();
        v.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        v
    }

    /// Aggregate reference scores into label scores (labels[ref] = label id).
    pub fn label_scores(&self, refs: &[RefScore], labels: &[u32], top_m: usize) -> Vec<LabelScore> {
        let mut per: std::collections::BTreeMap<u32, Vec<f32>> = Default::default();
        for r in refs {
            per.entry(labels[r.ref_id as usize])
                .or_default()
                .push(r.score); // refs sorted desc
        }
        let mut out: Vec<LabelScore> = per
            .into_iter()
            .map(|(l, s)| {
                let m = top_m.min(s.len()).max(1);
                LabelScore {
                    label: l,
                    score: s[..m].iter().sum::<f32>() / m as f32,
                    refs: s.len(),
                }
            })
            .collect();
        out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        out
    }
}

/// Calibration on the library itself (no extra data needed).
///
/// For a sample of references:
/// * genuine: query with the reference's own vectors, the reference itself
///   left out → is the right label found, with which score?
/// * impostor: same query with *every* reference of its label left out → the
///   best score an unknown sound would get.
///
/// threshold = (1 − max_false_accept) quantile of impostor scores: at most
/// that fraction of sounds absent from the library is accepted.
pub fn calibrate(
    view: &SemanticView,
    labels: &[u32],
    n_refs: usize,
    top_m: usize,
    max_false_accept: f32,
    max_samples: usize,
) -> Json {
    let mut by_ref: Vec<Vec<usize>> = vec![vec![]; n_refs];
    for i in 0..view.n {
        by_ref[view.ref_of(i) as usize].push(i);
    }
    let cand: Vec<usize> = (0..n_refs).filter(|&r| !by_ref[r].is_empty()).collect();
    let step = (cand.len() / max_samples.max(1)).max(1);
    let (mut genuine, mut impostor) = (Vec::new(), Vec::new());
    let mut loo_ok = 0usize;
    for &r in cand.iter().step_by(step) {
        let q: Vec<Vec<f32>> = by_ref[r].iter().map(|&i| view.vector(i)).collect();
        let rs = view.ref_scores(&q, n_refs, Some(r as u32));
        let ls = view.label_scores(&rs, labels, top_m);
        let me = labels[r];
        if let Some(b) = ls.first() {
            if b.label == me {
                loo_ok += 1;
                genuine.push(b.score);
            }
        }
        if let Some(b) = ls.iter().find(|l| l.label != me) {
            impostor.push(b.score);
        }
    }
    if impostor.is_empty() {
        return json::obj(vec![
            ("threshold", json::n(0.5)),
            ("note", json::s("single label: no impostor statistics")),
        ]);
    }
    impostor.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |fa: f32| {
        impostor[(((1.0 - fa) * impostor.len() as f32).ceil() as usize).min(impostor.len() - 1)]
    };
    let thr = at(max_false_accept);
    let curve = Json::Arr(
        [0.01f32, 0.02, 0.05, 0.1, 0.15, 0.2, 0.3, 0.4, 0.5]
            .iter()
            .map(|&fa| {
                let t = at(fa);
                let acc = genuine.iter().filter(|&&g| g >= t).count() as f64
                    / cand.len().div_ceil(step).max(1) as f64;
                json::obj(vec![
                    ("false_accept", json::n(fa as f64)),
                    ("threshold", json::n(t as f64)),
                    ("accepted_correct", json::n(acc)),
                ])
            })
            .collect(),
    );
    let accepted = genuine.iter().filter(|&&g| g >= thr).count() as f64;
    let samples = cand.len().div_ceil(step);
    json::obj(vec![
        ("threshold", json::n(thr as f64)),
        ("max_false_accept", json::n(max_false_accept as f64)),
        (
            "loo_accuracy",
            json::n(loo_ok as f64 / samples.max(1) as f64),
        ),
        (
            "loo_accepted_correct",
            json::n(accepted / samples.max(1) as f64),
        ),
        (
            "genuine_median",
            json::n(genuine.get(genuine.len() / 2).copied().unwrap_or(0.0) as f64),
        ),
        (
            "impostor_median",
            json::n(impostor[impostor.len() / 2] as f64),
        ),
        ("samples", json::n(samples as f64)),
        ("curve", curve),
    ])
}
