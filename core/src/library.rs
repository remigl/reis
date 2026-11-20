//! Sound library: references → fingerprints → one `.reis` file.
//!
//! # Input: a folder
//! ```text
//! sons/
//!   reis.json                 (optional configuration, see LibraryConfig)
//!   animaux/chien/aboiement1.wav     → label "animaux/chien"
//!   animaux/chien/aboiement2.mp3
//!   oiseaux/merle.flac               → label "oiseaux"
//!   oiseaux/merle.json               (optional metadata, any keys; "label"/"name" override)
//!   musique/One More Time.mp3        → label "musique", name from tags
//!   musique/One More Time.lrc        (optional synced lyrics)
//!   musique/cover.jpg                (optional artwork for the folder, or <same name>.jpg)
//! ```
//! # Output: one memory-mappable file
//! `b"REIS" | u32 version=2 | u32 n | n × (tag[4], u64 offset, u64 len)`, sections:
//! * `META` JSON: format, engines, configuration, statistics
//! * `REFS` JSON array: one object per reference {name, label, file, duration, meta, lyrics?, art?}
//! * `EXCT` exact engine: fingerprint parameters + landmark index
//! * `SEMA` semantic engine: embeddings (int8) + calibration
//! * `MODL` neural model (.rnm) used to build `SEMA` — the library is self-contained
//! * `LYRC` lyrics blobs, `ARTW` deduplicated images

use crate::bytes::*;
use crate::fingerprint::{Fingerprinter, FpParams};
use crate::index::{IndexBuilder, IndexView};
use crate::json::{self, Json};
use crate::lyrics::{self, Line, LyricsView};
use crate::nn::Model;
use crate::semantic::{self, SemanticBuilder, SemanticConfig, SemanticView};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const VERSION: u32 = 2;

// ---------------------------------------------------------------- input scan

/// One reference sound found in the input folder.
#[derive(Clone, Debug)]
pub struct SourceItem {
    pub path: PathBuf,
    pub label: String,
    pub name: String,
    pub meta: Vec<(String, Json)>,
    pub lyrics: Option<PathBuf>,
    pub art: Option<PathBuf>,
}

fn sidecar(p: &Path, ext: &str) -> Option<PathBuf> {
    let c = p.with_extension(ext);
    c.exists().then_some(c)
}

/// Walk `root` and describe every audio file (sorted, hidden files skipped).
/// `use_tags`: read title/artist/album with ffprobe (slower; useful for music).
pub fn scan_dir(root: &Path, use_tags: bool) -> Result<Vec<SourceItem>, String> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let rd = std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if crate::decode::is_audio(&p) {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut out = Vec::with_capacity(files.len());
    for p in files {
        let rel = p.strip_prefix(root).unwrap_or(&p);
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let parent: Vec<String> = rel
            .parent()
            .map(|r| {
                r.components()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let mut label = parent.join("/");
        let mut meta: Vec<(String, Json)> = Vec::new();
        if use_tags {
            for (k, v) in crate::decode::tags(&p) {
                if matches!(
                    k.as_str(),
                    "title" | "artist" | "album" | "date" | "genre" | "musicbrainz_trackid"
                ) {
                    meta.push((k, Json::Str(v)));
                }
            }
        }
        let mut name = match (
            meta.iter().find(|m| m.0 == "artist"),
            meta.iter().find(|m| m.0 == "title"),
        ) {
            (Some(a), Some(t)) => {
                format!("{} - {}", a.1.str().unwrap_or(""), t.1.str().unwrap_or(""))
            }
            (None, Some(t)) => t.1.str().unwrap_or("").to_string(),
            _ => stem.clone(),
        };
        if let Some(j) = sidecar(&p, "json") {
            let txt = std::fs::read_to_string(&j).map_err(|e| format!("{}: {e}", j.display()))?;
            let v = Json::parse(&txt).map_err(|e| format!("{}: {e}", j.display()))?;
            for (k, val) in v.obj().unwrap_or(&[]) {
                match (k.as_str(), val.str()) {
                    ("label", Some(s)) => label = s.to_string(),
                    ("name", Some(s)) => name = s.to_string(),
                    _ => {
                        meta.retain(|m| &m.0 != k);
                        meta.push((k.clone(), val.clone()));
                    }
                }
            }
        }
        if label.is_empty() {
            label = name.clone();
        }
        let art = ["jpg", "jpeg", "png", "webp"]
            .iter()
            .find_map(|e| sidecar(&p, e))
            .or_else(|| {
                let d = p.parent()?;
                [
                    "cover.jpg",
                    "cover.png",
                    "folder.jpg",
                    "front.jpg",
                    "cover.webp",
                ]
                .iter()
                .map(|n| d.join(n))
                .find(|c| c.exists())
            });
        out.push(SourceItem {
            lyrics: sidecar(&p, "lrc"),
            art,
            path: p,
            label,
            name,
            meta,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------- configuration

#[derive(Clone, Debug)]
pub struct LibraryConfig {
    /// "exact" (same recording, gives the position), "semantic" (same kind of sound)
    pub engines: Vec<String>,
    pub exact: FpParams,
    pub semantic: SemanticConfig,
    /// fraction of unknown sounds the automatic semantic threshold may accept
    pub max_false_accept: f32,
    pub use_tags: bool,
    pub artwork_px: u32,
    /// ignore audio after this many seconds (0 = everything)
    pub max_secs: f64,
    /// extra semantic vectors from simulated versions of each reference
    /// ("speaker": small speaker + room + phone mic). Improves recognition of
    /// sounds heard through a speaker, makes rejection of unknown sounds harder.
    pub augment: Vec<String>,
}

impl Default for LibraryConfig {
    fn default() -> Self {
        LibraryConfig {
            engines: vec!["exact".into(), "semantic".into()],
            exact: FpParams::default(),
            semantic: SemanticConfig::default(),
            max_false_accept: 0.2, // leave-one-out estimate; conservative (ESC-50: 20 % LOO ≈ 7 % measured)
            use_tags: false,
            artwork_px: 160,
            max_secs: 0.0,
            augment: vec![],
        }
    }
}

impl LibraryConfig {
    /// Apply a `reis.json`-style object on top of the current values.
    pub fn apply_json(&mut self, j: &Json) -> Result<(), String> {
        if let Some(e) = j.get("engines").and_then(|v| v.arr()) {
            self.engines = e.iter().filter_map(|v| v.str().map(String::from)).collect();
        }
        if let Some(x) = j.get("exact") {
            let p = &mut self.exact;
            if let Some(v) = x.get_num("peaks_per_sec") {
                p.peaks_per_sec = v as f32;
            }
            if let Some(v) = x.get_usize("fanout") {
                p.fanout = v;
            }
        }
        if let Some(s) = j.get("semantic") {
            let c = &mut self.semantic;
            if let Some(v) = s.get_num("window_s") {
                c.window_s = v as f32;
            }
            if let Some(v) = s.get_num("hop_s") {
                c.hop_s = v as f32;
            }
            if let Some(v) = s.get_usize("max_windows") {
                c.max_windows = v;
            }
            if let Some(v) = s.get_str("output") {
                c.output = v.into();
            }
            if let Some(v) = s.get_usize("top_m") {
                c.top_m = v;
            }
            if let Some(v) = s.get_num("max_false_accept") {
                self.max_false_accept = v as f32;
            }
        }
        if let Some(v) = j.get("use_tags").and_then(|v| v.bool()) {
            self.use_tags = v;
        }
        if let Some(v) = j.get_num("max_secs") {
            self.max_secs = v;
        }
        if let Some(v) = j.get("augment").and_then(|v| v.arr()) {
            self.augment = v.iter().filter_map(|x| x.str().map(String::from)).collect();
        }
        for e in &self.engines {
            if e != "exact" && e != "semantic" {
                return Err(format!("unknown engine '{e}' (available: exact, semantic)"));
            }
        }
        for a in &self.augment {
            if a != "speaker" {
                return Err(format!("unknown augmentation '{a}' (available: speaker)"));
            }
        }
        Ok(())
    }
    pub fn has(&self, e: &str) -> bool {
        self.engines.iter().any(|x| x == e)
    }
}

// ---------------------------------------------------------------- builder

pub struct LibraryBuilder {
    pub cfg: LibraryConfig,
    model_bytes: Option<Vec<u8>>,
    model: Option<Model>,
    exact_fp: Fingerprinter,
    exact: IndexBuilder,
    sem: SemanticBuilder,
    refs: Vec<Json>,
    labels: Vec<String>,
    lyrc: Vec<u8>,
    art: Vec<Vec<u8>>,
    art_ix: HashMap<Vec<u8>, u32>,
    pub audio_s: f64,
}

/// Fingerprints of one reference, computed independently (parallelisable).
pub struct Prepared {
    pub duration_s: f64,
    pub exact: Vec<(u32, u32)>,
    pub semantic: Vec<(f32, Vec<f32>)>,
}

impl LibraryBuilder {
    pub fn new(cfg: LibraryConfig, model_bytes: Option<Vec<u8>>) -> Result<Self, String> {
        let model = match (&model_bytes, cfg.has("semantic")) {
            (Some(b), true) => Some(Model::from_bytes(b)?),
            (None, true) => {
                return Err("the semantic engine needs a model (.rnm), see --model".into())
            }
            _ => None,
        };
        Ok(LibraryBuilder {
            exact_fp: Fingerprinter::new(cfg.exact),
            cfg,
            model_bytes,
            model,
            exact: IndexBuilder::default(),
            sem: SemanticBuilder::default(),
            refs: vec![],
            labels: vec![],
            lyrc: vec![],
            art: vec![],
            art_ix: HashMap::new(),
            audio_s: 0.0,
        })
    }

    /// Sample rate `prepare` expects.
    pub fn input_rate(&self) -> u32 {
        self.model
            .as_ref()
            .map(|m| m.frontend.sample_rate)
            .unwrap_or(self.cfg.exact.sample_rate)
    }

    /// Thread-safe part: compute all fingerprints of one decoded file.
    pub fn prepare(&self, audio: &[f32], rate: u32) -> Prepared {
        let exact = if self.cfg.has("exact") {
            let x8 = crate::audio::resample(audio, rate, self.cfg.exact.sample_rate);
            self.exact_fp.hashes(&x8)
        } else {
            vec![]
        };
        let semantic = match &self.model {
            Some(m) => {
                let x = if rate == m.frontend.sample_rate {
                    audio.to_vec()
                } else {
                    crate::audio::resample(audio, rate, m.frontend.sample_rate)
                };
                let mut w = semantic::embed_windows(m, &x, &self.cfg.semantic);
                if self.cfg.augment.iter().any(|a| a == "speaker") {
                    let seed = x.len() as u64
                        ^ x.iter()
                            .take(4096)
                            .map(|v| v.to_bits() as u64)
                            .fold(0u64, |a: u64, b| a.rotate_left(5) ^ b);
                    let y = crate::audio::speaker_sim(&x, m.frontend.sample_rate, seed);
                    w.extend(semantic::embed_windows(m, &y, &self.cfg.semantic));
                }
                w
            }
            None => vec![],
        };
        Prepared {
            duration_s: audio.len() as f64 / rate as f64,
            exact,
            semantic,
        }
    }

    /// Decode + prepare a file from disk.
    pub fn prepare_file(&self, p: &Path) -> Result<Prepared, String> {
        let r = self.input_rate();
        let x = crate::decode::load(p, r, self.cfg.max_secs)?;
        if x.len() < (r as usize) / 4 {
            return Err(format!("{}: shorter than 0.25 s", p.display()));
        }
        Ok(self.prepare(&x, r))
    }

    /// Add one reference. Returns its id.
    pub fn add(&mut self, item: &SourceItem, prep: Prepared) -> u32 {
        let id = self.refs.len() as u32;
        if self.cfg.has("exact") {
            self.exact.add(id, &prep.exact);
        }
        self.sem.add(id, &prep.semantic);
        self.audio_s += prep.duration_s;
        let mut r = vec![
            ("name".to_string(), json::s(&item.name)),
            ("label".to_string(), json::s(&item.label)),
            (
                "file".to_string(),
                json::s(
                    &item
                        .path
                        .file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_default(),
                ),
            ),
            (
                "duration".to_string(),
                json::n((prep.duration_s * 1000.0).round() / 1000.0),
            ),
        ];
        if !item.meta.is_empty() {
            r.push(("meta".into(), Json::Obj(item.meta.clone())));
        }
        if let Some(l) = item
            .lyrics
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
        {
            let lines: Vec<Line> = lyrics::parse_lrc(&l);
            if !lines.is_empty() {
                r.push(("lyrics".into(), json::n(self.lyrc.len() as f64)));
                self.lyrc.extend(lyrics::encode(&lines));
            }
        }
        if let Some(img) = item
            .art
            .as_ref()
            .and_then(|p| crate::decode::thumbnail(p, self.cfg.artwork_px))
        {
            let n = self.art.len() as u32;
            let aid = *self.art_ix.entry(img.clone()).or_insert(n);
            if aid == n {
                self.art.push(img);
            }
            r.push(("art".into(), json::n(aid as f64)));
        }
        if !self.labels.contains(&item.label) {
            self.labels.push(item.label.clone());
        }
        self.refs.push(Json::Obj(r));
        id
    }

    pub fn len(&self) -> usize {
        self.refs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.refs.is_empty()
    }

    pub fn finish(self) -> Vec<u8> {
        let cfg = &self.cfg;
        let mut sections: Vec<([u8; 4], Vec<u8>)> = Vec::new();
        let n_refs = self.refs.len();
        let mut stats = vec![
            ("references", json::n(n_refs as f64)),
            ("labels", json::n(self.labels.len() as f64)),
            ("audio_seconds", json::n(self.audio_s.round())),
        ];
        if cfg.has("exact") {
            let mut b = cfg.exact.to_bytes();
            let postings = self.exact.len();
            b.extend(self.exact.serialize());
            stats.push(("exact_postings", json::n(postings as f64)));
            sections.push((*b"EXCT", b));
        }
        if cfg.has("semantic") && !self.sem.is_empty() {
            let model = self.model.as_ref().unwrap();
            // calibration needs label ids per reference
            let label_ids: Vec<u32> = self
                .refs
                .iter()
                .map(|r| {
                    self.labels
                        .iter()
                        .position(|l| Some(l.as_str()) == r.get_str("label"))
                        .unwrap() as u32
                })
                .collect();
            let mut header = vec![
                ("model", json::s(&model.name)),
                ("dim", json::n(self.sem.dim as f64)),
                ("vectors", json::n(self.sem.len() as f64)),
                ("quant", json::s("int8")),
                ("window_s", json::n(cfg.semantic.window_s as f64)),
                ("hop_s", json::n(cfg.semantic.hop_s as f64)),
                ("max_windows", json::n(cfg.semantic.max_windows as f64)),
                ("output", json::s(&cfg.semantic.output)),
                ("top_m", json::n(cfg.semantic.top_m as f64)),
                (
                    "silence_rel_db",
                    json::n(cfg.semantic.silence_rel_db as f64),
                ),
                (
                    "augment",
                    Json::Arr(cfg.augment.iter().map(|a| json::s(a)).collect()),
                ),
            ];
            let mut sem = self.sem;
            sem.quantize();
            let tmp = sem.serialize(json::obj(header.clone()));
            let view = SemanticView::new(&tmp).unwrap();
            let cal = semantic::calibrate(
                &view,
                &label_ids,
                n_refs,
                cfg.semantic.top_m,
                cfg.max_false_accept,
                2000,
            );
            header.push(("calibration", cal));
            stats.push(("semantic_vectors", json::n(sem.len() as f64)));
            sections.push((*b"SEMA", sem.serialize(json::obj(header))));
            sections.push((*b"MODL", self.model_bytes.clone().unwrap()));
        }
        let mut artw = Vec::new();
        artw.extend_from_slice(&(self.art.len() as u32).to_le_bytes());
        let mut off = 0u32;
        for a in &self.art {
            artw.extend_from_slice(&off.to_le_bytes());
            off += a.len() as u32;
        }
        artw.extend_from_slice(&off.to_le_bytes());
        for a in &self.art {
            artw.extend_from_slice(a);
        }
        sections.push((*b"LYRC", self.lyrc));
        sections.push((*b"ARTW", artw));
        let meta = json::obj(vec![
            ("format", json::s("reis-library")),
            ("version", json::n(VERSION as f64)),
            (
                "engines",
                Json::Arr(cfg.engines.iter().map(|e| json::s(e)).collect()),
            ),
            ("stats", json::obj(stats)),
        ]);
        sections.insert(
            0,
            (
                *b"REFS",
                Json::Arr(self.refs).to_string_compact().into_bytes(),
            ),
        );
        sections.insert(0, (*b"META", meta.to_string_pretty().into_bytes()));
        write_container(&sections)
    }
}

fn write_container(secs: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let head = 12 + secs.len() * 20;
    let mut out = Vec::new();
    out.extend_from_slice(b"REIS");
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(secs.len() as u32).to_le_bytes());
    let mut o = head as u64;
    let mut pads = vec![];
    for (tag, d) in secs {
        let pad = (16 - (o % 16)) % 16;
        o += pad;
        pads.push(pad);
        out.extend_from_slice(tag);
        out.extend_from_slice(&o.to_le_bytes());
        out.extend_from_slice(&(d.len() as u64).to_le_bytes());
        o += d.len() as u64;
    }
    for ((_, d), pad) in secs.iter().zip(pads) {
        out.extend(std::iter::repeat_n(0u8, pad as usize));
        out.extend_from_slice(d);
    }
    out
}

// ---------------------------------------------------------------- reading

#[derive(Clone, Debug)]
pub struct Reference {
    pub name: String,
    pub label: String,
    pub label_id: u32,
    pub file: String,
    pub duration_ms: u32,
    pub meta: Json,
    lyrics: Option<usize>,
    art: Option<u32>,
}

impl Reference {
    pub fn meta_str(&self, k: &str) -> Option<&str> {
        self.meta.get_str(k)
    }
}

pub struct Library {
    blob: Blob,
    secs: HashMap<[u8; 4], (usize, usize)>,
    pub meta: Json,
    pub refs: Vec<Reference>,
    pub labels: Vec<String>,
    /// fingerprint parameters of the exact engine (None if absent)
    pub params: Option<FpParams>,
}

impl Library {
    pub fn open(path: &str) -> Result<Library, String> {
        let blob = Blob::open(path).map_err(|e| format!("{path}: {e}"))?;
        let b = blob.bytes();
        if b.len() < 12 || &b[0..4] != b"REIS" {
            return Err(format!("{path}: not a Reis library"));
        }
        let v = u32_at(b, 4);
        if v != VERSION {
            return Err(format!("{path}: library format v{v}, this build reads v{VERSION} — rebuild it with `reis index`"));
        }
        let n = u32_at(b, 8) as usize;
        let mut secs = HashMap::new();
        for i in 0..n {
            let e = 12 + i * 20;
            let tag: [u8; 4] = b[e..e + 4].try_into().unwrap();
            let (o, l) = (u64_at(b, e + 4) as usize, u64_at(b, e + 12) as usize);
            if o + l > b.len() {
                return Err(format!("{path}: truncated library"));
            }
            secs.insert(tag, (o, l));
        }
        let sec = |t: &[u8; 4]| secs.get(t).map(|&(o, l)| &b[o..o + l]);
        let meta = Json::parse(
            std::str::from_utf8(sec(b"META").ok_or("no META")?).map_err(|_| "META utf8")?,
        )?;
        let refs_j = Json::parse(
            std::str::from_utf8(sec(b"REFS").ok_or("no REFS")?).map_err(|_| "REFS utf8")?,
        )?;
        let mut labels: Vec<String> = Vec::new();
        let mut li: HashMap<String, u32> = HashMap::new();
        let mut refs = Vec::new();
        for r in refs_j.arr().unwrap_or(&[]) {
            let label = r.get_str("label").unwrap_or("").to_string();
            let id = *li.entry(label.clone()).or_insert_with(|| {
                labels.push(label.clone());
                (labels.len() - 1) as u32
            });
            refs.push(Reference {
                name: r.get_str("name").unwrap_or("").to_string(),
                label,
                label_id: id,
                file: r.get_str("file").unwrap_or("").to_string(),
                duration_ms: (r.get_num("duration").unwrap_or(0.0) * 1000.0) as u32,
                meta: r.get("meta").cloned().unwrap_or(Json::Obj(vec![])),
                lyrics: r.get_usize("lyrics"),
                art: r.get_num("art").map(|v| v as u32),
            });
        }
        let params = sec(b"EXCT").map(|s| FpParams::from_bytes(s));
        Ok(Library {
            blob,
            secs,
            meta,
            refs,
            labels,
            params,
        })
    }

    fn section(&self, t: &[u8; 4]) -> Option<&[u8]> {
        self.secs.get(t).map(|&(o, l)| &self.blob.bytes()[o..o + l])
    }
    pub fn section_sizes(&self) -> Vec<(String, usize)> {
        let mut v: Vec<_> = self
            .secs
            .iter()
            .map(|(k, v)| (String::from_utf8_lossy(k).to_string(), v.1))
            .collect();
        v.sort();
        v
    }
    pub fn file_size(&self) -> usize {
        self.blob.bytes().len()
    }
    pub fn has_exact(&self) -> bool {
        self.params.is_some()
    }
    pub fn has_semantic(&self) -> bool {
        self.secs.contains_key(b"SEMA")
    }
    /// Landmark index of the exact engine. Panics if the library has none.
    pub fn index(&self) -> IndexView<'_> {
        IndexView::new(
            &self
                .section(b"EXCT")
                .expect("library built without the exact engine")[FpParams::BYTES..],
        )
    }
    pub fn semantic(&self) -> Option<SemanticView<'_>> {
        self.section(b"SEMA")
            .and_then(|b| SemanticView::new(b).ok())
    }
    /// Neural model embedded in the library.
    pub fn model(&self) -> Option<Result<Model, String>> {
        self.section(b"MODL").map(Model::from_bytes)
    }
    pub fn label_ids(&self) -> Vec<u32> {
        self.refs.iter().map(|r| r.label_id).collect()
    }

    // --- per-reference accessors (names kept short for the FFI)
    pub fn song_count(&self) -> u32 {
        self.refs.len() as u32
    }
    pub fn title(&self, id: u32) -> &str {
        let r = &self.refs[id as usize];
        r.meta_str("title").unwrap_or(&r.name)
    }
    pub fn artist(&self, id: u32) -> &str {
        self.refs[id as usize].meta_str("artist").unwrap_or("")
    }
    pub fn album(&self, id: u32) -> &str {
        self.refs[id as usize].meta_str("album").unwrap_or("")
    }
    pub fn duration_ms(&self, id: u32) -> u32 {
        self.refs[id as usize].duration_ms
    }
    pub fn lyrics(&self, id: u32) -> Option<LyricsView<'_>> {
        let o = self.refs.get(id as usize)?.lyrics?;
        Some(lyrics::decode(&self.section(b"LYRC")?[o..]))
    }
    pub fn artwork(&self, id: u32) -> Option<&[u8]> {
        let a = self.refs.get(id as usize)?.art? as usize;
        let s = self.section(b"ARTW")?;
        let n = u32_at(s, 0) as usize;
        if a >= n {
            return None;
        }
        let base = 4 + (n + 1) * 4;
        let (o, e) = (u32_at(s, 4 + a * 4) as usize, u32_at(s, 8 + a * 4) as usize);
        Some(&s[base + o..base + e])
    }
}
