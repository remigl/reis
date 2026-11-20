#!/usr/bin/env python3
"""Lay out ESC-50 (https://github.com/karolpiczak/ESC-50, CC BY-NC 3.0) as a
Reis library + labelled query folders, with realistic degradations.

    prepare_esc50.py ESC50_DIR OUT_DIR [--holdout 10]

OUT_DIR/ref/<category>/*.wav       folds 1-4 of the *known* categories  (library)
OUT_DIR/q_<cond>/<category>/*.wav  fold 5 of all 50 categories          (queries)
                                   held-out categories must be rejected (open set)
OUT_DIR/same_<cond>/<category>/    3-s excerpts of *reference* clips    (exact engine)
Conditions: clean, speaker (phone speaker + room + mic), noise10 (pink noise +
another ESC sound at 10 dB SNR), speaker_noise5.
"""
import argparse
import csv
import os
import sys

import numpy as np
from scipy import signal

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import make_queries as mq  # noqa: E402

CONDS = ["clean", "speaker", "noise10", "speaker_noise5"]


def load(path):
    x, sr = mq.read_wav(path)
    return signal.resample_poly(x, mq.SR, sr).astype(np.float64)


def degrade(x, cond, rng, interf):
    if cond == "clean":
        return x
    # background noise only (pink + brown): mixing another ESC sound would make
    # the "right" answer ambiguous (two sound events at once)
    brown = np.cumsum(rng.standard_normal(len(x)))
    brown = signal.sosfilt(signal.butter(1, 20, "highpass", fs=mq.SR, output="sos"), brown)
    n = mq.pink(len(x), rng) + 0.5 * brown / (np.std(brown) + 1e-9)
    if cond == "speaker":
        return mq.speaker_chain(x, rng)
    if cond == "noise10":
        return mq.snr_mix(x, n, 10)
    if cond == "speaker_noise5":
        return mq.snr_mix(mq.speaker_chain(x, rng), n, 5)
    raise ValueError(cond)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("esc")
    ap.add_argument("out")
    ap.add_argument("--holdout", type=int, default=10)
    ap.add_argument("--augment-dir", default="", help="also write speaker-simulated copies of the references here (experiment)")
    a = ap.parse_args()
    rows = list(csv.DictReader(open(f"{a.esc}/meta/esc50.csv")))
    cats = sorted({r["category"] for r in rows})
    held = set(cats[::max(1, len(cats) // a.holdout)][: a.holdout])
    rng = np.random.default_rng(11)
    os.makedirs(a.out, exist_ok=True)
    with open(f"{a.out}/held_out.txt", "w") as f:
        f.write("\n".join(sorted(held)) + "\n")
    all_clips = [r["filename"] for r in rows]
    n_ref = n_q = 0
    for r in rows:
        src = f"{a.esc}/audio/{r['filename']}"
        cat = r["category"]
        if r["fold"] != "5" and cat not in held:
            d = f"{a.out}/ref/{cat}"
            os.makedirs(d, exist_ok=True)
            dst = f"{d}/{r['filename']}"
            if not os.path.exists(dst):
                os.symlink(os.path.abspath(src), dst)
            n_ref += 1
            if a.augment_dir:
                d2 = f"{a.augment_dir}/{cat}"
                os.makedirs(d2, exist_ok=True)
                if not os.path.exists(f"{d2}/{r['filename']}"):
                    os.symlink(os.path.abspath(src), f"{d2}/{r['filename']}")
                y = mq.speaker_chain(load(src), rng)
                mq.write_wav(f"{d2}/spk_{r['filename']}", y / max(1.0, np.max(np.abs(y))))
            # same-recording queries: 3 s excerpts of one reference out of 8
            if int(r["src_file"]) % 8 == 0:
                x = load(src)
                st = rng.integers(0, max(1, len(x) - 3 * mq.SR))
                seg = x[st:st + 3 * mq.SR]
                for cond in CONDS:
                    ix = load(f"{a.esc}/audio/{all_clips[rng.integers(len(all_clips))]}")[: len(seg)]
                    ix = np.pad(ix, (0, len(seg) - len(ix)))
                    y = degrade(seg, cond, rng, ix)
                    os.makedirs(f"{a.out}/same_{cond}/{cat}", exist_ok=True)
                    mq.write_wav(f"{a.out}/same_{cond}/{cat}/{r['filename']}", y / max(1.0, np.max(np.abs(y))))
        elif r["fold"] == "5":
            x = load(src)
            for cond in CONDS:
                ix = load(f"{a.esc}/audio/{all_clips[rng.integers(len(all_clips))]}")[: len(x)]
                ix = np.pad(ix, (0, len(x) - len(ix)))
                y = degrade(x, cond, rng, ix)
                os.makedirs(f"{a.out}/q_{cond}/{cat}", exist_ok=True)
                mq.write_wav(f"{a.out}/q_{cond}/{cat}/{r['filename']}", y / max(1.0, np.max(np.abs(y))))
            n_q += 1
    print(f"{len(cats) - len(held)} known categories, {len(held)} held out; {n_ref} references, {n_q} queries per condition")


if __name__ == "__main__":
    main()
