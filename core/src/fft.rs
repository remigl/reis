//! Minimal iterative radix-2 FFT (std only). Sizes are powers of two.

pub struct Fft {
    n: usize,
    rev: Vec<u32>,
    tw_re: Vec<f32>,
    tw_im: Vec<f32>,
}

impl Fft {
    pub fn new(n: usize) -> Self {
        assert!(n.is_power_of_two() && n >= 4);
        let bits = n.trailing_zeros();
        let rev = (0..n as u32)
            .map(|i| i.reverse_bits() >> (32 - bits))
            .collect();
        let (mut tw_re, mut tw_im) = (Vec::with_capacity(n / 2), Vec::with_capacity(n / 2));
        for k in 0..n / 2 {
            let a = -2.0 * std::f64::consts::PI * k as f64 / n as f64;
            tw_re.push(a.cos() as f32);
            tw_im.push(a.sin() as f32);
        }
        Fft {
            n,
            rev,
            tw_re,
            tw_im,
        }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    /// In-place complex FFT.
    pub fn run(&self, re: &mut [f32], im: &mut [f32]) {
        let n = self.n;
        for i in 0..n {
            let j = self.rev[i] as usize;
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let step = n / len;
            let mut start = 0;
            while start < n {
                for k in 0..half {
                    let (wr, wi) = (self.tw_re[k * step], self.tw_im[k * step]);
                    let a = start + k;
                    let b = a + half;
                    let xr = re[b] * wr - im[b] * wi;
                    let xi = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - xr;
                    im[b] = im[a] - xi;
                    re[a] += xr;
                    im[a] += xi;
                }
                start += len;
            }
            len <<= 1;
        }
    }

    /// Power spectrum of a real frame (bins 0..n/2).
    pub fn power(&self, frame: &[f32], re: &mut Vec<f32>, im: &mut Vec<f32>, out: &mut [f32]) {
        re.clear();
        re.extend_from_slice(frame);
        im.clear();
        im.resize(self.n, 0.0);
        self.run(re, im);
        for k in 0..out.len() {
            out[k] = re[k] * re[k] + im[k] * im[k];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sine_peak() {
        let n = 1024;
        let f = Fft::new(n);
        let x: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * 50.0 * i as f32 / n as f32).sin())
            .collect();
        let (mut re, mut im) = (vec![], vec![]);
        let mut p = vec![0.0; n / 2];
        f.power(&x, &mut re, &mut im, &mut p);
        let k = p
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert_eq!(k, 50);
    }
}
