//! Framework-free neural network runtime for audio CNNs (`.rnm` files).
//!
//! A `.rnm` file = `b"RNNM" | u32 version | u32 header_len | JSON header | pad16 | tensors`.
//! The header describes the audio front-end (log-mel), a flat list of ops and
//! the tensors. Supported ops (enough for MobileNet V1/V2/V3, EfficientNet-B0
//! style CNNs; BatchNorm must be folded into convs by the exporter):
//!
//! | op | fields | effect |
//! |---|---|---|
//! | `conv` | w, b, cin, cout, k[2], stride[2], pad[2], groups, act | 2-D convolution (+bias, +activation) |
//! | `se` | w1, b1, w2, b2, act, gate | channel squeeze-excitation |
//! | `push` / `add` | — | save tensor / add saved tensor (residual) |
//! | `gap` | — | global average pooling → vector |
//! | `linear` | w, b, act | fully connected on the vector |
//! | `output` | name, post | expose the current vector (`post`: none / sigmoid / softmax / l2) |
//!
//! Activations: `none`, `relu`, `relu6`, `hswish`, `hsigmoid`, `sigmoid`, `silu`.
//! Tensors are `[channels][freq][time]` f32. Converting a new model = writing a
//! small exporter (see `tools/models/`), no Rust change.

use crate::fft::Fft;
use crate::json::Json;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    None,
    Relu,
    Relu6,
    HSwish,
    HSigmoid,
    Sigmoid,
    Silu,
}

impl Act {
    fn parse(s: &str) -> Result<Act, String> {
        Ok(match s {
            "none" | "" => Act::None,
            "relu" => Act::Relu,
            "relu6" => Act::Relu6,
            "hswish" => Act::HSwish,
            "hsigmoid" => Act::HSigmoid,
            "sigmoid" => Act::Sigmoid,
            "silu" | "swish" => Act::Silu,
            _ => return Err(format!("unknown activation '{s}'")),
        })
    }
    #[inline]
    fn apply(self, x: f32) -> f32 {
        match self {
            Act::None => x,
            Act::Relu => x.max(0.0),
            Act::Relu6 => x.clamp(0.0, 6.0),
            Act::HSwish => x * (x + 3.0).clamp(0.0, 6.0) / 6.0,
            Act::HSigmoid => (x + 3.0).clamp(0.0, 6.0) / 6.0,
            Act::Sigmoid => 1.0 / (1.0 + (-x).exp()),
            Act::Silu => x / (1.0 + (-x).exp()),
        }
    }
    fn apply_all(self, v: &mut [f32]) {
        if self != Act::None {
            for x in v.iter_mut() {
                *x = self.apply(*x);
            }
        }
    }
}

#[derive(Clone, Debug)]
enum Op {
    Conv {
        w: usize,
        b: usize,
        cin: usize,
        cout: usize,
        kh: usize,
        kw: usize,
        sh: usize,
        sw: usize,
        ph: usize,
        pw: usize,
        groups: usize,
        act: Act,
    },
    Se {
        w1: usize,
        b1: usize,
        w2: usize,
        b2: usize,
        act: Act,
        gate: Act,
    },
    Push,
    Add,
    Gap,
    Linear {
        w: usize,
        b: usize,
        act: Act,
    },
    Output {
        name: String,
        post: String,
    },
}

#[derive(Clone, Debug)]
pub struct Frontend {
    pub sample_rate: u32,
    pub n_fft: usize,
    pub hop: usize,
    pub preemphasis: f32,
    pub window: Vec<f32>,
    pub mel: Vec<f32>, // n_mels x (n_fft/2+1)
    pub n_mels: usize,
    pub log_eps: f32,
    pub norm_add: f32,
    pub norm_div: f32,
}

pub struct Model {
    pub name: String,
    pub header: Json,
    pub frontend: Frontend,
    pub labels: Vec<String>,
    ops: Vec<Op>,
    tensors: Vec<Vec<f32>>,
    shapes: Vec<Vec<usize>>,
    fft: Fft,
    pub threads: usize,
}

/// Named outputs of one forward pass (e.g. "embedding", "logits").
pub type Outputs = HashMap<String, Vec<f32>>;

fn f16_to_f32(h: u16) -> f32 {
    let s = ((h >> 15) & 1) as u32;
    let e = ((h >> 10) & 0x1F) as u32;
    let m = (h & 0x3FF) as u32;
    let bits = if e == 0 {
        if m == 0 {
            s << 31
        } else {
            // subnormal
            let mut e2 = 127 - 15 + 1;
            let mut m2 = m;
            while m2 & 0x400 == 0 {
                m2 <<= 1;
                e2 -= 1;
            }
            (s << 31) | ((e2 as u32) << 23) | ((m2 & 0x3FF) << 13)
        }
    } else if e == 31 {
        (s << 31) | (0xFF << 23) | (m << 13)
    } else {
        (s << 31) | ((e + 127 - 15) << 23) | (m << 13)
    };
    f32::from_bits(bits)
}

impl Model {
    pub fn load(path: &str) -> Result<Model, String> {
        let b = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        Model::from_bytes(&b)
    }

    pub fn from_bytes(b: &[u8]) -> Result<Model, String> {
        if b.len() < 12 || &b[0..4] != b"RNNM" {
            return Err("not a Reis neural model (.rnm)".into());
        }
        let ver = u32::from_le_bytes(b[4..8].try_into().unwrap());
        if ver != 1 {
            return Err(format!("unsupported .rnm version {ver}"));
        }
        let hl = u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
        let header =
            Json::parse(std::str::from_utf8(&b[12..12 + hl]).map_err(|_| "bad header utf8")?)?;
        let data_start = 12 + hl + (16 - (12 + hl) % 16) % 16;
        let data = &b[data_start..];
        // tensors
        let mut idx: HashMap<String, usize> = HashMap::new();
        let mut tensors = Vec::new();
        let mut shapes = Vec::new();
        for (name, t) in header
            .get("tensors")
            .and_then(|t| t.obj())
            .ok_or("header: no tensors")?
        {
            let shape = t.get("shape").map(|s| s.usizes()).unwrap_or_default();
            let n: usize = shape.iter().product::<usize>().max(1);
            let off = t.get_usize("offset").ok_or("tensor offset")?;
            let v: Vec<f32> = match t.get_str("dtype").unwrap_or("f32") {
                "f16" => (0..n)
                    .map(|i| {
                        f16_to_f32(u16::from_le_bytes([
                            data[off + 2 * i],
                            data[off + 2 * i + 1],
                        ]))
                    })
                    .collect(),
                "f32" => (0..n)
                    .map(|i| {
                        f32::from_le_bytes(data[off + 4 * i..off + 4 * i + 4].try_into().unwrap())
                    })
                    .collect(),
                d => return Err(format!("unsupported dtype {d}")),
            };
            idx.insert(name.clone(), tensors.len());
            tensors.push(v);
            shapes.push(shape);
        }
        let t = |j: &Json, k: &str| -> Result<usize, String> {
            let n = j.get_str(k).ok_or(format!("op field '{k}'"))?;
            idx.get(n).copied().ok_or(format!("missing tensor {n}"))
        };
        let act = |j: &Json, k: &str| Act::parse(j.get_str(k).unwrap_or("none"));
        let mut ops = Vec::new();
        for o in header
            .get("ops")
            .and_then(|o| o.arr())
            .ok_or("header: no ops")?
        {
            let op = match o.get_str("op").unwrap_or("") {
                "conv" => {
                    let k = o.get("k").map(|v| v.usizes()).unwrap_or_default();
                    let s = o.get("stride").map(|v| v.usizes()).unwrap_or(vec![1, 1]);
                    let p = o.get("pad").map(|v| v.usizes()).unwrap_or(vec![0, 0]);
                    Op::Conv {
                        w: t(o, "w")?,
                        b: t(o, "b")?,
                        cin: o.get_usize("cin").ok_or("cin")?,
                        cout: o.get_usize("cout").ok_or("cout")?,
                        kh: k[0],
                        kw: k[1],
                        sh: s[0],
                        sw: s[1],
                        ph: p[0],
                        pw: p[1],
                        groups: o.get_usize("groups").unwrap_or(1),
                        act: act(o, "act")?,
                    }
                }
                "se" => Op::Se {
                    w1: t(o, "w1")?,
                    b1: t(o, "b1")?,
                    w2: t(o, "w2")?,
                    b2: t(o, "b2")?,
                    act: act(o, "act")?,
                    gate: Act::parse(o.get_str("gate").unwrap_or("sigmoid"))?,
                },
                "push" => Op::Push,
                "add" => Op::Add,
                "gap" => Op::Gap,
                "linear" => Op::Linear {
                    w: t(o, "w")?,
                    b: t(o, "b")?,
                    act: act(o, "act")?,
                },
                "output" => Op::Output {
                    name: o.get_str("name").unwrap_or("out").to_string(),
                    post: o.get_str("post").unwrap_or("none").to_string(),
                },
                x => return Err(format!("unsupported op '{x}'")),
            };
            ops.push(op);
        }
        let fe = header.get("frontend").ok_or("header: no frontend")?;
        if fe.get_str("type") != Some("logmel") {
            return Err("only the 'logmel' front-end is supported".into());
        }
        let wi = t(fe, "window")?;
        let mi = t(fe, "mel")?;
        let n_fft = fe.get_usize("n_fft").ok_or("n_fft")?;
        let frontend = Frontend {
            sample_rate: fe.get_num("sample_rate").ok_or("sample_rate")? as u32,
            n_fft,
            hop: fe.get_usize("hop").ok_or("hop")?,
            preemphasis: fe.get_num("preemphasis").unwrap_or(0.0) as f32,
            window: tensors[wi].clone(),
            n_mels: shapes[mi][0],
            mel: tensors[mi].clone(),
            log_eps: fe.get_num("log_eps").unwrap_or(1e-5) as f32,
            norm_add: fe.get_num("norm_add").unwrap_or(0.0) as f32,
            norm_div: fe.get_num("norm_div").unwrap_or(1.0) as f32,
        };
        if frontend.window.len() != n_fft || shapes[mi][1] != n_fft / 2 + 1 {
            return Err("front-end tensor shapes do not match n_fft".into());
        }
        let labels = header
            .get("labels")
            .and_then(|l| l.arr())
            .map(|a| a.iter().filter_map(|v| v.str().map(String::from)).collect())
            .unwrap_or_default();
        Ok(Model {
            name: header.get_str("name").unwrap_or("model").to_string(),
            fft: Fft::new(n_fft),
            header,
            frontend,
            labels,
            ops,
            tensors,
            shapes,
            threads: std::thread::available_parallelism()
                .map(|v| v.get())
                .unwrap_or(1),
        })
    }

    pub fn param_count(&self) -> usize {
        self.tensors.iter().map(|t| t.len()).sum::<usize>()
            - self.frontend.mel.len()
            - self.frontend.window.len()
    }

    /// Log-mel spectrogram `[n_mels][frames]` of mono audio at `frontend.sample_rate`.
    pub fn logmel(&self, x: &[f32]) -> (Vec<f32>, usize) {
        let f = &self.frontend;
        let n = f.n_fft;
        // pre-emphasis y[i] = x[i+1] - a * x[i]  (length - 1)
        let y: Vec<f32> = if f.preemphasis != 0.0 && x.len() > 1 {
            (0..x.len() - 1)
                .map(|i| x[i + 1] - f.preemphasis * x[i])
                .collect()
        } else {
            x.to_vec()
        };
        // centre with reflect padding (zero padding if too short to reflect)
        let half = n / 2;
        let len = y.len();
        let mut p = vec![0f32; len + 2 * half];
        p[half..half + len].copy_from_slice(&y);
        if len > half {
            for i in 0..half {
                p[half - 1 - i] = y[i + 1];
                p[half + len + i] = y[len - 2 - i];
            }
        }
        let frames = if p.len() >= n {
            1 + (p.len() - n) / f.hop
        } else {
            0
        };
        let nb = n / 2 + 1;
        let mut out = vec![0f32; f.n_mels * frames];
        let (mut re, mut im) = (Vec::with_capacity(n), Vec::with_capacity(n));
        let mut frame = vec![0f32; n];
        let mut pw = vec![0f32; nb];
        for t in 0..frames {
            for i in 0..n {
                frame[i] = p[t * f.hop + i] * f.window[i];
            }
            self.fft.power(&frame, &mut re, &mut im, &mut pw);
            for m in 0..f.n_mels {
                let row = &f.mel[m * nb..(m + 1) * nb];
                let mut acc = 0f32;
                for k in 0..nb {
                    acc += row[k] * pw[k];
                }
                out[m * frames + t] = ((acc + f.log_eps).ln() + f.norm_add) / f.norm_div;
            }
        }
        (out, frames)
    }

    /// Run the network on a log-mel `[n_mels][frames]`.
    pub fn forward_mel(&self, mel: &[f32], frames: usize) -> Outputs {
        let mut x = mel.to_vec();
        let (mut c, mut h, mut w) = (1usize, self.frontend.n_mels, frames);
        let mut vec_mode = false;
        let mut stack: Vec<Vec<f32>> = Vec::new();
        let mut outs = Outputs::new();
        for op in &self.ops {
            match op {
                Op::Conv {
                    w: wi,
                    b: bi,
                    cin,
                    cout,
                    kh,
                    kw,
                    sh,
                    sw,
                    ph,
                    pw,
                    groups,
                    act,
                } => {
                    assert_eq!(c, *cin, "conv input channels");
                    let (oh, ow) = ((h + 2 * ph - kh) / sh + 1, (w + 2 * pw - kw) / sw + 1);
                    let mut y = vec![0f32; cout * oh * ow];
                    let wt = &self.tensors[*wi];
                    let bs = &self.tensors[*bi];
                    if *kh == 1 && *kw == 1 && *sh == 1 && *sw == 1 && *groups == 1 {
                        pointwise(&x, wt, bs, *cin, *cout, h * w, &mut y, self.threads);
                    } else if *groups == *cin && *groups == *cout {
                        depthwise(
                            &x,
                            wt,
                            bs,
                            *cin,
                            h,
                            w,
                            *kh,
                            *kw,
                            *sh,
                            *sw,
                            *ph,
                            *pw,
                            oh,
                            ow,
                            &mut y,
                            self.threads,
                        );
                    } else {
                        conv_general(
                            &x, wt, bs, *cin, *cout, *groups, h, w, *kh, *kw, *sh, *sw, *ph, *pw,
                            oh, ow, &mut y,
                        );
                    }
                    act.apply_all(&mut y);
                    x = y;
                    c = *cout;
                    h = oh;
                    w = ow;
                }
                Op::Se {
                    w1,
                    b1,
                    w2,
                    b2,
                    act,
                    gate,
                } => {
                    let hw = h * w;
                    let s: Vec<f32> = (0..c)
                        .map(|ch| x[ch * hw..(ch + 1) * hw].iter().sum::<f32>() / hw as f32)
                        .collect();
                    let mut z = linear(&s, &self.tensors[*w1], &self.tensors[*b1]);
                    act.apply_all(&mut z);
                    let mut g = linear(&z, &self.tensors[*w2], &self.tensors[*b2]);
                    gate.apply_all(&mut g);
                    for ch in 0..c {
                        for v in &mut x[ch * hw..(ch + 1) * hw] {
                            *v *= g[ch];
                        }
                    }
                }
                Op::Push => stack.push(x.clone()),
                Op::Add => {
                    let s = stack.pop().expect("add without push");
                    assert_eq!(s.len(), x.len(), "residual shape");
                    for (a, b) in x.iter_mut().zip(s) {
                        *a += b;
                    }
                }
                Op::Gap => {
                    let hw = (h * w).max(1);
                    x = (0..c)
                        .map(|ch| x[ch * hw..(ch + 1) * hw].iter().sum::<f32>() / hw as f32)
                        .collect();
                    vec_mode = true;
                    h = 1;
                    w = 1;
                }
                Op::Linear { w: wi, b: bi, act } => {
                    assert!(vec_mode, "linear before gap");
                    let mut y = linear(&x, &self.tensors[*wi], &self.tensors[*bi]);
                    act.apply_all(&mut y);
                    c = y.len();
                    x = y;
                }
                Op::Output { name, post } => {
                    let mut v = x.clone();
                    match post.as_str() {
                        "sigmoid" => Act::Sigmoid.apply_all(&mut v),
                        "softmax" => {
                            let m = v.iter().cloned().fold(f32::MIN, f32::max);
                            let s: f32 = v.iter().map(|x| (x - m).exp()).sum();
                            v.iter_mut().for_each(|x| *x = (*x - m).exp() / s);
                        }
                        "l2" => l2_normalize(&mut v),
                        _ => {}
                    }
                    outs.insert(name.clone(), v);
                }
            }
        }
        outs
    }

    /// Audio (mono, any rate) → outputs. Resamples to the model rate.
    pub fn forward_audio(&self, x: &[f32], rate: u32) -> Outputs {
        let y = if rate == self.frontend.sample_rate {
            x.to_vec()
        } else {
            crate::audio::resample(x, rate, self.frontend.sample_rate)
        };
        let (m, frames) = self.logmel(&y);
        self.forward_mel(&m, frames.max(1))
    }

    pub fn shape_of(&self, i: usize) -> &[usize] {
        &self.shapes[i]
    }
}

pub fn l2_normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    v.iter_mut().for_each(|x| *x /= n);
}

/// y = W x + b, W is [out][in].
fn linear(x: &[f32], w: &[f32], b: &[f32]) -> Vec<f32> {
    let n_in = x.len();
    let n_out = b.len();
    (0..n_out)
        .map(|o| {
            b[o] + w[o * n_in..(o + 1) * n_in]
                .iter()
                .zip(x)
                .map(|(a, b)| a * b)
                .sum::<f32>()
        })
        .collect()
}

fn par_rows<F: Fn(usize, &mut [f32]) + Sync>(
    y: &mut [f32],
    rows: usize,
    row_len: usize,
    threads: usize,
    work: usize,
    f: F,
) {
    if threads <= 1 || work < 2_000_000 {
        for (r, out) in y.chunks_mut(row_len).enumerate() {
            f(r, out);
        }
        return;
    }
    let per = rows.div_ceil(threads);
    std::thread::scope(|s| {
        for (ti, chunk) in y.chunks_mut(per * row_len).enumerate() {
            let f = &f;
            s.spawn(move || {
                for (k, out) in chunk.chunks_mut(row_len).enumerate() {
                    f(ti * per + k, out);
                }
            });
        }
    });
}

/// 1x1 convolution = [cout x cin] · [cin x p] GEMM, rows split across threads.
fn pointwise(
    x: &[f32],
    w: &[f32],
    b: &[f32],
    cin: usize,
    cout: usize,
    p: usize,
    y: &mut [f32],
    threads: usize,
) {
    par_rows(y, cout, p, threads, cout * cin * p, |co, out| {
        out.iter_mut().for_each(|v| *v = b[co]);
        let wr = &w[co * cin..(co + 1) * cin];
        for ci in 0..cin {
            let a = wr[ci];
            if a == 0.0 {
                continue;
            }
            let xr = &x[ci * p..(ci + 1) * p];
            for (o, v) in out.iter_mut().zip(xr) {
                *o += a * v;
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn depthwise(
    x: &[f32],
    w: &[f32],
    b: &[f32],
    c: usize,
    h: usize,
    wd: usize,
    kh: usize,
    kw: usize,
    sh: usize,
    sw: usize,
    ph: usize,
    pw: usize,
    oh: usize,
    ow: usize,
    y: &mut [f32],
    threads: usize,
) {
    par_rows(
        y,
        c,
        oh * ow,
        threads,
        c * oh * ow * kh * kw * 4,
        |ch, out| {
            let xi = &x[ch * h * wd..(ch + 1) * h * wd];
            let k = &w[ch * kh * kw..(ch + 1) * kh * kw];
            out.iter_mut().for_each(|v| *v = b[ch]);
            for i in 0..kh {
                for j in 0..kw {
                    let kv = k[i * kw + j];
                    for oy in 0..oh {
                        let iy = (oy * sh + i) as isize - ph as isize;
                        if iy < 0 || iy >= h as isize {
                            continue;
                        }
                        let row = &xi[iy as usize * wd..(iy as usize + 1) * wd];
                        let orow = &mut out[oy * ow..(oy + 1) * ow];
                        // valid ox range: 0 <= ox*sw + j - pw < wd
                        let lo = if pw > j { (pw - j).div_ceil(sw) } else { 0 };
                        let hi = if wd + pw > j {
                            ((wd + pw - j - 1) / sw + 1).min(ow)
                        } else {
                            0
                        };
                        if sw == 1 {
                            let base = lo + j - pw;
                            for (o, v) in orow[lo..hi].iter_mut().zip(&row[base..base + (hi - lo)])
                            {
                                *o += kv * v;
                            }
                        } else {
                            for ox in lo..hi {
                                orow[ox] += kv * row[ox * sw + j - pw];
                            }
                        }
                    }
                }
            }
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn conv_general(
    x: &[f32],
    w: &[f32],
    b: &[f32],
    cin: usize,
    cout: usize,
    groups: usize,
    h: usize,
    wd: usize,
    kh: usize,
    kw: usize,
    sh: usize,
    sw: usize,
    ph: usize,
    pw: usize,
    oh: usize,
    ow: usize,
    y: &mut [f32],
) {
    let cig = cin / groups;
    let cog = cout / groups;
    for co in 0..cout {
        let g = co / cog;
        let out = &mut y[co * oh * ow..(co + 1) * oh * ow];
        out.iter_mut().for_each(|v| *v = b[co]);
        for cl in 0..cig {
            let ci = g * cig + cl;
            let xi = &x[ci * h * wd..(ci + 1) * h * wd];
            for i in 0..kh {
                for j in 0..kw {
                    let kv = w[((co * cig + cl) * kh + i) * kw + j];
                    for oy in 0..oh {
                        let iy = (oy * sh + i) as isize - ph as isize;
                        if iy < 0 || iy >= h as isize {
                            continue;
                        }
                        let row = &xi[iy as usize * wd..(iy as usize + 1) * wd];
                        for ox in 0..ow {
                            let ix = (ox * sw + j) as isize - pw as isize;
                            if ix >= 0 && (ix as usize) < wd {
                                out[oy * ow + ox] += kv * row[ix as usize];
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn f16_values() {
        assert_eq!(f16_to_f32(0x3C00), 1.0);
        assert_eq!(f16_to_f32(0xC000), -2.0);
        assert!((f16_to_f32(0x3555) - 0.33325).abs() < 1e-4);
        assert!(f16_to_f32(0x0001) > 0.0 && f16_to_f32(0x0001) < 1e-7);
    }
    #[test]
    fn depthwise_matches_general() {
        let (c, h, w, k) = (3, 7, 9, 3);
        let x: Vec<f32> = (0..c * h * w)
            .map(|i| ((i * 37) % 11) as f32 - 5.0)
            .collect();
        let wt: Vec<f32> = (0..c * k * k)
            .map(|i| ((i * 13) % 7) as f32 * 0.1 - 0.3)
            .collect();
        let b = vec![0.5, -0.25, 1.0];
        for (s, p) in [(1, 1), (2, 1), (2, 0)] {
            let (oh, ow) = ((h + 2 * p - k) / s + 1, (w + 2 * p - k) / s + 1);
            let mut a = vec![0.0; c * oh * ow];
            let mut g = vec![0.0; c * oh * ow];
            depthwise(&x, &wt, &b, c, h, w, k, k, s, s, p, p, oh, ow, &mut a, 1);
            conv_general(&x, &wt, &b, c, c, c, h, w, k, k, s, s, p, p, oh, ow, &mut g);
            for (u, v) in a.iter().zip(&g) {
                assert!((u - v).abs() < 1e-4, "stride {s} pad {p}: {u} vs {v}");
            }
        }
    }
}
