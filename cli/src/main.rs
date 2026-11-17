//! `reis` command line: build catalogs, identify files, benchmark, live listen.

use reis_core::audio::{read_wav, resample, rms_dbfs};
use reis_core::catalog::{Catalog, CatalogBuilder, SongMeta};
use reis_core::engine::{Engine, EngineConfig, Phase};
use reis_core::fingerprint::{Fingerprinter, FpParams};
use reis_core::index::match_hashes;
use reis_core::lyrics;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Instant;

fn usage() -> ! {
    eprintln!(
        "reis — offline music recognition

  reis build  --meta meta.tsv --out catalog.reis [--lyrics-dir D] [--art-dir D]
              [--pps 30] [--fanout 6] [--max-secs 0]
  reis id     catalog.reis query.(wav|any ffmpeg format)
  reis bench  catalog.reis queries.tsv [--csv out.csv]
  reis info   catalog.reis
  reis lyrics catalog.reis SONG_ID
  reis listen catalog.reis [--rate 16000]     (s16le mono PCM on stdin)
  reis simulate catalog.reis song.wav --start S [--secs 60] [--pause-at S --pause-len S] [--speed 1.0]
  reis scale  catalog.reis --copies N         (synthetic scaling of index size/latency)

meta.tsv columns: file, title, artist, album, duration [, lrc, art]  (paths relative to the tsv)"
    );
    std::process::exit(2)
}

fn arg(args: &[String], k: &str) -> Option<String> {
    args.iter()
        .position(|a| a == k)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Decode any audio to mono f32 at `rate` (WAV natively, anything else via ffmpeg).
fn load_audio(path: &Path, rate: u32, max_secs: f64) -> std::io::Result<Vec<f32>> {
    let is_wav = path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("wav"))
        .unwrap_or(false);
    if is_wav {
        if let Ok((x, sr)) = read_wav(path.to_str().unwrap()) {
            let mut y = resample(&x, sr, rate);
            if max_secs > 0.0 {
                y.truncate((max_secs * rate as f64) as usize);
            }
            return Ok(y);
        }
    }
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-i"]).arg(path);
    if max_secs > 0.0 {
        cmd.args(["-t", &max_secs.to_string()]);
    }
    let out = cmd
        .args(["-ac", "1", "-ar", &rate.to_string(), "-f", "f32le", "-"])
        .stderr(Stdio::null())
        .output()?;
    Ok(out
        .stdout
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect())
}

fn mem_report() -> String {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let g = |k: &str| {
        s.lines()
            .find(|l| l.starts_with(k))
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("?")
            .to_string()
    };
    format!(
        "VmHWM {} KB (RssAnon {} KB heap, RssFile {} KB mmap page cache)",
        g("VmHWM"),
        g("RssAnon"),
        g("RssFile")
    )
}

fn peak_rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find(|l| l.starts_with("VmHWM")).map(|l| {
                l.split_whitespace()
                    .nth(1)
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0)
            })
        })
        .unwrap_or(0)
}

fn cmd_build(a: &[String]) {
    let meta = PathBuf::from(arg(a, "--meta").unwrap_or_else(|| usage()));
    let out = arg(a, "--out").unwrap_or_else(|| usage());
    let base = meta.parent().unwrap().to_path_buf();
    let lyr_dir = arg(a, "--lyrics-dir").map(PathBuf::from);
    let art_dir = arg(a, "--art-dir").map(PathBuf::from);
    let max_secs: f64 = arg(a, "--max-secs")
        .map(|v| v.parse().unwrap())
        .unwrap_or(0.0);
    let mut p = FpParams::default();
    if let Some(v) = arg(a, "--pps") {
        p.peaks_per_sec = v.parse().unwrap();
    }
    if let Some(v) = arg(a, "--fanout") {
        p.fanout = v.parse().unwrap();
    }
    if let Some(v) = arg(a, "--whiten") {
        p.whiten_w = v.parse().unwrap();
    }
    if let Some(v) = arg(a, "--rank") {
        p.rank_raw = v == "raw";
    }
    if let Some(v) = arg(a, "--qbits") {
        let (f, t) = v.split_once(',').unwrap();
        p.q_f = f.parse().unwrap();
        p.q_t = t.parse().unwrap();
    }
    if let Some(v) = arg(a, "--lo") {
        p.bin_lo = v.parse().unwrap();
    }
    if let Some(v) = arg(a, "--dtmax") {
        p.dt_max = v.parse().unwrap();
    }
    if let Some(v) = arg(a, "--prom") {
        p.min_prominence_db = v.parse().unwrap();
    }
    let mut b = CatalogBuilder::new(p);
    let src = std::fs::read_to_string(&meta).expect("read meta");
    let mut lines = src.lines();
    let header: Vec<&str> = lines.next().unwrap().split('\t').collect();
    let col = |n: &str| header.iter().position(|h| *h == n);
    let (cf, ct, ca, cal, cd, cl, cart) = (
        col("file").unwrap(),
        col("title"),
        col("artist"),
        col("album"),
        col("duration"),
        col("lrc"),
        col("art"),
    );
    let t0 = Instant::now();
    let (mut n, mut nh, mut audio_s, mut with_lyr) = (0u32, 0usize, 0.0f64, 0u32);
    let rows: Vec<&str> = lines.collect();
    // Fingerprint in parallel, in batches, so memory stays bounded for big libraries.
    let threads = std::thread::available_parallelism()
        .map(|v| v.get())
        .unwrap_or(1);
    let rate = p.sample_rate;
    const BATCH: usize = 64;
    for (bi, batch) in rows.chunks(BATCH).enumerate() {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let paths: Vec<PathBuf> = batch
            .iter()
            .map(|r| base.join(r.split('\t').nth(cf).unwrap()))
            .collect();
        let mut results: Vec<Option<(Vec<(u32, u32)>, f64)>> = vec![None; batch.len()];
        let slots: Vec<std::sync::Mutex<Option<(Vec<(u32, u32)>, f64)>>> = (0..batch.len())
            .map(|_| std::sync::Mutex::new(None))
            .collect();
        std::thread::scope(|s| {
            for _ in 0..threads {
                s.spawn(|| {
                    let fp = Fingerprinter::new(p);
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if i >= paths.len() {
                            break;
                        }
                        let x = load_audio(&paths[i], rate, max_secs).unwrap_or_default();
                        *slots[i].lock().unwrap() =
                            Some((fp.hashes(&x), x.len() as f64 / rate as f64));
                    }
                });
            }
        });
        for (i, sl) in slots.into_iter().enumerate() {
            results[i] = sl.into_inner().unwrap();
        }
        for (k, r) in batch.iter().enumerate() {
            let i = bi * BATCH + k;
            let f: Vec<&str> = r.split('\t').collect();
            let get = |c: Option<usize>| {
                c.and_then(|c| f.get(c))
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            };
            let stem = Path::new(f[cf])
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let (hashes, secs) = results[k].take().unwrap();
            if hashes.is_empty() {
                eprintln!("warning: no audio decoded for {}", f[cf]);
            }
            let lrc_path = match get(cl) {
                s if !s.is_empty() => Some(base.join(s)),
                _ => lyr_dir.as_ref().map(|d| d.join(format!("{stem}.lrc"))),
            };
            let lyr = lrc_path
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|s| lyrics::parse_lrc(&s));
            let art_path = match get(cart) {
                s if !s.is_empty() => Some(base.join(s)),
                _ => art_dir
                    .as_ref()
                    .map(|d| d.join(format!("{}.webp", get(cal)))),
            };
            let art = art_path.and_then(|p| std::fs::read(p).ok());
            let dur: f64 = get(cd).parse().unwrap_or(secs);
            let m = SongMeta {
                title: get(ct),
                artist: get(ca),
                album: get(cal),
                duration_ms: (dur * 1000.0) as u32,
                ext_id: i as u32,
            };
            if lyr.as_ref().map(|l| !l.is_empty()).unwrap_or(false) {
                with_lyr += 1;
            }
            nh += hashes.len();
            audio_s += secs;
            b.add_song(&m, &hashes, lyr.as_deref(), art.as_deref());
            n += 1;
        }
        if bi % 10 == 9 {
            eprintln!("  {} / {} songs", n, rows.len());
        }
    }
    let bytes = b.write();
    std::fs::write(&out, &bytes).expect("write catalog");
    let el = t0.elapsed().as_secs_f64();
    eprintln!(
        "built {out}: {n} songs, {with_lyr} with lyrics, {nh} hashes ({:.1}/s audio), {:.2} MB, {:.1} KB/song, {:.1}s ({:.0}x realtime)",
        nh as f64 / audio_s,
        bytes.len() as f64 / 1e6,
        bytes.len() as f64 / 1e3 / n as f64,
        el,
        audio_s / el
    );
}

fn print_result(cat: &Catalog, r: &reis_core::MatchResult, p: &FpParams) {
    for (k, c) in r.top.iter().enumerate() {
        println!(
            "#{k} song {} \"{}\" — {}  offset {:.2}s  score {}",
            c.song,
            cat.title(c.song),
            cat.artist(c.song),
            c.offset_frames as f64 * p.frame_secs(),
            c.score
        );
    }
    println!(
        "confidence {:.3}  query hashes {}",
        r.confidence(),
        r.query_hashes
    );
}

fn cmd_id(a: &[String]) {
    let cat = Catalog::open(&a[0]).expect("open catalog");
    let p = cat.params;
    let x = load_audio(Path::new(&a[1]), p.sample_rate, 0.0).expect("audio");
    let d = EngineConfig::default();
    let fp = Fingerprinter::new(p.query(d.query_density, d.query_fanout));
    let t0 = Instant::now();
    let r = reis_core::identify(&cat, &fp, &x);
    let el = t0.elapsed();
    print_result(&cat, &r, &p);
    println!(
        "time {:.2} ms for {:.1}s of audio, peak RSS {} KB",
        el.as_secs_f64() * 1e3,
        x.len() as f64 / p.sample_rate as f64,
        peak_rss_kb()
    );
}

fn cmd_info(a: &[String]) {
    let cat = Catalog::open(&a[0]).expect("open");
    println!(
        "songs: {}  file: {:.2} MB",
        cat.song_count(),
        cat.file_size() as f64 / 1e6
    );
    for (k, v) in cat.section_sizes() {
        println!(
            "  {k}: {:.3} MB ({:.2} KB/song)",
            v as f64 / 1e6,
            v as f64 / 1e3 / cat.song_count().max(1) as f64
        );
    }
    println!("params: {:?}", cat.params);
}

fn cmd_lyrics(a: &[String]) {
    let cat = Catalog::open(&a[0]).expect("open");
    let id: u32 = a[1].parse().unwrap();
    println!("{} — {} ({})", cat.artist(id), cat.title(id), cat.album(id));
    if let Some(l) = cat.lyrics(id) {
        for (t, s, _) in &l.lines {
            println!(
                "[{:02}:{:05.2}] {s}",
                t / 60000,
                (t % 60000) as f64 / 1000.0
            );
        }
    }
}

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

fn pct(v: &mut Vec<f64>, q: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * q).round() as usize]
}

fn cmd_bench(a: &[String]) {
    let cat = Catalog::open(&a[0]).expect("open");
    let qf = PathBuf::from(&a[1]);
    let base = qf.parent().unwrap().to_path_buf();
    let csv = arg(a, "--csv");
    let accept_score: u32 = arg(a, "--accept-score")
        .map(|v| v.parse().unwrap())
        .unwrap_or(EngineConfig::default().accept_score);
    let accept_ratio: f32 = arg(a, "--accept-ratio")
        .map(|v| v.parse().unwrap())
        .unwrap_or(EngineConfig::default().accept_ratio);
    let p = cat.params;
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

/// Live: s16le mono PCM on stdin, prints current lyric line.
fn cmd_listen(a: &[String]) {
    let cat = Arc::new(Catalog::open(&a[0]).expect("open"));
    let rate: u32 = arg(a, "--rate")
        .map(|v| v.parse().unwrap())
        .unwrap_or(16000);
    let mut cfg = EngineConfig::default();
    cfg.duty_cycle_mic = a.iter().any(|v| v == "--battery");
    let mut eng = Engine::new(cat.clone(), cfg);
    let t0 = Instant::now();
    let mut stdin = std::io::stdin().lock();
    let mut buf = vec![0u8; (rate as usize / 10) * 2];
    let mut last_line = (-2i64, -1i64);
    let mut out = std::io::stdout();
    eprintln!("listening… (Ctrl-C to quit)");
    loop {
        if stdin.read_exact(&mut buf).is_err() {
            break;
        }
        let now = t0.elapsed().as_secs_f64() * 1000.0;
        let st0 = eng.state();
        if st0.wants_audio {
            let pcm: Vec<f32> = buf
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                .collect();
            eng.push_pcm(&pcm, rate, now);
        }
        let st = eng.tick(now);
        if let Some(s) = st.song {
            let li = cat
                .lyrics(s)
                .and_then(|l| l.line_at(st.position_ms as u32))
                .map(|i| i as i64)
                .unwrap_or(-1);
            if (s as i64, li) != last_line {
                if last_line.0 != s as i64 {
                    writeln!(
                        out,
                        "\n♪ {} — {}  [{}]  @{:.1}s  conf {:.2}",
                        cat.artist(s),
                        cat.title(s),
                        cat.album(s),
                        st.position_ms / 1000.0,
                        st.confidence
                    )
                    .ok();
                }
                if let (Some(l), true) = (cat.lyrics(s), li >= 0) {
                    writeln!(out, "  {}", l.lines[li as usize].1).ok();
                }
                last_line = (s as i64, li);
            }
        } else if st.phase == Phase::Listening && last_line.0 != -1 {
            writeln!(out, "… écoute (niveau {:.0} dBFS)", st.level_dbfs).ok();
            last_line = (-1, -1);
        }
    }
}

/// Simulated playback through the real engine: measures time-to-lock,
/// position error over time, mic duty cycle, pause handling, clock drift.
fn cmd_simulate(a: &[String]) {
    let cat = Arc::new(Catalog::open(&a[0]).expect("open"));
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
fn cmd_scale(a: &[String]) {
    let cat = Catalog::open(&a[0]).expect("open");
    let copies: u32 = arg(a, "--copies").map(|v| v.parse().unwrap()).unwrap_or(10);
    let queries = arg(a, "--queries");
    let p = cat.params;
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
    let mut cb = out.as_ref().map(|_| CatalogBuilder::new(p));
    let n0 = cat.song_count();
    let mut rng = 0x9E3779B97F4A7C15u64;
    for c in 0..copies {
        for (s, hs) in per_song.iter().enumerate() {
            if c == 0 {
                match cb.as_mut() {
                    Some(cb) => {
                        let m = SongMeta {
                            title: cat.title(s as u32).into(),
                            artist: cat.artist(s as u32).into(),
                            album: cat.album(s as u32).into(),
                            duration_ms: cat.duration_ms(s as u32),
                            ext_id: s as u32,
                        };
                        cb.add_song(&m, hs, None, None);
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
                    let m = SongMeta {
                        title: format!("synthetic {c}/{s}"),
                        artist: "scale".into(),
                        album: String::new(),
                        duration_ms: 60_000,
                        ext_id: 0,
                    };
                    cb.add_song(&m, &v, None, None);
                }
                None => b.add(c * n0 + s as u32, &v),
            }
        }
    }
    if let (Some(cb), Some(o)) = (cb, out) {
        let bytes = cb.write();
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
    let _ = rms_dbfs(&[]);
}

/// Peak / hash repeatability of a query against its true reference alignment.
fn cmd_diag(a: &[String]) {
    let cat = Catalog::open(&a[0]).expect("open");
    let p = cat.params;
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

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.is_empty() {
        usage();
    }
    let rest = &a[1..];
    match a[0].as_str() {
        "build" => cmd_build(rest),
        "id" if rest.len() >= 2 => cmd_id(rest),
        "bench" if rest.len() >= 2 => cmd_bench(rest),
        "info" if !rest.is_empty() => cmd_info(rest),
        "lyrics" if rest.len() >= 2 => cmd_lyrics(rest),
        "listen" if !rest.is_empty() => cmd_listen(rest),
        "simulate" if rest.len() >= 2 => cmd_simulate(rest),
        "scale" if !rest.is_empty() => cmd_scale(rest),
        "diag" if rest.len() >= 3 => cmd_diag(rest),
        _ => usage(),
    }
}
