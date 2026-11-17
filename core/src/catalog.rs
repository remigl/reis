//! `.reis` catalog bundle: one memory-mapped file holding everything the app
//! needs offline (fingerprint index, metadata, lyrics, artwork).
//!
//! Header: b"REIS" | u32 version | u32 n_sections | n * (tag[4], u64 off, u64 len)
//! Sections:
//!   PARM  fingerprint parameters (so an index is never queried with other params)
//!   HIDX  landmark index (see index.rs)
//!   SONG  fixed 32-byte records: title, artist, album (STRS offsets), duration_ms,
//!         lyrics offset (LYRC, u32::MAX = none), artwork id (u32::MAX = none), flags, ext
//!   STRS  varint-length-prefixed UTF-8 strings (deduplicated)
//!   LYRC  per-song lyrics blobs (see lyrics.rs)
//!   ARTW  u32 count, u32 offsets[count+1], image bytes (JPEG/WebP), deduplicated

use crate::bytes::*;
use crate::fingerprint::FpParams;
use crate::index::{IndexBuilder, IndexView};
use crate::lyrics::{self, Line, LyricsView};
use std::collections::HashMap;

pub const VERSION: u32 = 1;
const NONE: u32 = u32::MAX;
const REC: usize = 32;

#[derive(Clone, Debug, Default)]
pub struct SongMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u32,
    pub ext_id: u32, // free slot (e.g. row id in the source DB)
}

pub struct CatalogBuilder {
    pub params: FpParams,
    pub index: IndexBuilder,
    songs: Vec<[u32; 8]>,
    strs: Vec<u8>,
    str_ix: HashMap<String, u32>,
    lyrc: Vec<u8>,
    art: Vec<Vec<u8>>,
    art_ix: HashMap<u64, u32>,
}

fn fnv1a(b: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &c in b {
        h ^= c as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

impl CatalogBuilder {
    pub fn new(params: FpParams) -> Self {
        CatalogBuilder {
            params,
            index: IndexBuilder::default(),
            songs: vec![],
            strs: vec![],
            str_ix: HashMap::new(),
            lyrc: vec![],
            art: vec![],
            art_ix: HashMap::new(),
        }
    }
    fn s(&mut self, s: &str) -> u32 {
        if let Some(&o) = self.str_ix.get(s) {
            return o;
        }
        let o = self.strs.len() as u32;
        put_varint(&mut self.strs, s.len() as u64);
        self.strs.extend_from_slice(s.as_bytes());
        self.str_ix.insert(s.to_string(), o);
        o
    }
    /// Returns the song id.
    pub fn add_song(
        &mut self,
        m: &SongMeta,
        hashes: &[(u32, u32)],
        lyr: Option<&[Line]>,
        artwork: Option<&[u8]>,
    ) -> u32 {
        let id = self.songs.len() as u32;
        let (t, a, al) = (self.s(&m.title), self.s(&m.artist), self.s(&m.album));
        let lo = match lyr {
            Some(l) if !l.is_empty() => {
                let o = self.lyrc.len() as u32;
                self.lyrc.extend(lyrics::encode(l));
                o
            }
            _ => NONE,
        };
        let ao = match artwork {
            Some(img) if !img.is_empty() => {
                let h = fnv1a(img);
                *self.art_ix.entry(h).or_insert_with(|| {
                    self.art.push(img.to_vec());
                    (self.art.len() - 1) as u32
                })
            }
            _ => NONE,
        };
        self.songs
            .push([t, a, al, m.duration_ms, lo, ao, 0, m.ext_id]);
        self.index.add(id, hashes);
        id
    }

    pub fn write(self) -> Vec<u8> {
        let p = self.params;
        let mut parm = Vec::new();
        for v in [
            p.sample_rate,
            p.n_fft as u32,
            p.hop as u32,
            p.bin_lo as u32,
            p.bin_hi as u32,
            p.nb_f as u32,
            p.nb_t as u32,
            p.peaks_per_sec.to_bits(),
            p.fanout as u32,
            p.dt_min,
            p.dt_max,
            p.df_max as u32,
            p.min_prominence_db.to_bits(),
            p.whiten_w as u32,
            p.rank_raw as u32,
            p.q_f,
            p.q_t,
        ] {
            parm.extend_from_slice(&v.to_le_bytes());
        }
        let mut song = Vec::with_capacity(self.songs.len() * REC);
        for r in &self.songs {
            for v in r {
                song.extend_from_slice(&v.to_le_bytes());
            }
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
        let hidx = self.index.serialize();
        let secs: Vec<(&[u8; 4], &[u8])> = vec![
            (b"PARM", &parm),
            (b"HIDX", &hidx),
            (b"SONG", &song),
            (b"STRS", &self.strs),
            (b"LYRC", &self.lyrc),
            (b"ARTW", &artw),
        ];
        let head = 12 + secs.len() * 20;
        let mut out = Vec::new();
        out.extend_from_slice(b"REIS");
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(secs.len() as u32).to_le_bytes());
        let mut o = head as u64;
        let mut pads = vec![];
        for (tag, d) in &secs {
            let pad = (8 - (o % 8)) % 8; // 8-byte align sections
            o += pad;
            pads.push(pad);
            out.extend_from_slice(*tag);
            out.extend_from_slice(&o.to_le_bytes());
            out.extend_from_slice(&(d.len() as u64).to_le_bytes());
            o += d.len() as u64;
        }
        for ((_, d), pad) in secs.iter().zip(pads) {
            out.extend(std::iter::repeat(0u8).take(pad as usize));
            out.extend_from_slice(d);
        }
        out
    }
}

pub struct Catalog {
    blob: Blob,
    secs: HashMap<[u8; 4], (usize, usize)>,
    pub params: FpParams,
}

impl Catalog {
    pub fn open(path: &str) -> std::io::Result<Catalog> {
        let blob = Blob::open(path)?;
        let b = blob.bytes();
        let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_string());
        if b.len() < 12 || &b[0..4] != b"REIS" {
            return Err(bad("not a reis catalog"));
        }
        if u32_at(b, 4) != VERSION {
            return Err(bad("unsupported catalog version"));
        }
        let n = u32_at(b, 8) as usize;
        let mut secs = HashMap::new();
        for i in 0..n {
            let e = 12 + i * 20;
            let tag: [u8; 4] = b[e..e + 4].try_into().unwrap();
            let (o, l) = (u64_at(b, e + 4) as usize, u64_at(b, e + 12) as usize);
            if o + l > b.len() {
                return Err(bad("truncated catalog"));
            }
            secs.insert(tag, (o, l));
        }
        for t in [b"PARM", b"HIDX", b"SONG", b"STRS", b"LYRC", b"ARTW"] {
            if !secs.contains_key(t) {
                return Err(bad("missing section"));
            }
        }
        let (po, _) = secs[b"PARM"];
        let g = |i: usize| u32_at(b, po + 4 * i);
        let params = FpParams {
            sample_rate: g(0),
            n_fft: g(1) as usize,
            hop: g(2) as usize,
            bin_lo: g(3) as usize,
            bin_hi: g(4) as usize,
            nb_f: g(5) as usize,
            nb_t: g(6) as usize,
            peaks_per_sec: f32::from_bits(g(7)),
            fanout: g(8) as usize,
            dt_min: g(9),
            dt_max: g(10),
            df_max: g(11) as i32,
            min_prominence_db: f32::from_bits(g(12)),
            whiten_w: g(13) as usize,
            rank_raw: g(14) != 0,
            q_f: g(15),
            q_t: g(16),
        };
        Ok(Catalog { blob, secs, params })
    }

    pub fn section(&self, tag: &[u8; 4]) -> &[u8] {
        let (o, l) = self.secs[tag];
        &self.blob.bytes()[o..o + l]
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
    pub fn index(&self) -> IndexView<'_> {
        IndexView::new(self.section(b"HIDX"))
    }
    pub fn song_count(&self) -> u32 {
        (self.section(b"SONG").len() / REC) as u32
    }
    fn rec(&self, id: u32, field: usize) -> u32 {
        u32_at(self.section(b"SONG"), id as usize * REC + field * 4)
    }
    fn string(&self, off: u32) -> &str {
        let s = self.section(b"STRS");
        let mut p = off as usize;
        let l = get_varint(s, &mut p) as usize;
        std::str::from_utf8(&s[p..p + l]).unwrap_or("")
    }
    pub fn title(&self, id: u32) -> &str {
        self.string(self.rec(id, 0))
    }
    pub fn artist(&self, id: u32) -> &str {
        self.string(self.rec(id, 1))
    }
    pub fn album(&self, id: u32) -> &str {
        self.string(self.rec(id, 2))
    }
    pub fn duration_ms(&self, id: u32) -> u32 {
        self.rec(id, 3)
    }
    pub fn ext_id(&self, id: u32) -> u32 {
        self.rec(id, 7)
    }
    pub fn lyrics(&self, id: u32) -> Option<LyricsView<'_>> {
        let o = self.rec(id, 4);
        if o == NONE {
            return None;
        }
        Some(lyrics::decode(&self.section(b"LYRC")[o as usize..]))
    }
    pub fn artwork(&self, id: u32) -> Option<&[u8]> {
        let a = self.rec(id, 5);
        if a == NONE {
            return None;
        }
        let s = self.section(b"ARTW");
        let n = u32_at(s, 0);
        if a >= n {
            return None;
        }
        let base = 4 + (n as usize + 1) * 4;
        let (o, e) = (
            u32_at(s, 4 + a as usize * 4) as usize,
            u32_at(s, 8 + a as usize * 4) as usize,
        );
        Some(&s[base + o..base + e])
    }
}
