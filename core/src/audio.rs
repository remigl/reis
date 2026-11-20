//! Audio input: WAV decoding and streaming resampling to the analysis rate.

use std::io::{self, Read};

/// Decode a RIFF/WAVE file (PCM 8/16/24/32-bit int or 32-bit float) to mono f32.
pub fn decode_wav(bytes: &[u8]) -> io::Result<(Vec<f32>, u32)> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("not a WAV file"));
    }
    let mut p = 12;
    let (mut fmt, mut ch, mut sr, mut bits) = (0u16, 0u16, 0u32, 0u16);
    let mut data: Option<&[u8]> = None;
    while p + 8 <= bytes.len() {
        let id = &bytes[p..p + 4];
        let sz = u32::from_le_bytes(bytes[p + 4..p + 8].try_into().unwrap()) as usize;
        let body = &bytes[p + 8..(p + 8 + sz).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            fmt = u16::from_le_bytes([body[0], body[1]]);
            ch = u16::from_le_bytes([body[2], body[3]]);
            sr = u32::from_le_bytes(body[4..8].try_into().unwrap());
            bits = u16::from_le_bytes([body[14], body[15]]);
            if fmt == 0xFFFE && body.len() >= 26 {
                fmt = u16::from_le_bytes([body[24], body[25]]);
            }
        } else if id == b"data" {
            data = Some(body);
        }
        p += 8 + sz + (sz & 1);
    }
    let data = data.ok_or_else(|| bad("no data chunk"))?;
    if ch == 0 || sr == 0 {
        return Err(bad("bad fmt chunk"));
    }
    let bps = (bits / 8) as usize;
    let frame = bps * ch as usize;
    let n = data.len() / frame;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut acc = 0.0f32;
        for c in 0..ch as usize {
            let s = &data[i * frame + c * bps..i * frame + (c + 1) * bps];
            acc += match (fmt, bits) {
                (1, 8) => (s[0] as f32 - 128.0) / 128.0,
                (1, 16) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
                (1, 24) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8388608.0,
                (1, 32) => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2147483648.0,
                (3, 32) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                _ => return Err(bad("unsupported sample format")),
            };
        }
        out.push(acc / ch as f32);
    }
    Ok((out, sr))
}

pub fn read_wav(path: &str) -> io::Result<(Vec<f32>, u32)> {
    let mut b = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut b)?;
    decode_wav(&b)
}

const PHASES: usize = 256;

/// Streaming windowed-sinc resampler (table driven, no trig in the hot loop).
pub struct Resampler {
    ratio: f64, // input samples per output sample
    half: usize,
    table: Vec<f32>, // PHASES+1 rows of 2*half taps
    buf: Vec<f32>,
    pos: f64, // position of next output sample, in buf coordinates
    pub in_rate: u32,
    pub out_rate: u32,
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        let ratio = in_rate as f64 / out_rate as f64;
        let cut = 0.90 * (1.0f64).min(1.0 / ratio);
        let half = ((10.0 * ratio.max(1.0)).ceil() as usize).max(8);
        let mut table = vec![0f32; (PHASES + 1) * 2 * half];
        for ph in 0..=PHASES {
            let frac = ph as f64 / PHASES as f64;
            let mut sum = 0.0;
            for k in 0..2 * half {
                // tap k corresponds to input sample floor(pos) - half + 1 + k
                let d = (k as f64 - half as f64 + 1.0) - frac;
                let x = d * cut;
                let s = if x.abs() < 1e-9 {
                    1.0
                } else {
                    (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
                };
                let w = d / half as f64;
                let win = if w.abs() >= 1.0 {
                    0.0
                } else {
                    0.42 + 0.5 * (std::f64::consts::PI * w).cos()
                        + 0.08 * (2.0 * std::f64::consts::PI * w).cos()
                };
                let v = s * win * cut;
                table[ph * 2 * half + k] = v as f32;
                sum += v;
            }
            for k in 0..2 * half {
                table[ph * 2 * half + k] /= sum as f32;
            }
        }
        Resampler {
            ratio,
            half,
            table,
            buf: vec![0.0; half],
            pos: half as f64,
            in_rate,
            out_rate,
        }
    }

    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.in_rate == self.out_rate {
            out.extend_from_slice(input);
            return;
        }
        self.buf.extend_from_slice(input);
        let h = self.half;
        loop {
            let i = self.pos.floor() as usize;
            if i + h >= self.buf.len() {
                break;
            }
            let frac = self.pos - i as f64;
            let ph = (frac * PHASES as f64).round() as usize;
            let taps = &self.table[ph * 2 * h..(ph + 1) * 2 * h];
            let src = &self.buf[i + 1 - h..i + 1 + h];
            let mut acc = 0.0f32;
            for k in 0..2 * h {
                acc += taps[k] * src[k];
            }
            out.push(acc);
            self.pos += self.ratio;
        }
        // drop consumed samples, keep history
        let keep_from = (self.pos.floor() as usize).saturating_sub(h);
        if keep_from > 0 {
            self.buf.drain(..keep_from);
            self.pos -= keep_from as f64;
        }
    }
}

pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    let mut r = Resampler::new(from, to);
    let mut out = Vec::with_capacity((x.len() as f64 * to as f64 / from as f64) as usize + 16);
    r.push(x, &mut out);
    // flush tail
    r.push(&vec![0.0; r.half * 2], &mut out);
    out.truncate((x.len() as f64 * to as f64 / from as f64) as usize);
    out
}

pub fn rms_dbfs(x: &[f32]) -> f32 {
    if x.is_empty() {
        return -120.0;
    }
    let e: f32 = x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32;
    10.0 * (e + 1e-12).log10()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resample_keeps_tone() {
        let sr = 44100;
        let x: Vec<f32> = (0..sr)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin())
            .collect();
        let y = resample(&x, sr, 8000);
        assert!((y.len() as i64 - 8000).abs() < 2);
        let r = rms_dbfs(&y[1000..7000]);
        assert!((r - (-3.01)).abs() < 0.3, "rms {r}");
    }
}

// ------------------------------------------------------------------ augmentation

/// RBJ biquad, direct form I.
#[derive(Clone, Copy)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
}

impl Biquad {
    fn new(kind: &str, f0: f32, q: f32, gain_db: f32, sr: f32) -> Biquad {
        let w = 2.0 * std::f32::consts::PI * f0 / sr;
        let (s, c) = w.sin_cos();
        let al = s / (2.0 * q);
        let a = 10f32.powf(gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            "lp" => (
                (1.0 - c) / 2.0,
                1.0 - c,
                (1.0 - c) / 2.0,
                1.0 + al,
                -2.0 * c,
                1.0 - al,
            ),
            "hp" => (
                (1.0 + c) / 2.0,
                -(1.0 + c),
                (1.0 + c) / 2.0,
                1.0 + al,
                -2.0 * c,
                1.0 - al,
            ),
            _ => (
                1.0 + al * a,
                -2.0 * c,
                1.0 - al * a,
                1.0 + al / a,
                -2.0 * c,
                1.0 - al / a,
            ), // peaking
        };
        Biquad {
            b: [b0 / a0, b1 / a0, b2 / a0],
            a: [a1 / a0, a2 / a0],
        }
    }
    fn run(&self, x: &mut [f32]) {
        let (mut x1, mut x2, mut y1, mut y2) = (0f32, 0f32, 0f32, 0f32);
        for v in x.iter_mut() {
            let y =
                self.b[0] * *v + self.b[1] * x1 + self.b[2] * x2 - self.a[0] * y1 - self.a[1] * y2;
            x2 = x1;
            x1 = *v;
            y2 = y1;
            y1 = y;
            *v = y;
        }
    }
}

fn xorshift(s: &mut u64) -> f32 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    // approx. standard normal (sum of 4 uniforms)
    let u = |s: u64, k: u32| ((s >> (k * 16)) & 0xFFFF) as f32 / 65536.0;
    (u(*s, 0) + u(*s, 1) + u(*s, 2) + u(*s, 3) - 2.0) * 1.732
}

/// FFT convolution (overlap-add), output truncated to `x.len()`.
pub fn convolve(x: &[f32], h: &[f32]) -> Vec<f32> {
    let n = (h.len() * 2).next_power_of_two().max(1024);
    let block = n - h.len() + 1;
    let fft = crate::fft::Fft::new(n);
    let mut hr = vec![0f32; n];
    let mut hi = vec![0f32; n];
    hr[..h.len()].copy_from_slice(h);
    fft.run(&mut hr, &mut hi);
    let mut y = vec![0f32; x.len() + h.len()];
    let (mut br, mut bi) = (vec![0f32; n], vec![0f32; n]);
    let mut s = 0;
    while s < x.len() {
        let e = (s + block).min(x.len());
        br.iter_mut().for_each(|v| *v = 0.0);
        bi.iter_mut().for_each(|v| *v = 0.0);
        br[..e - s].copy_from_slice(&x[s..e]);
        fft.run(&mut br, &mut bi);
        for k in 0..n {
            let (a, b) = (br[k] * hr[k] - bi[k] * hi[k], br[k] * hi[k] + bi[k] * hr[k]);
            br[k] = a;
            bi[k] = -b; // conjugate: inverse FFT through the forward transform
        }
        fft.run(&mut br, &mut bi);
        for k in 0..(e - s + h.len() - 1).min(n) {
            y[s + k] += br[k] / n as f32;
        }
        s = e;
    }
    y.truncate(x.len());
    y
}

/// Simulates "played on a small speaker, in a room, recorded by a phone":
/// 300–6500 Hz band-pass, 1.8 kHz resonance, soft saturation, room reverb
/// (RT60 0.3–0.6 s, direct/reverberant 0–6 dB), mic high-pass and self-noise.
/// Deterministic for a given `seed`. Used to augment libraries (`--augment speaker`).
pub fn speaker_sim(x: &[f32], rate: u32, seed: u64) -> Vec<f32> {
    let sr = rate as f32;
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut y = x.to_vec();
    for f in [
        Biquad::new("hp", 300.0, 0.541, 0.0, sr),
        Biquad::new("hp", 300.0, 1.307, 0.0, sr),
        Biquad::new("lp", 6500.0, 0.541, 0.0, sr),
        Biquad::new("lp", 6500.0, 1.307, 0.0, sr),
        Biquad::new("peak", 1800.0, 2.0, 4.0, sr),
    ] {
        f.run(&mut y);
    }
    let m = y.iter().fold(0f32, |a, v| a.max(v.abs())).max(1e-9);
    let k = 1.5f32;
    y.iter_mut()
        .for_each(|v| *v = (k * *v / m).tanh() / k.tanh());
    // room impulse response
    let u = |s: &mut u64| (xorshift(s) * 0.29 + 0.5).clamp(0.0, 1.0);
    let rt60 = 0.3 + 0.3 * u(&mut s);
    let drr_db = 6.0 * u(&mut s);
    let n = (rt60 * sr) as usize;
    let mut ir: Vec<f32> = (0..n)
        .map(|i| {
            if (i as f32) < 0.004 * sr {
                0.0
            } else {
                xorshift(&mut s) * (-6.9 * i as f32 / sr / rt60).exp()
            }
        })
        .collect();
    let e: f32 = ir.iter().map(|v| v * v).sum::<f32>().max(1e-12);
    let g = (10f32.powf(-drr_db / 10.0) / e).sqrt();
    ir.iter_mut().for_each(|v| *v *= g);
    ir[0] += 1.0;
    let early = ((0.003 + 0.005 * u(&mut s)) * sr) as usize;
    if early < ir.len() {
        ir[early] += 0.5;
    }
    let mut z = convolve(&y, &ir);
    Biquad::new("hp", 100.0, 0.707, 0.0, sr).run(&mut z);
    let m = z.iter().fold(0f32, |a, v| a.max(v.abs())).max(1e-9);
    z.iter_mut()
        .for_each(|v| *v = *v / m * 0.5 + 0.001 * xorshift(&mut s));
    z
}

#[cfg(test)]
mod aug_tests {
    use super::*;
    #[test]
    fn convolve_matches_direct() {
        let x: Vec<f32> = (0..3000)
            .map(|i| ((i * 7919) % 101) as f32 / 50.0 - 1.0)
            .collect();
        let h: Vec<f32> = (0..700)
            .map(|i| (-(i as f32) / 100.0).exp() * if i % 3 == 0 { 1.0 } else { -0.5 })
            .collect();
        let y = convolve(&x, &h);
        for t in [0usize, 1, 699, 700, 1500, 2999] {
            let d: f32 = (0..h.len())
                .filter(|&k| k <= t)
                .map(|k| h[k] * x[t - k])
                .sum();
            assert!((y[t] - d).abs() < 1e-3, "t {t}: {} vs {d}", y[t]);
        }
    }
    #[test]
    fn speaker_sim_is_deterministic_and_bandlimited() {
        let sr = 16000;
        let x: Vec<f32> = (0..sr)
            .map(|i| (2.0 * std::f32::consts::PI * 100.0 * i as f32 / sr as f32).sin())
            .collect();
        let a = speaker_sim(&x, sr as u32, 7);
        let b = speaker_sim(&x, sr as u32, 7);
        assert_eq!(a, b);
        // a 100 Hz tone is strongly attenuated by the 300 Hz high-pass
        assert!(rms_dbfs(&a[4000..]) < rms_dbfs(&x) - 10.0);
    }
}
