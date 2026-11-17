#!/usr/bin/env python3
"""End-to-end engine simulation (the real state machine, 20 ms mic callbacks).

Usage: sim_engine.py REIS_BIN CATALOG DB_DIR NEG_DIR OUT_DIR [N]

Scenarios per song (random start): clean, speaker, speaker+noise5,
pause (4 s of silence at t=20 s), clock drift (+0.2 % playback speed), and
songs that are NOT in the catalog (false locks). Reports time-to-lock,
position error while locked, mic duty cycle.
"""
import os
import re
import subprocess
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import make_queries as mq  # noqa: E402

PAT = re.compile(r"time-to-lock (\S+)s \| pos err p50 (\S+) ms p95 (\S+) ms max (\S+) ms \| mic duty (\S+)% \| matches (\d+) corrections (\d+) \| pause detected (\w+) \| cpu (\S+) ms/s")


def run(bin_, cat, wav, start, extra):
    out = subprocess.run([bin_, "simulate", cat, wav, "--start", f"{start:.2f}", "--secs", "40", *extra], capture_output=True, text=True).stdout
    m = PAT.search(out)
    if not m:
        return None
    ttl, p50, p95, mx, duty, matches, corr, pause, cpu = m.groups()
    return dict(ttl=float(ttl), p50=float(p50), p95=float(p95), max=float(mx), duty=float(duty), cpu=float(cpu), pause=pause == "true", raw=out)


def main():
    bin_, cat, db, neg, out = sys.argv[1:6]
    n = int(sys.argv[6]) if len(sys.argv) > 6 else 20
    rng = np.random.default_rng(3)
    os.makedirs(out, exist_ok=True)
    songs = sorted(os.listdir(f"{db}/audio"))[:: max(1, 300 // n)][:n]
    res = {k: [] for k in ["clean", "speaker", "speaker_noise5", "pause", "drift+0.2%"]}
    for s in songs:
        x, sr = mq.read_wav(f"{db}/audio/{s}")
        from scipy import signal
        x16 = signal.resample_poly(x, mq.SR, sr)
        ix, _ = mq.read_wav(f"{neg}/audio/{sorted(os.listdir(neg + '/audio'))[rng.integers(60)]}")
        ix = signal.resample_poly(ix, mq.SR, sr)[: len(x16)]
        ix = np.pad(ix, (0, len(x16) - len(ix)))
        variants = {
            "clean": x16,
            "speaker": mq.speaker_chain(x16, rng),
            "speaker_noise5": mq.snr_mix(mq.speaker_chain(x16, rng), 0.6 * mq.pink(len(x16), rng) + 0.8 * ix / (np.std(ix) + 1e-9), 5),
        }
        for k, v in variants.items():
            p = f"{out}/{k}_{s}"
            mq.write_wav(p, v / max(1.0, np.max(np.abs(v))))
            start = rng.uniform(0, 10)
            r = run(bin_, cat, p, start, [])
            if r:
                res[k].append(r)
        start = rng.uniform(0, 5)
        r = run(bin_, cat, f"{out}/speaker_{s}", start, ["--pause-at", "20", "--pause-len", "4"])
        if r:
            res["pause"].append(r)
        r = run(bin_, cat, f"{out}/speaker_{s}", rng.uniform(0, 5), ["--speed", "1.002"])
        if r:
            res["drift+0.2%"].append(r)
    print("| scenario | n | locked | time-to-lock p50 / p90 s | pos err p50 / p95 ms | max err ms | CPU ms per s |")
    print("|---|---|---|---|---|---|---|")
    for k, v in res.items():
        lk = [r for r in v if r["ttl"] == r["ttl"]]
        ttl = sorted(r["ttl"] for r in lk)
        q = lambda a, f: a[min(len(a) - 1, int(f * len(a)))] if a else float("nan")
        print(f"| {k} | {len(v)} | {len(lk)}/{len(v)} | {q(ttl, .5):.2f} / {q(ttl, .9):.2f} | "
              f"{np.median([r['p50'] for r in lk]):.0f} / {np.median([r['p95'] for r in lk]):.0f} | "
              f"{np.median([r['max'] for r in lk]):.0f} | {np.median([r['cpu'] for r in v]):.2f} |"
              + (f" pause detected {sum(r['pause'] for r in v)}/{len(v)}" if k == "pause" else ""))
    # false locks on songs not in the catalog
    fl = 0
    negs = sorted(os.listdir(f"{neg}/audio"))
    for s in negs:
        r = run(bin_, cat, f"{neg}/audio/{s}", 0, ["--secs", "55"])
        if r and r["ttl"] == r["ttl"]:
            fl += 1
    print(f"\nfalse locks on {len(negs)} songs absent from the catalog (55 s each, continuous listening): {fl}")


if __name__ == "__main__":
    main()
