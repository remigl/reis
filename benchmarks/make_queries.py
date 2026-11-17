#!/usr/bin/env python3
"""Build a reproducible query set with realistic degradations.

Usage: make_queries.py DB_DIR NEG_DIR OUT_DIR [N_POS] [N_NEG] [SEED]

For every (condition, duration) pair, N_POS excerpts of catalogued songs and
N_NEG excerpts of songs NOT in the catalogue (false-positive measurement).
Writes OUT_DIR/q/*.wav (16 kHz mono, like a phone mic stream) and
OUT_DIR/queries.tsv: file, expected_song (-1 = negative), true_offset_s, cond, dur
"""
import os
import subprocess
import sys
import tempfile
import wave

import numpy as np
from scipy import signal

SR = 16000
DURS = [2, 3, 5, 10]
CONDS = ["clean", "speaker", "noise10", "noise0", "speaker_noise5", "lowvol", "highvol", "mp3_64k", "aac_48k", "opus_16k"]


def read_wav(p):
    with wave.open(p) as w:
        x = np.frombuffer(w.readframes(w.getnframes()), "<i2").astype(np.float32) / 32768
        return x, w.getframerate()


def write_wav(p, x, sr=SR):
    with wave.open(p, "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(sr)
        w.writeframes((np.clip(x, -1, 1) * 32767).astype("<i2").tobytes())


def pink(n, rng):
    w = rng.normal(0, 1, n)
    b, a = [0.049922035, -0.095993537, 0.050612699, -0.004408786], [1, -2.494956002, 2.017265875, -0.522189400]
    y = signal.lfilter(b, a, w)
    return y / (np.std(y) + 1e-9)


def snr_mix(x, noise, snr_db):
    px = np.mean(x ** 2) + 1e-12
    pn = np.mean(noise ** 2) + 1e-12
    return x + noise * np.sqrt(px / pn / 10 ** (snr_db / 10))


def room_ir(rng, rt60=None, drr_db=None):
    """Living-room-like impulse response: direct path + early reflection +
    exponentially decaying diffuse tail, with a direct-to-reverberant ratio
    typical of a phone held 0.5-2 m from a speaker (0..+6 dB)."""
    rt60 = rt60 or rng.uniform(0.3, 0.6)
    drr_db = rng.uniform(0, 6) if drr_db is None else drr_db
    n = int(rt60 * SR)
    t = np.arange(n) / SR
    tail = rng.normal(0, 1, n) * np.exp(-6.9 * t / rt60)
    tail[: int(0.004 * SR)] = 0
    tail *= np.sqrt(10 ** (-drr_db / 10) / np.sum(tail ** 2))
    ir = tail
    ir[0] += 1.0
    ir[int(rng.uniform(0.003, 0.008) * SR)] += 0.5
    return ir


def speaker_chain(x, rng):
    # small phone/laptop speaker: no lows, peaky mids, soft saturation
    sos = signal.butter(4, [300, 6500], "bandpass", fs=SR, output="sos")
    y = signal.sosfilt(sos, x)
    b, a = signal.iirpeak(1800, 2.0, fs=SR)
    y = y + 0.6 * signal.lfilter(b, a, y)
    y = y / (np.max(np.abs(y)) + 1e-9)
    y = np.tanh(1.5 * y) / np.tanh(1.5)  # moderate speaker saturation
    y = signal.fftconvolve(y, room_ir(rng))[: len(x)]
    # mic: highpass + self-noise
    y = signal.sosfilt(signal.butter(2, 100, "highpass", fs=SR, output="sos"), y)
    y = y / (np.max(np.abs(y)) + 1e-9) * 0.5
    return y + rng.normal(0, 10 ** (-60 / 20), len(y))


def codec(x, args, ext):
    with tempfile.TemporaryDirectory() as d:
        a, b, c = f"{d}/a.wav", f"{d}/b.{ext}", f"{d}/c.wav"
        write_wav(a, x)
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", a, *args, b], check=True)
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", b, "-ac", "1", "-ar", str(SR), c], check=True)
        y, _ = read_wav(c)
    # codec delay compensation is not applied on purpose: real files have it too
    return np.pad(y, (0, max(0, len(x) - len(y))))[: len(x)]


def degrade(x, cond, rng, interf):
    if cond == "clean":
        return x
    if cond == "speaker":
        return speaker_chain(x, rng)
    if cond in ("noise10", "noise0"):
        n = 0.6 * pink(len(x), rng) + 0.8 * interf / (np.std(interf) + 1e-9)
        return snr_mix(x, n, 10 if cond == "noise10" else 0)
    if cond == "speaker_noise5":
        y = speaker_chain(x, rng)
        n = 0.6 * pink(len(x), rng) + 0.8 * interf / (np.std(interf) + 1e-9)
        return snr_mix(y, n, 5)
    if cond == "lowvol":
        y = x * 10 ** (-38 / 20)
        y = y + rng.normal(0, 10 ** (-70 / 20), len(y))
        return np.round(y * 32767) / 32767
    if cond == "highvol":
        return np.clip(x * 10 ** (18 / 20), -1, 1)
    if cond == "mp3_64k":
        return codec(x, ["-c:a", "libmp3lame", "-b:a", "64k"], "mp3")
    if cond == "aac_48k":
        return codec(x, ["-c:a", "aac", "-b:a", "48k"], "m4a")
    if cond == "opus_16k":
        return codec(x, ["-c:a", "libopus", "-b:a", "16k"], "opus")
    raise ValueError(cond)


def main():
    db, neg, out = sys.argv[1:4]
    npos = int(sys.argv[4]) if len(sys.argv) > 4 else 60
    nneg = int(sys.argv[5]) if len(sys.argv) > 5 else 20
    rng = np.random.default_rng(int(sys.argv[6]) if len(sys.argv) > 6 else 7)
    os.makedirs(f"{out}/q", exist_ok=True)
    dbs = sorted(f for f in os.listdir(f"{db}/audio") if f.endswith(".wav"))
    negs = sorted(f for f in os.listdir(f"{neg}/audio") if f.endswith(".wav"))
    cache = {}

    def load(path):
        if path not in cache:
            x, sr = read_wav(path)
            cache[path] = signal.resample_poly(x, SR, sr).astype(np.float32)
        return cache[path]

    rows = ["file\texpected\ttrue_offset\tcond\tdur"]
    k = 0
    for cond in CONDS:
        for dur in DURS:
            for j in range(npos + nneg):
                is_neg = j >= npos
                src = negs[rng.integers(len(negs))] if is_neg else dbs[rng.integers(len(dbs))]
                x = load(f"{neg if is_neg else db}/audio/{src}")
                start = rng.uniform(0, len(x) / SR - dur - 0.5)
                seg = x[int(start * SR): int(start * SR) + int(dur * SR)]
                ix = load(f"{neg}/audio/{negs[rng.integers(len(negs))]}")
                s2 = rng.integers(0, len(ix) - len(seg))
                y = degrade(seg.astype(np.float64), cond, rng, ix[s2:s2 + len(seg)].astype(np.float64))
                name = f"q/{k:05d}.wav"
                write_wav(f"{out}/{name}", y)
                exp = -1 if is_neg else int(src[1:5])
                rows.append(f"{name}\t{exp}\t{start:.4f}\t{cond}\t{dur}")
                k += 1
        print("done", cond, file=sys.stderr)
    with open(f"{out}/queries.tsv", "w") as f:
        f.write("\n".join(rows) + "\n")


if __name__ == "__main__":
    main()
