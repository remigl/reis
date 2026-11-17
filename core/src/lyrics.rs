//! Synced lyrics: LRC parsing (incl. enhanced word tags) and a compact codec.
//!
//! Encoding (per song):
//!   varint n_unique_lines, then each: varint len, utf8 bytes
//!   varint n_events, then each: varint dt_cs (delta from previous event, centiseconds),
//!                                 varint line_idx, varint n_words,
//!                                 n_words * (varint dt_cs_from_line_start, varint byte_end)
//! Repeated lines (choruses) are stored once.

use crate::bytes::*;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub t_ms: u32,
    pub byte_end: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub t_ms: u32,
    pub text: String,
    pub words: Vec<Word>,
}

fn parse_ts(s: &str) -> Option<u32> {
    // mm:ss(.xx|.xxx) or mm:ss:xx
    let (m, rest) = s.split_once(':')?;
    let m: u32 = m.trim().parse().ok()?;
    let (sec, frac) = match rest.split_once(['.', ':']) {
        Some((a, b)) => (a, b),
        None => (rest, "0"),
    };
    let sec: u32 = sec.trim().parse().ok()?;
    let frac = frac.trim();
    let fms = match frac.len() {
        0 => 0,
        1 => frac.parse::<u32>().ok()? * 100,
        2 => frac.parse::<u32>().ok()? * 10,
        _ => frac[..3].parse::<u32>().ok()?,
    };
    Some(m * 60_000 + sec * 1000 + fms)
}

pub fn parse_lrc(src: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut offset_ms: i64 = 0;
    for raw in src.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while rest.starts_with('[') {
            let Some(end) = rest.find(']') else { break };
            let tag = &rest[1..end];
            if let Some(v) = tag.strip_prefix("offset:") {
                offset_ms = v.trim().parse().unwrap_or(0);
            } else if let Some(t) = parse_ts(tag) {
                times.push(t);
            }
            rest = &rest[end + 1..];
        }
        if times.is_empty() {
            continue;
        }
        // enhanced LRC: <mm:ss.xx>word <mm:ss.xx>word
        let mut text = String::new();
        let mut words_abs = Vec::new();
        let mut r = rest;
        let mut pending: Option<u32> = None;
        while !r.is_empty() {
            if r.starts_with('<') {
                if let Some(e) = r.find('>') {
                    if let Some(t) = parse_ts(&r[1..e]) {
                        if let Some(pt) = pending.take() {
                            words_abs.push((pt, text.len() as u32));
                        }
                        pending = Some(t);
                        r = &r[e + 1..];
                        continue;
                    }
                }
            }
            let c = r.chars().next().unwrap();
            text.push(c);
            r = &r[c.len_utf8()..];
        }
        if let Some(pt) = pending {
            words_abs.push((pt, text.len() as u32));
        }
        let text = text.trim_end().to_string();
        for &t in &times {
            let t = (t as i64 - offset_ms).max(0) as u32; // LRC: positive offset = lyrics earlier
            let words = words_abs
                .iter()
                .map(|&(wt, be)| Word {
                    t_ms: (wt as i64 - offset_ms).max(t as i64) as u32,
                    byte_end: be.min(text.len() as u32),
                })
                .collect();
            out.push(Line {
                t_ms: t,
                text: text.clone(),
                words,
            });
        }
    }
    out.sort_by_key(|l| l.t_ms);
    out
}

pub fn encode(lines: &[Line]) -> Vec<u8> {
    let mut uniq: Vec<&str> = Vec::new();
    let mut idx: HashMap<&str, usize> = HashMap::new();
    for l in lines {
        if !idx.contains_key(l.text.as_str()) {
            idx.insert(&l.text, uniq.len());
            uniq.push(&l.text);
        }
    }
    let mut out = Vec::new();
    put_varint(&mut out, uniq.len() as u64);
    for s in &uniq {
        put_varint(&mut out, s.len() as u64);
        out.extend_from_slice(s.as_bytes());
    }
    put_varint(&mut out, lines.len() as u64);
    let mut prev = 0u32;
    for l in lines {
        let cs = l.t_ms / 10;
        put_varint(&mut out, (cs - prev) as u64);
        prev = cs;
        put_varint(&mut out, idx[l.text.as_str()] as u64);
        put_varint(&mut out, l.words.len() as u64);
        for w in &l.words {
            put_varint(&mut out, ((w.t_ms - l.t_ms.min(w.t_ms)) / 10) as u64);
            put_varint(&mut out, w.byte_end as u64);
        }
    }
    out
}

/// Zero-copy decoded view: texts borrow from the catalog mmap.
pub struct LyricsView<'a> {
    pub lines: Vec<(u32, &'a str, Vec<Word>)>,
}

pub fn decode(b: &[u8]) -> LyricsView<'_> {
    let mut p = 0;
    let nu = get_varint(b, &mut p) as usize;
    let mut uniq = Vec::with_capacity(nu);
    for _ in 0..nu {
        let l = get_varint(b, &mut p) as usize;
        uniq.push(std::str::from_utf8(&b[p..p + l]).unwrap_or(""));
        p += l;
    }
    let ne = get_varint(b, &mut p) as usize;
    let mut lines = Vec::with_capacity(ne);
    let mut cs = 0u32;
    for _ in 0..ne {
        cs += get_varint(b, &mut p) as u32;
        let i = get_varint(b, &mut p) as usize;
        let nw = get_varint(b, &mut p) as usize;
        let mut words = Vec::with_capacity(nw);
        for _ in 0..nw {
            let dt = get_varint(b, &mut p) as u32;
            let be = get_varint(b, &mut p) as u32;
            words.push(Word {
                t_ms: cs * 10 + dt * 10,
                byte_end: be,
            });
        }
        lines.push((cs * 10, *uniq.get(i).unwrap_or(&""), words));
    }
    LyricsView { lines }
}

impl<'a> LyricsView<'a> {
    /// Index of the line active at `pos_ms` (last line whose time <= pos), or None before the first.
    pub fn line_at(&self, pos_ms: u32) -> Option<usize> {
        let i = self.lines.partition_point(|l| l.0 <= pos_ms);
        if i == 0 {
            None
        } else {
            Some(i - 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lrc_roundtrip() {
        let src = "[ar:X]\n[00:12.40]ligne 1\n[00:16.20][01:00.00]refrain\n[00:20.10]<00:20.10>mot <00:20.60>deux\n";
        let l = parse_lrc(src);
        assert_eq!(l.len(), 4);
        assert_eq!(l[0].t_ms, 12_400);
        assert_eq!(l[3].text, "refrain");
        assert_eq!(l[2].words.len(), 2);
        let enc = encode(&l);
        let v = decode(&enc);
        assert_eq!(v.lines.len(), 4);
        assert_eq!(v.lines[2].1, "mot deux");
        assert_eq!(v.lines[2].2[1].t_ms, 20_600);
        assert_eq!(v.line_at(17_000), Some(1));
        assert_eq!(v.line_at(1_000), None);
        assert_eq!(v.line_at(99_000), Some(3));
    }
}
