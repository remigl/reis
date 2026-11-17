//! End-to-end: build a catalog from synthetic "songs", then identify excerpts
//! one-shot and through the streaming engine (lock, position, lyrics).

use reis_core::catalog::{Catalog, CatalogBuilder, SongMeta};
use reis_core::engine::{Engine, EngineConfig, Phase};
use reis_core::fingerprint::{Fingerprinter, FpParams};
use reis_core::lyrics::parse_lrc;
use std::sync::Arc;

const SR: u32 = 8000;

fn song(seed: u64, secs: f32) -> Vec<f32> {
    let mut s = seed;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    let n = (secs * SR as f32) as usize;
    let mut x = vec![0f32; n];
    let note = (0.18 * SR as f32) as usize;
    let mut i = 0;
    while i < n {
        let f: Vec<f64> = (0..3).map(|_| 200.0 + rnd() * 2800.0).collect();
        for k in i..(i + note).min(n) {
            let t = k as f64 / SR as f64;
            let env = (-((k - i) as f64) / note as f64 * 3.0).exp();
            x[k] = (f
                .iter()
                .map(|ff| (2.0 * std::f64::consts::PI * ff * t).sin())
                .sum::<f64>()
                * env
                * 0.25) as f32;
        }
        i += note;
    }
    x
}

fn build(name: &str) -> (Arc<Catalog>, Vec<Vec<f32>>) {
    let p = FpParams::default();
    let fp = Fingerprinter::new(p);
    let mut b = CatalogBuilder::new(p);
    let songs: Vec<Vec<f32>> = (0..5).map(|i| song(1000 + i, 40.0)).collect();
    for (i, s) in songs.iter().enumerate() {
        let lrc = parse_lrc("[00:05.00]un\n[00:10.00]deux\n[00:20.00]trois\n");
        let m = SongMeta {
            title: format!("T{i}"),
            artist: "A".into(),
            album: "B".into(),
            duration_ms: 40_000,
            ext_id: i as u32,
        };
        b.add_song(&m, &fp.hashes(s), Some(&lrc), Some(b"fakeimg"));
    }
    // one file per test: tests run in parallel
    let path = std::env::temp_dir().join(format!("reis_test_{}_{name}.reis", std::process::id()));
    std::fs::write(&path, b.write()).unwrap();
    let cat = Arc::new(Catalog::open(path.to_str().unwrap()).unwrap());
    (cat, songs)
}

#[test]
fn one_shot_offset() {
    let (cat, songs) = build("one_shot");
    let d = EngineConfig::default();
    let fp = Fingerprinter::new(cat.params.query(d.query_density, d.query_fanout));
    let start = 13.37;
    let s0 = (start * SR as f64) as usize;
    let q = &songs[3][s0..s0 + 4 * SR as usize];
    let r = reis_core::identify(&cat, &fp, q);
    let c = r.best().unwrap();
    assert_eq!(c.song, 3);
    let off = c.offset_frames as f64 * cat.params.frame_secs();
    assert!((off - start).abs() < 0.03, "offset {off}");
    assert_eq!(cat.title(3), "T3");
    assert_eq!(cat.artwork(3).unwrap(), b"fakeimg");
    assert_eq!(cat.lyrics(3).unwrap().line_at(12_000), Some(1));
}

#[test]
fn engine_locks_and_tracks() {
    let (cat, songs) = build("engine");
    let mut e = Engine::new(cat.clone(), EngineConfig::default());
    let start = 6.0f64;
    let chunk = 160usize; // 20 ms
    let mut pos = (start * SR as f64) as usize;
    let mut now = 0.0;
    let mut locked_at = None;
    while now < 20_000.0 {
        let c = &songs[1][pos..pos + chunk];
        pos += chunk;
        now += 20.0;
        if e.state().wants_audio {
            e.push_pcm(c, SR, now);
        }
        let st = e.tick(now);
        if st.phase == Phase::Locked && locked_at.is_none() {
            locked_at = Some(now);
        }
    }
    let st = e.state();
    assert_eq!(st.song, Some(1));
    assert!(locked_at.unwrap() < 4000.0, "lock took {:?}", locked_at);
    let truth = start * 1000.0 + now;
    assert!(
        (st.position_ms - truth).abs() < 50.0,
        "pos {} truth {}",
        st.position_ms,
        truth
    );
}
