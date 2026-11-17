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
