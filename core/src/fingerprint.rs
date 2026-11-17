//! Landmark ("constellation") fingerprinting, Shazam-style (Wang, 2003).
//!
//! 1. 8 kHz mono, STFT (1024-point Hann, hop 128 = 16 ms).
//! 2. log power, spectrally whitened per frame (removes speaker / mic EQ tilt
//!    and makes the representation gain-invariant).
//! 3. Peaks = local maxima of a rectangular time-frequency neighbourhood,
//!    density-limited to the N most prominent per second.
//! 4. Each anchor peak is paired with the next `fanout` peaks in a target zone;
//!    hash = f1 (9 bits) | f2-f1 (8 bits) | dt (7 bits) -> 24-bit key.
//!    The anchor time is stored with the key, so a match also yields the
//!    *offset* of the query in the reference -> position in the song.

use crate::fft::Fft;

pub const HASH_BITS: u32 = 24;
const FLOOR_DB: f32 = 60.0;

#[derive(Clone, Copy, Debug)]
pub struct FpParams {
    pub sample_rate: u32,
    pub n_fft: usize,
    pub hop: usize,
    pub bin_lo: usize,
    pub bin_hi: usize,
    pub nb_f: usize, // half-width of peak neighbourhood in bins
    pub nb_t: usize, // half-width of peak neighbourhood in frames
    pub peaks_per_sec: f32,
    pub fanout: usize,
    pub dt_min: u32,
    pub dt_max: u32,
    pub df_max: i32,
    pub min_prominence_db: f32,
    pub whiten_w: usize,
    /// rank peaks by raw (gain-invariant within a slice) energy instead of whitened prominence
    pub rank_raw: bool,
    /// hash quantisation: frequency fields >> q_f, dt >> q_t (robustness vs specificity)
    pub q_f: u32,
    pub q_t: u32,
}

impl Default for FpParams {
    fn default() -> Self {
        FpParams {
            sample_rate: 8000,
            n_fft: 1024,
            hop: 128,
            bin_lo: 32,  // 250 Hz: phone/laptop speakers reproduce little below
            bin_hi: 480, // 3.75 kHz
            nb_f: 12,
            nb_t: 8,
            peaks_per_sec: 30.0,
            fanout: 3,
            dt_min: 2,
            dt_max: 100,
            df_max: 127,
            min_prominence_db: 3.0,
            whiten_w: 24,
            rank_raw: true,
            q_f: 1,
            q_t: 1,
        }
    }
}

impl FpParams {
    /// Query-side parameters: denser peaks than the reference so that the
    /// reference's (sparser) peaks survive noise-induced rank changes.
    pub fn query(&self, density_mult: f32, fanout_mult: f32) -> FpParams {
        let mut q = *self;
        q.peaks_per_sec *= density_mult;
        q.fanout = ((q.fanout as f32) * fanout_mult).round() as usize;
        q
    }
    pub fn frame_secs(&self) -> f64 {
        self.hop as f64 / self.sample_rate as f64
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Peak {
    pub t: u32,
    pub f: u16,
    pub v: f32,
}

/// Whitened log-power spectrogram, row-major [frame][bin - bin_lo].
pub struct Spectrogram {
    pub frames: usize,
    pub bins: usize,
    pub data: Vec<f32>, // whitened
    pub raw: Vec<f32>,  // log power
}

pub struct Fingerprinter {
    pub p: FpParams,
    fft: Fft,
    win: Vec<f32>,
}

impl Fingerprinter {
    pub fn new(p: FpParams) -> Self {
        let n = p.n_fft;
        let win = (0..n)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos())
            .collect();
        Fingerprinter {
            fft: Fft::new(n),
            win,
            p,
        }
    }

    pub fn spectrogram(&self, x: &[f32]) -> Spectrogram {
        let p = &self.p;
        let n = p.n_fft;
        let bins = p.bin_hi - p.bin_lo;
        if x.len() < n {
            return Spectrogram {
                frames: 0,
                bins,
                data: vec![],
                raw: vec![],
            };
        }
        let frames = (x.len() - n) / p.hop + 1;
        let mut data = vec![0f32; frames * bins];
        let mut raw = vec![0f32; frames * bins];
        let (mut re, mut im) = (Vec::with_capacity(n), Vec::with_capacity(n));
        let mut frame = vec![0f32; n];
        let mut pw = vec![0f32; p.bin_hi];
        let mut lg = vec![0f32; bins];
        let w = p.whiten_w;
        let mut csum = vec![0f32; bins + 1];
        for t in 0..frames {
            let s = &x[t * p.hop..t * p.hop + n];
            for i in 0..n {
                frame[i] = s[i] * self.win[i];
            }
            self.fft.power(&frame, &mut re, &mut im, &mut pw);
            let mut mx = -300.0f32;
            for b in 0..bins {
                lg[b] = 10.0 * (pw[b + p.bin_lo] + 1e-20).log10();
                mx = mx.max(lg[b]);
            }
            // relative floor: gain-invariant, ignores ultra-quiet bins whose
            // values only reflect the recording's noise floor
            for v in lg.iter_mut() {
                *v = v.max(mx - FLOOR_DB);
            }
            for b in 0..bins {
                csum[b + 1] = csum[b] + lg[b];
            }
            raw[t * bins..(t + 1) * bins].copy_from_slice(&lg);
            let row = &mut data[t * bins..(t + 1) * bins];
            for b in 0..bins {
                let lo = b.saturating_sub(w);
                let hi = (b + w + 1).min(bins);
                let mean = (csum[hi] - csum[lo]) / (hi - lo) as f32;
                row[b] = lg[b] - mean;
            }
        }
        Spectrogram {
            frames,
            bins,
            data,
            raw,
        }
    }

    pub fn peaks(&self, s: &Spectrogram) -> Vec<Peak> {
        let p = &self.p;
        let (tf, nb) = (s.frames, s.bins);
        if tf == 0 {
            return vec![];
        }
        // separable max filter: frequency then time
        let mut mf = vec![0f32; tf * nb];
        for t in 0..tf {
            let row = &s.raw[t * nb..(t + 1) * nb];
            let out = &mut mf[t * nb..(t + 1) * nb];
            sliding_max(row, p.nb_f, out);
        }
        let mut mt = vec![0f32; tf * nb];
        let mut col = vec![0f32; tf];
        let mut colo = vec![0f32; tf];
        for b in 0..nb {
            for t in 0..tf {
                col[t] = mf[t * nb + b];
            }
            sliding_max(&col, p.nb_t, &mut colo);
            for t in 0..tf {
                mt[t * nb + b] = colo[t];
            }
        }
        let mut cand = Vec::new();
        for t in 0..tf {
            for b in 0..nb {
                let v = s.data[t * nb + b];
                if v >= p.min_prominence_db && s.raw[t * nb + b] == mt[t * nb + b] {
                    cand.push(Peak {
                        t: t as u32,
                        f: (b + p.bin_lo) as u16,
                        v: if p.rank_raw { s.raw[t * nb + b] } else { v },
                    });
                }
            }
        }
        // density control: keep top-K per 1 s slice
        let slice = ((1.0 / p.frame_secs()).round() as u32).max(1);
        let k = p.peaks_per_sec.round() as usize;
        let mut out = Vec::with_capacity(cand.len().min(tf / slice as usize * k + k));
        let mut i = 0;
        while i < cand.len() {
            let s0 = cand[i].t / slice;
            let mut j = i;
            while j < cand.len() && cand[j].t / slice == s0 {
                j += 1;
            }
            let grp = &mut cand[i..j];
            grp.sort_by(|a, b| b.v.partial_cmp(&a.v).unwrap());
            let mut keep: Vec<Peak> = grp.iter().take(k).copied().collect();
            keep.sort_by_key(|p| (p.t, p.f));
            out.extend(keep);
            i = j;
        }
        out
    }

    /// (hash, anchor_frame) pairs.
    pub fn hashes_from_peaks(&self, peaks: &[Peak]) -> Vec<(u32, u32)> {
        let p = &self.p;
        let mut out = Vec::with_capacity(peaks.len() * p.fanout);
        for i in 0..peaks.len() {
            let a = peaks[i];
            let mut n = 0;
            for b in &peaks[i + 1..] {
                let dt = b.t - a.t;
                if dt > p.dt_max {
                    break;
                }
                if dt < p.dt_min {
                    continue;
                }
                let df = b.f as i32 - a.f as i32;
                if df.abs() > p.df_max {
                    continue;
                }
                out.push((make_hash_q(a.f, df, dt, p.q_f, p.q_t), a.t));
                n += 1;
                if n >= p.fanout {
                    break;
                }
            }
        }
        out
    }

    pub fn hashes(&self, x: &[f32]) -> Vec<(u32, u32)> {
        let s = self.spectrogram(x);
        let pk = self.peaks(&s);
        self.hashes_from_peaks(&pk)
    }
}

#[inline]
pub fn make_hash_q(f1: u16, df: i32, dt: u32, qf: u32, qt: u32) -> u32 {
    if qf == 0 && qt == 0 {
        return make_hash(f1, df, dt);
    }
    let f1q = (f1 as u32) >> qf;
    let dfq = (df + 128) as u32 >> qf;
    let dtq = dt >> qt;
    (f1q << 15) | (dfq << 7) | (dtq & 0x7F)
}

#[inline]
pub fn make_hash(f1: u16, df: i32, dt: u32) -> u32 {
    ((f1 as u32 & 0x1FF) << 15) | (((df + 128) as u32 & 0xFF) << 7) | (dt & 0x7F)
}

/// out[i] = max(x[i-w ..= i+w]) — monotonic deque, O(n).
fn sliding_max(x: &[f32], w: usize, out: &mut [f32]) {
    let n = x.len();
    let mut dq: std::collections::VecDeque<usize> =
        std::collections::VecDeque::with_capacity(2 * w + 2);
    let mut next = 0;
    for i in 0..n {
        let hi = (i + w).min(n - 1);
        while next <= hi {
            while let Some(&b) = dq.back() {
                if x[b] <= x[next] {
                    dq.pop_back();
                } else {
                    break;
                }
            }
            dq.push_back(next);
            next += 1;
        }
        while let Some(&f) = dq.front() {
            if f + w < i {
                dq.pop_front();
            } else {
                break;
            }
        }
        out[i] = x[*dq.front().unwrap()];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sliding_max_ok() {
        let x = [1.0, 3.0, 2.0, 0.0, 5.0, 1.0];
        let mut o = [0.0; 6];
        sliding_max(&x, 1, &mut o);
        assert_eq!(o, [3.0, 3.0, 3.0, 5.0, 5.0, 5.0]);
    }
}
