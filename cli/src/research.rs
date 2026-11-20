//! Research commands for the exact (landmark) engine and the neural runtime:
//! benchmark, real-time simulation, scaling, peak diagnostics, model check.

use crate::util::*;
use reis_core::engine::{Engine, EngineConfig, Phase};
use reis_core::fingerprint::Fingerprinter;
use reis_core::index::match_hashes;
use reis_core::library::{LibraryBuilder, LibraryConfig, Prepared, SourceItem};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

#[derive(Default, Clone)]
struct Agg {
    n_pos: u32,
    n_neg: u32,
    top1: u32,
    top5: u32,
    wrong_accept: u32,
    fp: u32,
    off_err: Vec<f64>,
    times: Vec<f64>,
}

pub fn cmd_bench(a: &[String]) {
    let cat = open_lib(&a[0]);
    let qf = PathBuf::from(&a[1]);
    let base = qf.parent().unwrap().to_path_buf();
    let csv = arg(a, "--csv");
    let accept_score: u32 = arg(a, "--accept-score")
        .map(|v| v.parse().unwrap())
        .unwrap_or(EngineConfig::default().accept_score);
    let accept_ratio: f32 = arg(a, "--accept-ratio")
        .map(|v| v.parse().unwrap())
        .unwrap_or(EngineConfig::default().accept_ratio);
    let p = cat
        .params
        .expect("this command needs a library built with the exact engine");
    let d = EngineConfig::default();
    let qd: f32 = arg(a, "--qd")
        .map(|v| v.parse().unwrap())
        .unwrap_or(d.query_density);
    let qfo: f32 = arg(a, "--qf")
        .map(|v| v.parse().unwrap())
        .unwrap_or(d.query_fanout);
    let fp = Fingerprinter::new(p.query(qd, qfo));
    let src = std::fs::read_to_string(&qf).unwrap();
    let mut agg: HashMap<(String, String), Agg> = HashMap::new();
    let mut tot = Agg::default();
    let mut rows_out =
        String::from("file,cond,dur,expected,true_off,got,got_off,score,second,conf,accepted,ms\n");
    let mut lines = src.lines();
    lines.next();
    for l in lines {
        let f: Vec<&str> = l.split('\t').collect();
        let (path, exp, toff, cond, dur) = (
            f[0],
            f[1].parse::<i64>().unwrap(),
            f[2].parse::<f64>().unwrap(),
            f[3],
            f[4],
        );
        let x = load_audio(&base.join(path), p.sample_rate, 0.0).unwrap();
        let t0 = Instant::now();
        let r = reis_core::identify(&cat, &fp, &x);
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        let best = r.best();
        let acc = best
            .map(|c| {
                c.score >= accept_score
                    && c.score as f32 >= accept_ratio * (r.second_score() as f32 + 1.0)
            })
            .unwrap_or(false);
        let e = agg.entry((cond.to_string(), dur.to_string())).or_default();
        for g in [&mut *e, &mut tot] {
            g.times.push(ms);
            if exp >= 0 {
                g.n_pos += 1;
                if r.top.iter().any(|c| c.song as i64 == exp) {
                    g.top5 += 1;
                }
                if acc {
                    let c = best.unwrap();
                    if c.song as i64 == exp {
                        g.top1 += 1;
                        g.off_err
                            .push((c.offset_frames as f64 * p.frame_secs() - toff).abs() * 1000.0);
                    } else {
                        g.wrong_accept += 1;
                    }
                }
            } else {
                g.n_neg += 1;
                if acc {
                    g.fp += 1;
                }
            }
        }
        let (gs, go, sc) = best
            .map(|c| {
                (
                    c.song as i64,
                    c.offset_frames as f64 * p.frame_secs(),
                    c.score,
                )
            })
            .unwrap_or((-1, 0.0, 0));
        rows_out += &format!(
            "{path},{cond},{dur},{exp},{toff:.3},{gs},{go:.3},{sc},{},{:.3},{},{ms:.2}\n",
            r.second_score(),
            r.confidence(),
            acc as u8
        );
    }
    if let Some(c) = csv {
        std::fs::write(c, rows_out).unwrap();
    }
    let mut keys: Vec<_> = agg.keys().cloned().collect();
    keys.sort_by(|a, b| {
        a.0.cmp(&b.0).then(
            a.1.parse::<f64>()
                .unwrap()
                .partial_cmp(&b.1.parse::<f64>().unwrap())
                .unwrap(),
        )
    });
    println!("| condition | dur | top-1 | top-5 | wrong | FP (neg) | offset err p50/p95 ms | time p50 ms |");
    println!("|---|---|---|---|---|---|---|---|");
    let row = |k: &str, d: &str, g: &mut Agg| {
        println!(
            "| {k} | {d} | {:.1}% | {:.1}% | {:.1}% | {:.1}% | {:.0}/{:.0} | {:.2} |",
            100.0 * g.top1 as f64 / g.n_pos.max(1) as f64,
            100.0 * g.top5 as f64 / g.n_pos.max(1) as f64,
            100.0 * g.wrong_accept as f64 / g.n_pos.max(1) as f64,
            100.0 * g.fp as f64 / g.n_neg.max(1) as f64,
            pct(&mut g.off_err, 0.5),
            pct(&mut g.off_err, 0.95),
            pct(&mut g.times, 0.5)
        );
    };
    for k in keys {
        let mut g = agg[&k].clone();
        row(&k.0, &k.1, &mut g);
    }
    row("**ALL**", "", &mut tot);
    println!(
        "\n{}, catalog {:.2} MB, songs {}",
        mem_report(),
        cat.file_size() as f64 / 1e6,
        cat.song_count()
    );
}

/// Simulated playback through the real engine: measures time-to-lock,
/// position error over time, mic duty cycle, pause handling, clock drift.
pub fn cmd_simulate(a: &[String]) {
    let cat = Arc::new(open_lib(&a[0]));
    let song_path = &a[1];
    let start: f64 = arg(a, "--start").map(|v| v.parse().unwrap()).unwrap_or(0.0);
    let secs: f64 = arg(a, "--secs").map(|v| v.parse().unwrap()).unwrap_or(45.0);
    let pause_at: f64 = arg(a, "--pause-at")
        .map(|v| v.parse().unwrap())
        .unwrap_or(-1.0);
    let pause_len: f64 = arg(a, "--pause-len")
        .map(|v| v.parse().unwrap())
        .unwrap_or(0.0);
    let speed: f64 = arg(a, "--speed").map(|v| v.parse().unwrap()).unwrap_or(1.0); // playback clock vs device clock
    let noise_db: f32 = arg(a, "--noise-db")
        .map(|v| v.parse().unwrap())
        .unwrap_or(-200.0);
    let rate = 16000u32;
    let x = load_audio(Path::new(song_path), rate, 0.0).unwrap();
    let mut cfg = EngineConfig::default();
    cfg.duty_cycle_mic = a.iter().any(|v| v == "--battery");
    for kv in std::env::var("REIS_E")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
    {
        let (k, v) = kv.split_once('=').unwrap();
        match k {
            "accept_score" => cfg.accept_score = v.parse().unwrap(),
            "accept_ratio" => cfg.accept_ratio = v.parse().unwrap(),
            "weak_score" => cfg.weak_score = v.parse().unwrap(),
            "weak_ratio" => cfg.weak_ratio = v.parse().unwrap(),
            "attempt_every" => cfg.attempt_every_s = v.parse().unwrap(),
            _ => panic!("unknown {k}"),
        }
    }
    let mut eng = Engine::new(cat.clone(), cfg);
    let chunk = (rate / 50) as usize; // 20 ms callbacks, like a real mic
    let mut song_pos = start; // seconds into the song (ground truth)
    let mut t = 0.0f64; // device clock, s
    let mut rng = 0x1234_5678u64;
    let mut first_lock = None;
    let (mut errs, mut mic_on_chunks, mut total_chunks) = (vec![], 0u64, 0u64);
    let mut was_paused_detected = false;
    let mut cpu_s = 0.0f64;
    let mut log = String::new();
    while t < secs {
        let paused = pause_at >= 0.0 && t >= pause_at && t < pause_at + pause_len;
        let mut pcm = vec![0f32; chunk];
        if !paused {
            for (k, v) in pcm.iter_mut().enumerate() {
                let idx = ((song_pos + k as f64 / rate as f64 * speed) * rate as f64) as usize;
                *v = *x.get(idx).unwrap_or(&0.0);
            }
            song_pos += chunk as f64 / rate as f64 * speed;
        }
        if noise_db > -150.0 {
            let amp = 10f32.powf(noise_db / 20.0) * 1.7;
            for v in pcm.iter_mut() {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                *v += amp * ((rng >> 40) as f32 / (1u64 << 24) as f32 - 0.5);
            }
        }
        t += chunk as f64 / rate as f64;
        let now = t * 1000.0;
        total_chunks += 1;
        let tc = Instant::now();
        if eng.state().wants_audio {
            mic_on_chunks += 1;
            eng.push_pcm(&pcm, rate, now);
        }
        let st = eng.tick(now);
        cpu_s += tc.elapsed().as_secs_f64();
        if st.phase == Phase::Paused {
            was_paused_detected = true;
        }
        if st.song.is_some() && first_lock.is_none() {
            first_lock = Some(t);
            log += &format!(
                "locked at t={t:.2}s song {} pos {:.3}s (truth {:.3}s) score {}\n",
                st.song.unwrap(),
                st.position_ms / 1000.0,
                song_pos,
                st.last_score
            );
        }
        if st.song.is_some() && (total_chunks % 50 == 0) {
            let e = st.position_ms / 1000.0 - song_pos;
            errs.push(e.abs() * 1000.0);
            if total_chunks % 250 == 0 {
                log += &format!(
                    "t={t:5.1}s phase {:?} pos {:7.3} truth {:7.3} err {:+6.0} ms mic {}\n",
                    st.phase,
                    st.position_ms / 1000.0,
                    song_pos,
                    e * 1000.0,
                    st.wants_audio as u8
                );
            }
        }
    }
    print!("{log}");
    let mut e2 = errs.clone();
    println!(
        "time-to-lock {:.2}s | pos err p50 {:.0} ms p95 {:.0} ms max {:.0} ms | mic duty {:.0}% | matches {} corrections {} | pause detected {} | cpu {:.2} ms/s",
        first_lock.unwrap_or(f64::NAN),
        pct(&mut e2, 0.5),
        pct(&mut e2, 0.95),
        errs.iter().cloned().fold(0.0, f64::max),
        100.0 * mic_on_chunks as f64 / total_chunks as f64,
        eng.state().matches,
        eng.state().corrections,
        was_paused_detected,
        cpu_s * 1000.0 / secs
    );
}

/// Scale test: replicate the real catalog's hashes N times with transposed
/// frequencies (distinct songs, realistic hash statistics), then measure index
/// size and lookup latency of real queries.
pub fn cmd_scale(a: &[String]) {
    let cat = open_lib(&a[0]);
    let copies: u32 = arg(a, "--copies").map(|v| v.parse().unwrap()).unwrap_or(10);
    let queries = arg(a, "--queries");
    let p = cat
        .params
        .expect("this command needs a library built with the exact engine");
    let d = EngineConfig::default();
    let fp = Fingerprinter::new(p.query(d.query_density, d.query_fanout));
    // recover per-song hashes by re-reading audio is expensive; instead walk the index
    let ix = cat.index();
    let mut per_song: Vec<Vec<(u32, u32)>> = vec![vec![]; cat.song_count() as usize];
    for h in 0..(1u32 << 24) {
        ix.lookup(h, |s, t| per_song[s as usize].push((h, t)));
    }
    let out = arg(a, "--out");
    let mut b = reis_core::index::IndexBuilder::default();
    let mut cb = out.as_ref().map(|_| {
        let mut cfg = LibraryConfig::default();
        cfg.engines = vec!["exact".into()];
        cfg.exact = p;
        LibraryBuilder::new(cfg, None).unwrap()
    });
    let item = |name: String| SourceItem {
        path: PathBuf::from(&name),
        label: "scale".into(),
        name,
        meta: vec![],
        lyrics: None,
        art: None,
    };
    let n0 = cat.song_count();
    let mut rng = 0x9E3779B97F4A7C15u64;
    for c in 0..copies {
        for (s, hs) in per_song.iter().enumerate() {
            if c == 0 {
                match cb.as_mut() {
                    Some(cb) => {
                        let d = cat.duration_ms(s as u32) as f64 / 1000.0;
                        cb.add(
                            &item(cat.refs[s].name.clone()),
                            Prepared {
                                duration_s: d,
                                exact: hs.clone(),
                                semantic: vec![],
                            },
                        );
                    }
                    None => b.add(s as u32, hs),
                }
                continue;
            }
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let shift = 1 + (rng % 40) as u32; // transpose f1 by 1..40 bins
            let tshift = (rng >> 20) as u32 % 2000;
            let v: Vec<(u32, u32)> = hs
                .iter()
                .map(|&(h, t)| {
                    let f1 = ((h >> 15) + shift) & 0x1FF;
                    ((f1 << 15) | (h & 0x7FFF), t + tshift)
                })
                .collect();
            match cb.as_mut() {
                Some(cb) => {
                    cb.add(
                        &item(format!("synthetic {c}/{s}")),
                        Prepared {
                            duration_s: 60.0,
                            exact: v,
                            semantic: vec![],
                        },
                    );
                }
                None => b.add(c * n0 + s as u32, &v),
            }
        }
    }
    if let (Some(cb), Some(o)) = (cb, out) {
        let bytes = cb.finish();
        std::fs::write(&o, &bytes).unwrap();
        println!(
            "wrote {o}: {} songs, {:.2} MB",
            copies * n0,
            bytes.len() as f64 / 1e6
        );
        return;
    }
    let n = b.len();
    let t0 = Instant::now();
    let bytes = b.serialize();
    let songs = copies * n0;
    println!(
        "songs {songs}  postings {n}  index {:.2} MB  ({:.2} B/posting, {:.1} KB/song)  serialize {:.1}s",
        bytes.len() as f64 / 1e6,
        bytes.len() as f64 / n as f64,
        bytes.len() as f64 / 1e3 / songs as f64,
        t0.elapsed().as_secs_f64()
    );
    if let Some(qf) = queries {
        let big = reis_core::index::IndexView::new(&bytes);
        let qf = PathBuf::from(qf);
        let base = qf.parent().unwrap().to_path_buf();
        let src = std::fs::read_to_string(&qf).unwrap();
        let (mut ok, mut tot, mut times, mut fpos, mut nneg) = (0, 0, vec![], 0, 0);
        let cfg = EngineConfig::default();
        for l in src.lines().skip(1) {
            let f: Vec<&str> = l.split('\t').collect();
            let exp: i64 = f[1].parse().unwrap();
            let x = load_audio(&base.join(f[0]), p.sample_rate, 0.0).unwrap();
            let t0 = Instant::now();
            let h = fp.hashes(&x);
            let r = match_hashes(&big, &h, 5);
            times.push(t0.elapsed().as_secs_f64() * 1e3);
            let acc = r
                .best()
                .map(|c| {
                    c.score >= cfg.accept_score
                        && c.score as f32 >= cfg.accept_ratio * (r.second_score() as f32 + 1.0)
                })
                .unwrap_or(false);
            if exp >= 0 {
                tot += 1;
                if acc && r.best().unwrap().song as i64 == exp {
                    ok += 1;
                }
            } else {
                nneg += 1;
                if acc {
                    fpos += 1;
                }
            }
        }
        println!(
            "queries: top-1 {:.1}% ({ok}/{tot})  FP {fpos}/{nneg}  time p50 {:.2} ms p95 {:.2} ms  peak RSS {} KB",
            100.0 * ok as f64 / tot.max(1) as f64,
            pct(&mut times, 0.5),
            pct(&mut times, 0.95),
            peak_rss_kb()
        );
    }
}

/// Peak / hash repeatability of a query against its true reference alignment.
pub fn cmd_diag(a: &[String]) {
    let cat = open_lib(&a[0]);
    let p = cat
        .params
        .expect("this command needs a library built with the exact engine");
    let dflt = EngineConfig::default();
    let q = load_audio(Path::new(&a[1]), p.sample_rate, 0.0).unwrap();
    let r = load_audio(Path::new(&a[2]), p.sample_rate, 0.0).unwrap();
    let off: f64 = a[3].parse().unwrap();
    let mut p = p;
    let (mut qd, mut qf) = (dflt.query_density, dflt.query_fanout);
    for kv in std::env::var("REIS_P")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
    {
        let (k, v) = kv.split_once('=').unwrap();
        match k {
            "nb_f" => p.nb_f = v.parse().unwrap(),
            "nb_t" => p.nb_t = v.parse().unwrap(),
            "pps" => p.peaks_per_sec = v.parse().unwrap(),
            "fanout" => p.fanout = v.parse().unwrap(),
            "whiten" => p.whiten_w = v.parse().unwrap(),
            "raw" => p.rank_raw = v == "1",
            "prom" => p.min_prominence_db = v.parse().unwrap(),
            "lo" => p.bin_lo = v.parse().unwrap(),
            "hi" => p.bin_hi = v.parse().unwrap(),
            "qd" => qd = v.parse().unwrap(),
            "q_f" => p.q_f = v.parse().unwrap(),
            "q_t" => p.q_t = v.parse().unwrap(),
            "dtmax" => p.dt_max = v.parse().unwrap(),
            "qf" => qf = v.parse().unwrap(),
            _ => panic!("unknown {k}"),
        }
    }
    let fr = Fingerprinter::new(p);
    let fq = Fingerprinter::new(p.query(qd, qf));
    let pr = fr.peaks(&fr.spectrogram(&r));
    let pq = fq.peaks(&fq.spectrogram(&q));
    let dfr = (off / p.frame_secs()).round() as i64;
    let set: std::collections::HashSet<(i64, i64)> =
        pr.iter().map(|k| (k.t as i64, k.f as i64)).collect();
    let qn = q.len() as f64 / p.sample_rate as f64;
    let refs_in: Vec<_> = pr
        .iter()
        .filter(|k| {
            (k.t as i64) >= dfr + 10 && (k.t as i64) < dfr + (qn / p.frame_secs()) as i64 - 10
        })
        .collect();
    let mut hit = 0;
    for k in &refs_in {
        let qt = k.t as i64 - dfr;
        if pq
            .iter()
            .any(|x| (x.t as i64 - qt).abs() <= 1 && (x.f as i64 - k.f as i64).abs() <= 1)
        {
            hit += 1;
        }
    }
    let _ = set;
    let hr: std::collections::HashSet<(u32, i64)> = fr
        .hashes_from_peaks(&pr)
        .into_iter()
        .map(|(h, t)| (h, t as i64))
        .collect();
    let hq = fq.hashes_from_peaks(&pq);
    let hh = hq
        .iter()
        .filter(|&&(h, t)| hr.contains(&(h, t as i64 + dfr)))
        .count();
    let nref_h = hr
        .iter()
        .filter(|&&(_, t)| t >= dfr + 10 && t < dfr + (qn / p.frame_secs()) as i64 - 110)
        .count();
    println!(
        "{:.3} {:.3} {} {}",
        hit as f64 / refs_in.len().max(1) as f64,
        hh as f64 / nref_h.max(1) as f64,
        hq.len(),
        hh
    );
}

/// Compare the Rust runtime against the exporter's PyTorch reference outputs.
pub fn cmd_model_check(a: &[String]) {
    let m = reis_core::nn::Model::load(&a[0]).unwrap();
    let b = std::fs::read(&a[1]).unwrap();
    let mut p = 0;
    let mut arr = || {
        let n = u32::from_le_bytes(b[p..p + 4].try_into().unwrap()) as usize;
        p += 4;
        let v: Vec<f32> = (0..n)
            .map(|i| f32::from_le_bytes(b[p + 4 * i..p + 4 * i + 4].try_into().unwrap()))
            .collect();
        p += 4 * n;
        v
    };
    let (x, mel_ref, emb_ref, logit_ref) = (arr(), arr(), arr(), arr());
    let t0 = Instant::now();
    let (mel, frames) = m.logmel(&x);
    let t_mel = t0.elapsed();
    let mel_err = mel
        .iter()
        .zip(&mel_ref)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    let t1 = Instant::now();
    let out = m.forward_mel(&mel_ref, frames);
    let t_net = t1.elapsed();
    let emb = &out["embedding"];
    let cos = |a: &[f32], b: &[f32]| {
        let d: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        d / (a.iter().map(|x| x * x).sum::<f32>().sqrt()
            * b.iter().map(|x| x * x).sum::<f32>().sqrt())
    };
    let logits_rust: Vec<f32> = out["logits"].iter().map(|p| (p / (1.0 - p)).ln()).collect();
    let lerr = logits_rust
        .iter()
        .zip(&logit_ref)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    let top = |v: &[f32]| {
        let mut i: Vec<usize> = (0..v.len()).collect();
        i.sort_by(|a, b| v[*b].partial_cmp(&v[*a]).unwrap());
        i.truncate(5);
        i
    };
    println!(
        "model {}  params {}  frames {frames}",
        m.name,
        m.param_count()
    );
    println!(
        "front-end  max |rust - torch| = {mel_err:.2e}   ({:.1} ms)",
        t_mel.as_secs_f64() * 1e3
    );
    println!(
        "embedding  cosine(rust, torch) = {:.6}  dim {}",
        cos(emb, &emb_ref),
        emb.len()
    );
    println!(
        "logits     max |rust - torch| = {lerr:.3e}   network {:.1} ms for {:.1} s of audio",
        t_net.as_secs_f64() * 1e3,
        x.len() as f64 / m.frontend.sample_rate as f64
    );
    println!(
        "top-5 rust : {:?}",
        top(&logits_rust)
            .iter()
            .map(|&i| m.labels[i].as_str())
            .collect::<Vec<_>>()
    );
    println!(
        "top-5 torch: {:?}",
        top(&logit_ref)
            .iter()
            .map(|&i| m.labels[i].as_str())
            .collect::<Vec<_>>()
    );
}
