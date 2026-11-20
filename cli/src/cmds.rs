//! Everyday commands: index, recognize, tag, eval, info, lyrics, listen.

use crate::util::*;
use reis_core::engine::{Engine, EngineConfig, Phase};
use reis_core::json::{self, Json};
use reis_core::library::{scan_dir, Library, LibraryBuilder, LibraryConfig, Prepared};
use reis_core::nn::Model;
use reis_core::recognize::{Recognition, Recognizer, RecognizerConfig, Verdict};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const DEFAULT_MODEL: &str = "mn10_as.rnm";

/// --model, $REIS_MODEL, ./models/, <exe>/models/, ~/.reis/models/
pub fn find_model(a: &[String]) -> Option<PathBuf> {
    if let Some(m) = arg(a, "--model") {
        return Some(PathBuf::from(m));
    }
    if let Ok(m) = std::env::var("REIS_MODEL") {
        return Some(PathBuf::from(m));
    }
    let mut c = vec![PathBuf::from("models").join(DEFAULT_MODEL)];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            c.push(d.join("models").join(DEFAULT_MODEL));
            c.push(d.join(DEFAULT_MODEL));
        }
    }
    if let Some(h) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        c.push(
            PathBuf::from(h)
                .join(".reis")
                .join("models")
                .join(DEFAULT_MODEL),
        );
    }
    c.into_iter().find(|p| p.exists())
}

fn no_model_help() -> String {
    format!(
        "no neural model found for the semantic engine.\n  Download {DEFAULT_MODEL} from https://github.com/remigl/reis/releases/tag/models-v1\n  and put it in ./models/, next to the reis executable, or pass --model PATH.\n  (or index with --engines exact to use only exact matching)"
    )
}

// ------------------------------------------------------------------ index

pub fn cmd_index(a: &[String]) {
    let pos = positional(
        a,
        &[
            "-o",
            "--out",
            "--engines",
            "--model",
            "--window",
            "--hop",
            "--max-windows",
            "--max-secs",
            "--false-accept",
            "--output",
            "--threads",
            "--pps",
            "--fanout",
            "--augment",
        ],
    );
    let dir = PathBuf::from(
        pos.first()
            .unwrap_or_else(|| die("usage: reis index DIR -o library.reis")),
    );
    let out = arg(a, "-o")
        .or_else(|| arg(a, "--out"))
        .unwrap_or_else(|| "library.reis".into());
    let mut cfg = LibraryConfig::default();
    if let Ok(txt) = std::fs::read_to_string(dir.join("reis.json")) {
        let j = Json::parse(&txt).unwrap_or_else(|e| die(format!("reis.json: {e}")));
        cfg.apply_json(&j)
            .unwrap_or_else(|e| die(format!("reis.json: {e}")));
        eprintln!("using {}", dir.join("reis.json").display());
    }
    if let Some(e) = arg(a, "--engines") {
        cfg.engines = e.split(',').map(|s| s.trim().to_string()).collect();
    }
    cfg.semantic.window_s = arg_parse(a, "--window", cfg.semantic.window_s);
    cfg.semantic.hop_s = arg_parse(a, "--hop", cfg.semantic.hop_s);
    cfg.semantic.max_windows = arg_parse(a, "--max-windows", cfg.semantic.max_windows);
    if let Some(o) = arg(a, "--output") {
        cfg.semantic.output = o;
    }
    cfg.exact.peaks_per_sec = arg_parse(a, "--pps", cfg.exact.peaks_per_sec);
    cfg.exact.fanout = arg_parse(a, "--fanout", cfg.exact.fanout);
    cfg.max_secs = arg_parse(a, "--max-secs", cfg.max_secs);
    cfg.max_false_accept = arg_parse(a, "--false-accept", cfg.max_false_accept);
    if flag(a, "--tags") {
        cfg.use_tags = true;
    }
    if let Some(v) = arg(a, "--augment") {
        cfg.augment = v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "none")
            .collect();
    }
    for e in &cfg.engines {
        if e != "exact" && e != "semantic" {
            die(format!("unknown engine '{e}' (available: exact, semantic)"));
        }
    }
    let model_bytes = if cfg.has("semantic") {
        let p = find_model(a).unwrap_or_else(|| die(no_model_help()));
        eprintln!("model {}", p.display());
        Some(std::fs::read(&p).unwrap_or_else(|e| die(format!("{}: {e}", p.display()))))
    } else {
        None
    };
    let items = scan_dir(&dir, cfg.use_tags).unwrap_or_else(|e| die(e));
    if items.is_empty() {
        die(format!("no audio files found in {}", dir.display()));
    }
    let labels: std::collections::BTreeSet<&str> = items.iter().map(|i| i.label.as_str()).collect();
    eprintln!(
        "{} reference files, {} labels, engines: {}",
        items.len(),
        labels.len(),
        cfg.engines.join(" + ")
    );
    let threads: usize = arg_parse(
        a,
        "--threads",
        std::thread::available_parallelism()
            .map(|v| v.get())
            .unwrap_or(1),
    );
    let mut b = LibraryBuilder::new(cfg, model_bytes).unwrap_or_else(|e| die(e));
    let t0 = Instant::now();
    let mut skipped = 0;
    const BATCH: usize = 64;
    for (bi, chunk) in items.chunks(BATCH).enumerate() {
        let next = AtomicUsize::new(0);
        let slots: Vec<Mutex<Option<Result<Prepared, String>>>> =
            (0..chunk.len()).map(|_| Mutex::new(None)).collect();
        let bref = &b;
        std::thread::scope(|s| {
            for _ in 0..threads.min(chunk.len()) {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= chunk.len() {
                        break;
                    }
                    *slots[i].lock().unwrap() = Some(bref.prepare_file(&chunk[i].path));
                });
            }
        });
        for (k, sl) in slots.into_iter().enumerate() {
            match sl.into_inner().unwrap().unwrap() {
                Ok(p) => {
                    b.add(&chunk[k], p);
                }
                Err(e) => {
                    eprintln!("\n  skipped: {e}");
                    skipped += 1;
                }
            }
        }
        progress(
            (bi * BATCH + chunk.len()).min(items.len()),
            items.len(),
            "fingerprinted",
        );
    }
    if b.is_empty() {
        die("no reference could be decoded");
    }
    let audio_s = b.audio_s;
    let bytes = b.finish();
    std::fs::write(&out, &bytes).unwrap_or_else(|e| die(format!("{out}: {e}")));
    let el = t0.elapsed().as_secs_f64();
    eprintln!(
        "wrote {out}: {:.2} MB, {:.0} min of audio in {:.1} s ({:.0}x real time){}",
        bytes.len() as f64 / 1e6,
        audio_s / 60.0,
        el,
        audio_s / el,
        if skipped > 0 {
            format!(", {skipped} skipped")
        } else {
            String::new()
        }
    );
    print_info(&Library::open(&out).unwrap_or_else(|e| die(e)));
}

// ------------------------------------------------------------------ info / lyrics

pub fn print_info(lib: &Library) {
    let st = lib.meta.get("stats");
    println!(
        "library: {} references, {} labels, engines {}, {:.2} MB",
        lib.refs.len(),
        lib.labels.len(),
        lib.meta
            .get("engines")
            .and_then(|e| e.arr())
            .map(|e| e
                .iter()
                .filter_map(|v| v.str())
                .collect::<Vec<_>>()
                .join(" + "))
            .unwrap_or_default(),
        lib.file_size() as f64 / 1e6
    );
    if let Some(s) = st {
        println!(
            "  audio: {:.1} min",
            s.get_num("audio_seconds").unwrap_or(0.0) / 60.0
        );
    }
    for (k, v) in lib.section_sizes() {
        println!("  section {k}: {:.3} MB", v as f64 / 1e6);
    }
    if let Some(sem) = lib.semantic() {
        let c = sem.header.get("calibration");
        println!(
            "  semantic: model {}, {} vectors x {} dims, window {} s; threshold {:.3} (≤ {:.0}% unknown sounds accepted), leave-one-out accuracy {:.1}%",
            sem.header.get_str("model").unwrap_or("?"),
            sem.n,
            sem.dim,
            sem.header.get_num("window_s").unwrap_or(0.0),
            sem.threshold(),
            100.0 * c.and_then(|c| c.get_num("max_false_accept")).unwrap_or(0.0),
            100.0 * c.and_then(|c| c.get_num("loo_accuracy")).unwrap_or(0.0)
        );
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &lib.refs {
        *counts.entry(&r.label).or_default() += 1;
    }
    let shown: Vec<String> = counts
        .iter()
        .take(12)
        .map(|(l, n)| format!("{l} ({n})"))
        .collect();
    println!(
        "  labels: {}{}",
        shown.join(", "),
        if counts.len() > 12 {
            format!(", … +{}", counts.len() - 12)
        } else {
            String::new()
        }
    );
}

pub fn cmd_info(a: &[String]) {
    let lib = open_lib(&a[0]);
    print_info(&lib);
    if flag(a, "--refs") {
        for (i, r) in lib.refs.iter().enumerate() {
            println!(
                "{i}\t{}\t{}\t{:.1}s\t{}",
                r.label,
                r.name,
                r.duration_ms as f64 / 1000.0,
                r.file
            );
        }
    }
}

pub fn cmd_lyrics(a: &[String]) {
    let lib = open_lib(&a[0]);
    let id: u32 = a
        .get(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| die("usage: reis lyrics LIB REF_ID"));
    println!("{}", lib.refs[id as usize].name);
    match lib.lyrics(id) {
        Some(l) => {
            for (t, s, _) in &l.lines {
                println!(
                    "[{:02}:{:05.2}] {s}",
                    t / 60000,
                    (t % 60000) as f64 / 1000.0
                );
            }
        }
        None => println!("(no lyrics)"),
    }
}

// ------------------------------------------------------------------ recognize / tag

fn rec_config(a: &[String]) -> RecognizerConfig {
    let mut c = RecognizerConfig::default();
    if let Some(e) = arg(a, "--engines") {
        c.engines = e.split(',').map(|s| s.trim().to_string()).collect();
    }
    c.top = arg_parse(a, "--top", c.top);
    c.semantic_threshold = arg(a, "--threshold").and_then(|v| v.parse().ok());
    c.false_accept = arg(a, "--false-accept").and_then(|v| v.parse().ok());
    c.tag_threshold = arg_parse(a, "--tag-threshold", c.tag_threshold);
    c
}

fn describe(rec: &Recognizer, r: &Recognition) -> String {
    let lib = &rec.lib;
    let mut s = String::new();
    match r.verdict.as_ref().unwrap() {
        Verdict::Exact { ref_id, offset_s } => {
            let rf = &lib.refs[*ref_id as usize];
            s += &format!(
                "✓ exact: \"{}\" [{}] at {:.2} s",
                rf.name, rf.label, offset_s
            );
        }
        Verdict::Similar { label, score } => {
            s += &format!(
                "≈ similar: {label} (score {score:.2}, threshold {:.2})",
                r.semantic_threshold
            )
        }
        Verdict::Tag { label, prob } => {
            s += &format!(
                "? not in the library — sounds like: {label} ({:.0} %)",
                prob * 100.0
            )
        }
        Verdict::Silence => s += &format!("· silence ({:.0} dBFS)", r.level_dbfs),
        Verdict::Unknown => s += "✗ unknown",
    }
    if !r.similar.is_empty() {
        let v: Vec<String> = r
            .similar
            .iter()
            .take(3)
            .map(|(l, sc, _)| format!("{l} {sc:.2}"))
            .collect();
        s += &format!("\n    labels: {}", v.join(" · "));
        let n: Vec<String> = r
            .neighbours
            .iter()
            .take(3)
            .map(|n| format!("{} {:.2}", lib.refs[n.ref_id as usize].file, n.score))
            .collect();
        s += &format!("\n    nearest: {}", n.join(" · "));
    }
    if let Some(e) = &r.exact {
        if !e.accepted {
            s += &format!(
                "\n    exact: best \"{}\" score {} (second {}) — not accepted",
                lib.refs[e.ref_id as usize].name, e.score, e.second
            );
        }
    }
    if r.tags_suppressed {
        s += "\n    tags: none reliable (input looks like stationary noise)";
    }
    if !r.tags.is_empty() {
        let v: Vec<String> = r
            .tags
            .iter()
            .take(3)
            .map(|(l, p)| format!("{l} {:.0}%", p * 100.0))
            .collect();
        s += &format!("\n    tags: {}", v.join(" · "));
    }
    let t: Vec<String> = r
        .timings_ms
        .iter()
        .map(|(k, v)| format!("{k} {v:.0} ms"))
        .collect();
    s += &format!("\n    ({})", t.join(", "));
    s
}

pub fn cmd_recognize(a: &[String]) {
    let pos = positional(
        a,
        &[
            "--engines",
            "--top",
            "--threshold",
            "--false-accept",
            "--tag-threshold",
            "--start",
            "--duration",
            "--segments",
            "--model",
        ],
    );
    if pos.len() < 2 {
        die("usage: reis recognize LIB FILE... [--json] [--segments S]");
    }
    let lib = Arc::new(open_lib(&pos[0]));
    let mut rec = Recognizer::new(lib, rec_config(a)).unwrap_or_else(|e| die(e));
    if rec.model.is_none() {
        if let Some(p) = find_model(a) {
            rec = rec.with_model(Model::load(p.to_str().unwrap()).unwrap_or_else(|e| die(e)));
        }
    }
    let json_out = flag(a, "--json");
    let start: f64 = arg_parse(a, "--start", 0.0);
    let dur: f64 = arg_parse(a, "--duration", 0.0);
    let seg: f64 = arg_parse(a, "--segments", 0.0);
    let rate = 32000;
    for f in &pos[1..] {
        let x = load_audio(
            Path::new(f),
            rate,
            if dur > 0.0 { start + dur } else { 0.0 },
        )
        .unwrap_or_else(|e| die(e));
        let x = &x[((start * rate as f64) as usize).min(x.len())..];
        let pieces: Vec<(f64, &[f32])> = if seg > 0.0 {
            x.chunks((seg * rate as f64) as usize)
                .enumerate()
                .filter(|(_, c)| c.len() > rate as usize / 2)
                .map(|(i, c)| (start + i as f64 * seg, c))
                .collect()
        } else {
            vec![(start, x)]
        };
        for (t, p) in pieces {
            let r = rec.recognize(p, rate);
            if json_out {
                let mut j = rec.to_json(&r);
                if let Json::Obj(v) = &mut j {
                    v.insert(0, ("file".into(), json::s(f)));
                    v.insert(1, ("start_s".into(), json::n(t)));
                }
                println!("{}", j.to_string_compact());
            } else if seg > 0.0 {
                println!(
                    "{f} @ {:>7.1} s  {}",
                    t,
                    describe(&rec, &r).replace('\n', "\n                 ")
                );
            } else {
                println!("{f}\n  {}", describe(&rec, &r).replace('\n', "\n  "));
            }
        }
    }
}

/// General AudioSet tagging, no library needed.
pub fn cmd_tag(a: &[String]) {
    let pos = positional(a, &["--model", "--top"]);
    let p = find_model(a).unwrap_or_else(|| die(no_model_help()));
    let m = Model::load(p.to_str().unwrap()).unwrap_or_else(|e| die(e));
    let top: usize = arg_parse(a, "--top", 5);
    for f in &pos {
        let x = load_audio(Path::new(f), m.frontend.sample_rate, 0.0).unwrap_or_else(|e| die(e));
        let t = Instant::now();
        let out = m.forward_audio(&x, m.frontend.sample_rate);
        let p = &out["logits"];
        if p.iter().filter(|&&v| v > 0.9).count() >= 5 {
            println!("{f}: no reliable tag (input looks like stationary noise / silence)");
            continue;
        }
        let mut idx: Vec<usize> = (0..p.len()).collect();
        idx.sort_by(|a, b| p[*b].partial_cmp(&p[*a]).unwrap());
        let v: Vec<String> = idx
            .iter()
            .take(top)
            .map(|&i| format!("{} {:.0}%", m.labels[i], p[i] * 100.0))
            .collect();
        println!(
            "{f}: {}   ({:.0} ms)",
            v.join(" · "),
            t.elapsed().as_secs_f64() * 1e3
        );
    }
}

// ------------------------------------------------------------------ eval

/// Evaluate on a folder of labelled queries (same layout as `reis index`).
/// Labels absent from the library count as "should be rejected" (open set).
pub fn cmd_eval(a: &[String]) {
    let pos = positional(
        a,
        &[
            "--engines",
            "--top",
            "--threshold",
            "--false-accept",
            "--tag-threshold",
            "--csv",
            "--model",
        ],
    );
    if pos.len() < 2 {
        die("usage: reis eval LIB QUERY_DIR [--csv results.csv]");
    }
    let lib = Arc::new(open_lib(&pos[0]));
    let mut cfg = rec_config(a);
    if arg(a, "--engines").is_none() {
        cfg.engines = vec!["exact".into(), "semantic".into()];
    }
    let rec = Recognizer::new(lib.clone(), cfg).unwrap_or_else(|e| die(e));
    let items = scan_dir(Path::new(&pos[1]), false).unwrap_or_else(|e| die(e));
    let known: std::collections::HashSet<&str> = lib.labels.iter().map(|s| s.as_str()).collect();
    let (mut n_known, mut top1, mut top5, mut wrong, mut n_unk, mut false_acc) = (0, 0, 0, 0, 0, 0);
    let mut per: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    let mut times = vec![];
    let mut csv = String::from(
        "file,expected,predicted,kind,score,top_label,top_score,second_score,in_top5\n",
    );
    let mut confusion: BTreeMap<(String, String), u32> = BTreeMap::new();
    for (k, it) in items.iter().enumerate() {
        let x = load_audio(&it.path, 32000, 0.0).unwrap_or_else(|e| die(e));
        let t = Instant::now();
        let r = rec.recognize(&x, 32000);
        times.push(t.elapsed().as_secs_f64() * 1e3);
        let (pred, kind, score) = match r.verdict.as_ref().unwrap() {
            Verdict::Exact { ref_id, .. } => (
                lib.refs[*ref_id as usize].label.clone(),
                "exact",
                r.exact.as_ref().unwrap().score as f32,
            ),
            Verdict::Similar { label, score } => (label.clone(), "similar", *score),
            _ => (
                String::new(),
                "rejected",
                r.similar.first().map(|s| s.1).unwrap_or(0.0),
            ),
        };
        let (tl, ts) = r
            .similar
            .first()
            .map(|s| (s.0.clone(), s.1))
            .unwrap_or_default();
        let s2 = r.similar.get(1).map(|s| s.1).unwrap_or(0.0);
        let in5 = r.similar.iter().any(|s| s.0 == it.label) as u8;
        csv += &format!(
            "\"{}\",\"{}\",\"{}\",{kind},{score:.3},\"{tl}\",{ts:.4},{s2:.4},{in5}\n",
            it.path.display(),
            it.label,
            pred
        );
        if known.contains(it.label.as_str()) {
            n_known += 1;
            let e = per.entry(it.label.clone()).or_default();
            e.1 += 1;
            if pred == it.label {
                top1 += 1;
                e.0 += 1;
            } else if !pred.is_empty() {
                wrong += 1;
                *confusion
                    .entry((it.label.clone(), pred.clone()))
                    .or_default() += 1;
            }
            if r.similar.iter().any(|s| s.0 == it.label) || pred == it.label {
                top5 += 1;
            }
        } else {
            n_unk += 1;
            if !pred.is_empty() {
                false_acc += 1;
            }
        }
        progress(k + 1, items.len(), "queries");
    }
    let pc = |a: u32, b: u32| {
        if b == 0 {
            f64::NAN
        } else {
            100.0 * a as f64 / b as f64
        }
    };
    println!(
        "queries: {} with a known label, {} with a label absent from the library",
        n_known, n_unk
    );
    println!("top-1 (accepted and correct): {:.1}%", pc(top1, n_known));
    println!(
        "top-{} (correct label among candidates): {:.1}%",
        rec.cfg.top,
        pc(top5, n_known)
    );
    println!("wrong label accepted: {:.1}%", pc(wrong, n_known));
    if n_unk > 0 {
        println!(
            "false accepts on unknown sounds: {:.1}%",
            pc(false_acc, n_unk)
        );
    }
    println!(
        "time per query p50 {:.1} ms, p95 {:.1} ms",
        pct(&mut times.clone(), 0.5),
        pct(&mut times, 0.95)
    );
    if flag(a, "--per-label") {
        for (l, (ok, n)) in &per {
            println!("  {l:<30} {:.0}% ({ok}/{n})", pc(*ok, *n));
        }
    }
    let mut conf: Vec<_> = confusion.into_iter().collect();
    conf.sort_by(|a, b| b.1.cmp(&a.1));
    if !conf.is_empty() {
        let v: Vec<String> = conf
            .iter()
            .take(5)
            .map(|((e, p), n)| format!("{e}→{p} ({n})"))
            .collect();
        println!("most frequent confusions: {}", v.join(", "));
    }
    if let Some(c) = arg(a, "--csv") {
        std::fs::write(c, csv).unwrap();
    }
}

// ------------------------------------------------------------------ listen

/// Live recognition from raw PCM on stdin (s16le mono).
/// Exact library → real-time lock + position + lyrics; semantic library →
/// a line every time the detected sound changes.
pub fn cmd_listen(a: &[String]) {
    let lib = Arc::new(open_lib(&a[0]));
    let rate: u32 = arg_parse(a, "--rate", 16000);
    let every: f64 = arg_parse(a, "--every", 1.0);
    let mut stream = if lib.has_exact() && !flag(a, "--no-exact") {
        let mut cfg = EngineConfig::default();
        cfg.duty_cycle_mic = flag(a, "--battery");
        Some(Engine::new(lib.clone(), cfg))
    } else {
        None
    };
    let rec = if lib.has_semantic() && !flag(a, "--no-semantic") {
        let mut c = RecognizerConfig::default();
        c.engines = vec!["semantic".into(), "tags".into()];
        Some(Recognizer::new(lib.clone(), c).unwrap_or_else(|e| die(e)))
    } else {
        None
    };
    let win = rec
        .as_ref()
        .and_then(|r| r.lib.semantic().map(|s| s.config().window_s))
        .unwrap_or(3.0) as f64;
    let mut ring: Vec<f32> = Vec::new();
    let t0 = Instant::now();
    let mut stdin = std::io::stdin().lock();
    let mut buf = vec![0u8; (rate as usize / 10) * 2];
    let mut last_line = (-2i64, -1i64);
    let mut last_sem = String::new();
    let mut next_sem = 0.0;
    let mut out = std::io::stdout();
    eprintln!("listening… (Ctrl-C to quit)");
    loop {
        if stdin.read_exact(&mut buf).is_err() {
            break;
        }
        let now = t0.elapsed().as_secs_f64() * 1000.0;
        let pcm: Vec<f32> = buf
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
            .collect();
        if let Some(eng) = stream.as_mut() {
            if eng.state().wants_audio {
                eng.push_pcm(&pcm, rate, now);
            }
            let st = eng.tick(now);
            if let Some(s) = st.song {
                let li = lib
                    .lyrics(s)
                    .and_then(|l| l.line_at(st.position_ms as u32))
                    .map(|i| i as i64)
                    .unwrap_or(-1);
                if (s as i64, li) != last_line {
                    if last_line.0 != s as i64 {
                        writeln!(
                            out,
                            "\n♪ {}  [{}]  @{:.1}s",
                            lib.refs[s as usize].name,
                            lib.refs[s as usize].label,
                            st.position_ms / 1000.0
                        )
                        .ok();
                    }
                    if let (Some(l), true) = (lib.lyrics(s), li >= 0) {
                        writeln!(out, "  {}", l.lines[li as usize].1).ok();
                    }
                    last_line = (s as i64, li);
                }
                continue; // an exact lock is the most specific answer
            } else if st.phase == Phase::Listening {
                last_line = (-1, -1);
            }
        }
        if let Some(r) = &rec {
            ring.extend_from_slice(&pcm);
            let cap = (win * rate as f64) as usize;
            if ring.len() > cap {
                ring.drain(..ring.len() - cap);
            }
            if now >= next_sem && ring.len() >= cap {
                next_sem = now + every * 1000.0;
                let res = r.recognize(&ring, rate);
                // (key, text): a line is printed only when the detected sound changes
                let (key, txt) = match res.verdict.unwrap() {
                    Verdict::Similar { label, score } => {
                        (format!("s:{label}"), format!("≈ {label} ({score:.2})"))
                    }
                    Verdict::Tag { label, prob } => (
                        format!("t:{label}"),
                        format!("? {label} ({:.0}%)", prob * 100.0),
                    ),
                    Verdict::Exact { ref_id, .. } => (
                        format!("e:{ref_id}"),
                        format!("✓ {}", lib.refs[ref_id as usize].name),
                    ),
                    Verdict::Silence => ("silence".into(), "· silence".into()),
                    Verdict::Unknown => ("unknown".into(), "…".into()),
                };
                if key != last_sem {
                    writeln!(out, "[{:>6.1}s] {txt}", now / 1000.0).ok();
                    last_sem = key;
                }
            }
        }
    }
}
