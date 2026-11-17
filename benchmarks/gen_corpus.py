#!/usr/bin/env python3
"""Generate a reproducible synthetic music corpus for benchmarking.

No copyrighted audio is available in CI, so we synthesise songs that have the
properties that matter for fingerprinting: harmonic content, percussive
transients, a bass line, a melody, chord changes, and — importantly — shared
building blocks across songs (same drum kits, same scales) so that the
matcher is stressed with near-duplicates.

Usage: gen_corpus.py OUT_DIR N_SONGS [DURATION_S] [SEED]
Writes OUT_DIR/audio/sNNNN.wav (22050 Hz mono 16-bit), OUT_DIR/meta.tsv and
OUT_DIR/lyrics/sNNNN.lrc (synthetic placeholder lyrics, timed to the bars).
"""
import os
import sys
import wave

import numpy as np

SR = 22050
SCALES = {
    "major": [0, 2, 4, 5, 7, 9, 11],
    "minor": [0, 2, 3, 5, 7, 8, 10],
    "dorian": [0, 2, 3, 5, 7, 9, 10],
    "penta": [0, 3, 5, 7, 10],
}
PROGS = [[0, 5, 3, 4], [0, 3, 4, 4], [0, 4, 5, 3], [5, 3, 0, 4], [0, 0, 3, 4], [1, 4, 0, 0]]
WORDS = ("la nuit le vent route soleil encore toujours pluie danse ville coeur "
         "lumière rêve plage étoile nous vous jamais demain loin ici feu ciel").split()


def midi_hz(m):
    return 440.0 * 2 ** ((m - 69) / 12)


def adsr(n, a=0.01, d=0.1, s=0.7, r=0.1):
    t = np.arange(n) / SR
    dur = n / SR
    env = np.ones(n) * s
    env[t < a] = t[t < a] / a
    m = (t >= a) & (t < a + d)
    env[m] = 1 - (1 - s) * (t[m] - a) / d
    m = t > dur - r
    env[m] *= np.clip((dur - t[m]) / r, 0, 1)
    return env


def tone(f, n, rng, harm, bright):
    t = np.arange(n) / SR
    out = np.zeros(n)
    det = 1 + rng.normal(0, 0.002)
    for h in range(1, harm + 1):
        if f * h > SR / 2.2:
            break
        out += np.sin(2 * np.pi * f * h * det * t + rng.uniform(0, 6.28)) / (h ** bright)
    return out


def drums(n_samples, bpm, rng, kit):
    out = np.zeros(n_samples)
    beat = int(SR * 60 / bpm)
    six = beat // 4
    pat_k = kit["kick"]
    pat_s = kit["snare"]
    pat_h = kit["hat"]
    for i in range(0, n_samples // six):
        pos = i * six
        step = i % 16
        if pat_k[step]:
            L = min(int(0.25 * SR), n_samples - pos)
            t = np.arange(L) / SR
            f = 50 + 90 * np.exp(-t * 30)
            out[pos:pos + L] += 0.9 * np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t * 12)
        if pat_s[step]:
            L = min(int(0.18 * SR), n_samples - pos)
            t = np.arange(L) / SR
            nz = rng.normal(0, 1, L)
            out[pos:pos + L] += 0.45 * (nz * np.exp(-t * 25) + 0.5 * np.sin(2 * np.pi * 190 * t) * np.exp(-t * 30))
        if pat_h[step]:
            L = min(int(0.05 * SR), n_samples - pos)
            nz = np.diff(rng.normal(0, 1, L + 1))
            out[pos:pos + L] += 0.12 * nz * np.exp(-np.arange(L) / SR * 80)
    return out


def make_kits(rng, k=6):
    kits = []
    for _ in range(k):
        kick = np.zeros(16, bool); kick[[0, 8]] = True
        kick[rng.choice(16, 2)] = True
        snare = np.zeros(16, bool); snare[[4, 12]] = True
        hat = np.zeros(16, bool); hat[::2] = True
        if rng.random() < .5:
            hat[1::2] = True
        kits.append({"kick": kick, "snare": snare, "hat": hat})
    return kits


def song(idx, dur, rng, kits):
    n = int(dur * SR)
    bpm = rng.uniform(78, 150)
    root = rng.integers(40, 52)
    scale = SCALES[list(SCALES)[rng.integers(len(SCALES))]]
    prog = PROGS[rng.integers(len(PROGS))]
    kit = kits[rng.integers(len(kits))]
    beat = 60 / bpm
    bar = 4 * beat
    mix = np.zeros(n)
    pad_h, pad_b = rng.integers(3, 9), rng.uniform(0.8, 2.0)
    lead_h, lead_b = rng.integers(2, 7), rng.uniform(0.6, 1.6)
    # chords + bass per bar
    nb = int(np.ceil(dur / bar))
    for b in range(nb):
        deg = prog[b % len(prog)]
        s = int(b * bar * SR)
        L = min(int(bar * SR), n - s)
        if L <= 0:
            break
        chord = [scale[(deg + k) % len(scale)] + 12 * ((deg + k) // len(scale)) for k in (0, 2, 4)]
        pad = sum(tone(midi_hz(root + 12 + c), L, rng, pad_h, pad_b) for c in chord)
        mix[s:s + L] += 0.12 * pad * adsr(L, 0.08, 0.3, 0.6, 0.2)
        for q in range(4):
            bs = s + int(q * beat * SR)
            bl = min(int(beat * SR * 0.9), n - bs)
            if bl <= 0:
                continue
            mix[bs:bs + bl] += 0.35 * tone(midi_hz(root + chord[0] - 12 + (12 if q == 2 else 0)), bl, rng, 4, 1.5) * adsr(bl, 0.005, 0.1, 0.5, 0.05)
    # lead melody random walk
    t = 0.0
    deg = rng.integers(0, len(scale))
    while t < dur:
        d = beat * rng.choice([0.5, 0.5, 1, 1, 1.5, 2])
        deg = int(np.clip(deg + rng.integers(-2, 3), 0, 2 * len(scale) - 1))
        if rng.random() > 0.15:
            s = int(t * SR); L = min(int(d * SR), n - s)
            if L > 0:
                m = root + 24 + scale[deg % len(scale)] + 12 * (deg // len(scale))
                vib = 1
                mix[s:s + L] += 0.22 * tone(midi_hz(m) * vib, L, rng, lead_h, lead_b) * adsr(L, 0.01, 0.08, 0.6, 0.05)
        t += d
    mix += drums(n, bpm, rng, kit)
    mix /= np.max(np.abs(mix)) + 1e-9
    mix *= 0.89
    # synthetic lyrics: one line every bar after a short intro
    lines = []
    tt = bar * 2
    while tt < dur - bar:
        lines.append((tt, " ".join(rng.choice(WORDS, rng.integers(3, 7)))))
        tt += bar * rng.choice([1, 1, 2])
    return mix.astype(np.float32), bpm, lines


def write_wav(path, x):
    pcm = (np.clip(x, -1, 1) * 32767).astype("<i2")
    with wave.open(path, "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(SR)
        w.writeframes(pcm.tobytes())


def fmt(t):
    m = int(t // 60)
    return f"[{m:02d}:{t - 60 * m:05.2f}]"


def main():
    out, N = sys.argv[1], int(sys.argv[2])
    dur = float(sys.argv[3]) if len(sys.argv) > 3 else 60
    seed = int(sys.argv[4]) if len(sys.argv) > 4 else 1234
    rng = np.random.default_rng(seed)
    kits = make_kits(rng)
    os.makedirs(f"{out}/audio", exist_ok=True)
    os.makedirs(f"{out}/lyrics", exist_ok=True)
    with open(f"{out}/meta.tsv", "w") as meta:
        meta.write("file\ttitle\tartist\talbum\tduration\n")
        for i in range(N):
            x, bpm, lines = song(i, dur, rng, kits)
            name = f"s{i:04d}"
            write_wav(f"{out}/audio/{name}.wav", x)
            with open(f"{out}/lyrics/{name}.lrc", "w") as f:
                for t, l in lines:
                    f.write(f"{fmt(t)}{l}\n")
            meta.write(f"audio/{name}.wav\tChanson {i}\tArtiste {i % 37}\tAlbum {i // 10}\t{dur:.1f}\n")
            if i % 50 == 0:
                print("generated", i, file=sys.stderr)


if __name__ == "__main__":
    main()
