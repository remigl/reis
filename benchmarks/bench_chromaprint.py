#!/usr/bin/env python3
"""Chromaprint baseline (the fingerprint AcoustID stores), same query set.

Fingerprints come from the real libchromaprint (through ffmpeg's `chromaprint`
muxer, algorithm 1 = AcoustID default). Matching is an *exhaustive* sliding
bit-error-rate search over every reference and every offset — an optimistic
upper bound for any Chromaprint index (AcoustID's server only does worse).

Usage: bench_chromaprint.py DB_DIR QUERY_TSV OUT_CSV
"""
import csv
import os
import subprocess
import sys
import time
from collections import defaultdict

import numpy as np

POP = np.array([bin(i).count("1") for i in range(256)], np.uint8)
HOP = 4096 / 3 / 11025  # seconds per sub-fingerprint


def fp(path):
    out = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-f", "chromaprint", "-fp_format", "raw", "-algorithm", "1", "-"],
                         capture_output=True, check=True).stdout
    return np.frombuffer(out, "<u4")


def main():
    db, qtsv, out = sys.argv[1:4]
    base = os.path.dirname(qtsv)
    meta = list(csv.DictReader(open(f"{db}/meta.tsv"), delimiter="\t"))
    t0 = time.time()
    refs = [fp(f"{db}/{m['file']}") for m in meta]
    print(f"ref fingerprints: {len(refs)} in {time.time() - t0:.1f}s, avg {np.mean([len(r) for r in refs]):.0f} ints "
          f"= {np.mean([len(r) for r in refs]) * 4 / 1000:.2f} KB raw / 60 s", file=sys.stderr)
    rows = list(csv.DictReader(open(qtsv), delimiter="\t"))
    res = []
    for k, r in enumerate(rows):
        t1 = time.time()
        q = fp(f"{base}/{r['file']}")
        t_fp = time.time() - t1
        n = len(q)
        best = []
        t2 = time.time()
        if n >= 1:
            qb = q.view(np.uint8)
            for si, ref in enumerate(refs):
                if len(ref) < n:
                    continue
                # all offsets at once: windows [len-n+1, n]
                w = np.lib.stride_tricks.sliding_window_view(ref, n)
                x = np.bitwise_xor(w, q[None, :])
                bits = POP[x.view(np.uint8)].reshape(x.shape[0], -1).sum(1)
                o = int(np.argmin(bits))
                best.append((bits[o] / (32 * n), si, o))
            best.sort()
        t_match = time.time() - t2
        if best:
            b1, s1, o1 = best[0]
            b2 = best[1][0] if len(best) > 1 else 0.5
        else:
            b1, s1, o1, b2 = 0.5, -1, 0, 0.5
        top5 = [s for _, s, _ in best[:5]]
        res.append(dict(file=r["file"], cond=r["cond"], dur=r["dur"], expected=r["expected"], true_off=r["true_offset"],
                        got=s1, got_off=o1 * HOP, ber=b1, ber2=b2, n=n, top5=int(int(r["expected"]) in top5),
                        fp_ms=t_fp * 1000, match_ms=t_match * 1000))
        if k % 400 == 0:
            print("query", k, file=sys.stderr)
    with open(out, "w") as f:
        w = csv.DictWriter(f, fieldnames=list(res[0]))
        w.writeheader()
        w.writerows(res)
    # calibrate: accept if ber <= T and ber2 - ber >= M, FP <= 0.5 %
    pos = [x for x in res if x["expected"] != "-1"]
    neg = [x for x in res if x["expected"] == "-1"]
    bestc = None
    for T in np.arange(0.20, 0.46, 0.01):
        for M in np.arange(0.0, 0.15, 0.01):
            acc = lambda x: x["n"] > 0 and x["ber"] <= T and x["ber2"] - x["ber"] >= M
            fpn = sum(acc(x) for x in neg) + sum(acc(x) and str(x["got"]) != x["expected"] for x in pos)
            ok = sum(acc(x) and str(x["got"]) == x["expected"] for x in pos)
            if fpn <= 0.005 * len(res) and (bestc is None or ok > bestc[0]):
                bestc = (ok, T, M, fpn)
    ok, T, M, fpn = bestc
    acc = lambda x: x["n"] > 0 and x["ber"] <= T and x["ber2"] - x["ber"] >= M
    print(f"best rule: BER<={T:.2f} and margin>={M:.2f} -> top-1 {100 * ok / len(pos):.1f}% FP events {fpn}/{len(res)}")
    print("| condition | dur | top-1 | top-5 | offset err p50 ms | match p50 ms |")
    print("|---|---|---|---|---|---|")
    g = defaultdict(list)
    for x in pos:
        g[(x["cond"], int(x["dur"]))].append(x)
    for k in sorted(g):
        v = g[k]
        okk = [x for x in v if acc(x) and str(x["got"]) == x["expected"]]
        err = sorted(abs(x["got_off"] - float(x["true_off"])) * 1000 for x in okk)
        print(f"| {k[0]} | {k[1]} | {100 * len(okk) / len(v):.1f}% | {100 * np.mean([x['top5'] for x in v]):.1f}% | "
              f"{err[len(err) // 2] if err else float('nan'):.0f} | {np.median([x['match_ms'] for x in v]):.1f} |")
    gd = defaultdict(list)
    for x in pos:
        gd[int(x["dur"])].append(acc(x) and str(x["got"]) == x["expected"])
    print("by duration: " + " ".join(f"{d}s:{100 * np.mean(v):.0f}%" for d, v in sorted(gd.items())))


if __name__ == "__main__":
    main()
