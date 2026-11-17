//! Compact inverted index hash -> [(song, anchor_frame)] and offset-voting matcher.
//!
//! On-disk layout (inside a catalog section, little endian):
//!   u32 bucket_bits B
//!   u32 * (2^B + 1)  byte offsets of buckets in DATA
//!   DATA: per bucket, entries sorted by (hash_low, song, time), encoded as
//!         varint(count) then for each entry
//!           varint(d_low); if d_low == 0 { varint(d_song); if d_song == 0
//!           { varint(d_time) } else { varint(time) } } else { varint(song), varint(time) }
//! Bucket = top B bits of the 24-bit hash. B is chosen so buckets hold ~16
//! entries: a lookup decodes a few dozen bytes, cache-friendly, mmap-able.

use crate::bytes::*;
use crate::fingerprint::HASH_BITS;

pub const MAX_FRAMES: u32 = u16::MAX as u32; // ~17.5 min at 16 ms hop

#[derive(Default)]
pub struct IndexBuilder {
    entries: Vec<u64>, // hash<<40 | song<<16 | t
}

impl IndexBuilder {
    pub fn add(&mut self, song: u32, hashes: &[(u32, u32)]) {
        assert!(song < (1 << 24));
        for &(h, t) in hashes {
            if t < MAX_FRAMES {
                self.entries
                    .push(((h as u64) << 40) | ((song as u64) << 16) | t as u64);
            }
        }
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn serialize(mut self) -> Vec<u8> {
        self.entries.sort_unstable();
        self.entries.dedup();
        let n = self.entries.len().max(1);
        let b = ((n as f64 / 16.0).log2().ceil() as u32).clamp(8, HASH_BITS);
        let low_bits = HASH_BITS - b;
        let nbuckets = 1usize << b;
        let mut dir = vec![0u32; nbuckets + 1];
        let mut data = Vec::with_capacity(self.entries.len() * 5);
        let mut i = 0;
        for bk in 0..nbuckets {
            dir[bk] = data.len() as u32;
            let mut j = i;
            while j < self.entries.len()
                && ((self.entries[j] >> 40) as u32 >> low_bits) as usize == bk
            {
                j += 1;
            }
            if j > i {
                put_varint(&mut data, (j - i) as u64);
                let (mut pl, mut ps, mut pt) = (u64::MAX, 0u64, 0u64);
                for &e in &self.entries[i..j] {
                    let low = (e >> 40) & ((1 << low_bits) - 1);
                    let s = (e >> 16) & 0xFF_FFFF;
                    let t = e & 0xFFFF;
                    if pl == u64::MAX || low != pl {
                        put_varint(&mut data, if pl == u64::MAX { low } else { low - pl });
                        put_varint(&mut data, s);
                        put_varint(&mut data, t);
                    } else {
                        put_varint(&mut data, 0);
                        put_varint(&mut data, s - ps);
                        put_varint(&mut data, if s == ps { t - pt } else { t });
                    }
                    pl = low;
                    ps = s;
                    pt = t;
                }
            }
            i = j;
        }
        dir[nbuckets] = data.len() as u32;
        let mut out = Vec::with_capacity(4 + dir.len() * 4 + data.len());
        out.extend_from_slice(&b.to_le_bytes());
        for d in dir {
            out.extend_from_slice(&d.to_le_bytes());
        }
        out.extend_from_slice(&data);
        out
    }
}

/// Read-only view over a serialized index.
pub struct IndexView<'a> {
    bits: u32,
    dir: &'a [u8],
    data: &'a [u8],
}

impl<'a> IndexView<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        let bits = u32_at(b, 0);
        let dl = ((1usize << bits) + 1) * 4;
        IndexView {
            bits,
            dir: &b[4..4 + dl],
            data: &b[4 + dl..],
        }
    }

    /// Calls `f(song, time)` for every posting of `hash`.
    #[inline]
    pub fn lookup(&self, hash: u32, mut f: impl FnMut(u32, u32)) {
        let low_bits = HASH_BITS - self.bits;
        let bk = (hash >> low_bits) as usize;
        let target = (hash & ((1 << low_bits) - 1)) as u64;
        let start = u32_at(self.dir, bk * 4) as usize;
        let end = u32_at(self.dir, bk * 4 + 4) as usize;
        if start == end {
            return;
        }
        let d = &self.data[..end];
        let mut p = start;
        let cnt = get_varint(d, &mut p);
        let (mut low, mut s, mut t) = (0u64, 0u64, 0u64);
        for k in 0..cnt {
            let dl = get_varint(d, &mut p);
            if k == 0 || dl != 0 {
                low = if k == 0 { dl } else { low + dl };
                s = get_varint(d, &mut p);
                t = get_varint(d, &mut p);
            } else {
                let ds = get_varint(d, &mut p);
                let v = get_varint(d, &mut p);
                if ds == 0 {
                    t += v;
                } else {
                    s += ds;
                    t = v;
                }
            }
            if low > target {
                return;
            }
            if low == target {
                f(s as u32, t as u32);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Candidate {
    pub song: u32,
    pub offset_frames: i32, // reference_time - query_time
    pub score: u32,         // aligned votes (±1 frame)
}

#[derive(Clone, Debug, Default)]
pub struct MatchResult {
    pub top: Vec<Candidate>, // best first, one per song
    pub query_hashes: usize,
}

impl MatchResult {
    pub fn best(&self) -> Option<Candidate> {
        self.top.first().copied()
    }
    pub fn second_score(&self) -> u32 {
        self.top.get(1).map(|c| c.score).unwrap_or(0)
    }
    /// Heuristic confidence in [0,1]: margin of best over runner-up and over
    /// the random-coincidence floor.
    pub fn confidence(&self) -> f32 {
        let Some(b) = self.best() else { return 0.0 };
        let s1 = b.score as f32;
        let s2 = self.second_score() as f32;
        let margin = (s1 - s2 - 2.0).max(0.0);
        let ratio = if s1 > 0.0 {
            1.0 - (s2 + 2.0) / (s1 + 2.0)
        } else {
            0.0
        };
        (1.0 - (-margin / 6.0).exp()) * ratio.clamp(0.0, 1.0)
    }
}

/// Offset-histogram voting.
///
/// Votes are collected as packed u64 `song<<40 | delta<<16 | query_frame`,
/// sorted and run-length counted: cache-friendly (no hash map), and the
/// `dedup` gives "at most one vote per query frame per alignment" for free, so
/// held notes / drones cannot inflate a spurious alignment.
pub fn match_hashes(ix: &IndexView, q: &[(u32, u32)], topn: usize) -> MatchResult {
    let mut qs: Vec<(u32, u32)> = q.to_vec();
    qs.sort_unstable();
    qs.dedup();
    let mut v: Vec<u64> = Vec::with_capacity(qs.len() * 2);
    for &(h, qt) in &qs {
        let qt = qt.min(0xFFFF);
        ix.lookup(h, |s, t| {
            let d = (t as i64 - qt as i64 + (1 << 20)) as u64 & 0xFF_FFFF;
            v.push(((s as u64) << 40) | (d << 16) | qt as u64);
        });
    }
    v.sort_unstable();
    v.dedup();
    // runs of identical (song, delta)
    let mut runs: Vec<(u32, u32, u32)> = Vec::new(); // song, delta, votes
    let mut i = 0;
    while i < v.len() {
        let k = v[i] >> 16;
        let mut j = i + 1;
        while j < v.len() && v[j] >> 16 == k {
            j += 1;
        }
        if j - i >= 1 {
            runs.push(((k >> 24) as u32, (k & 0xFF_FFFF) as u32, (j - i) as u32));
        }
        i = j;
    }
    // best offset per song, merging neighbours (±1 frame)
    let mut top: Vec<Candidate> = Vec::new();
    let mut r = 0;
    while r < runs.len() {
        let s = runs[r].0;
        let mut e = r;
        while e < runs.len() && runs[e].0 == s {
            e += 1;
        }
        let mut best = Candidate {
            song: s,
            offset_frames: 0,
            score: 0,
        };
        for k in r..e {
            if runs[k].2 < 2 {
                continue;
            }
            let d = runs[k].1;
            let mut sc = runs[k].2;
            if k > r && runs[k - 1].1 + 1 == d {
                sc += runs[k - 1].2;
            }
            if k + 1 < e && runs[k + 1].1 == d + 1 {
                sc += runs[k + 1].2;
            }
            if sc > best.score {
                best = Candidate {
                    song: s,
                    offset_frames: d as i32 - (1 << 20),
                    score: sc,
                };
            }
        }
        if best.score > 0 {
            top.push(best);
        }
        r = e;
    }
    top.sort_by(|a, b| b.score.cmp(&a.score).then(a.song.cmp(&b.song)));
    top.truncate(topn);
    MatchResult {
        top,
        query_hashes: qs.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_lookup() {
        let mut b = IndexBuilder::default();
        b.add(3, &[(0xABCDEF, 10), (0xABCDEF, 12), (0x000001, 5)]);
        b.add(7, &[(0xABCDEF, 99)]);
        let bytes = b.serialize();
        let ix = IndexView::new(&bytes);
        let mut got = vec![];
        ix.lookup(0xABCDEF, |s, t| got.push((s, t)));
        assert_eq!(got, vec![(3, 10), (3, 12), (7, 99)]);
        let mut g2 = vec![];
        ix.lookup(0x000001, |s, t| g2.push((s, t)));
        assert_eq!(g2, vec![(3, 5)]);
        let mut g3 = vec![];
        ix.lookup(0x123456, |s, t| g3.push((s, t)));
        assert!(g3.is_empty());
    }
}
