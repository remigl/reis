#!/usr/bin/env python3
"""Open-set analysis of `reis eval --threshold -1 --csv` outputs.

For each decision rule (absolute score, margin s1-s2, ratio) report, at the
operating point where <= FA of unknown sounds are accepted: correct accepts
and wrong-label accepts on known sounds. Usage: openset.py held_out.txt a.csv [b.csv ...]"""
import csv
import sys

held = set(open(sys.argv[1]).read().split())
FA = [0.05, 0.10]
for f in sys.argv[2:]:
    r = list(csv.DictReader(open(f)))
    for x in r:
        x["s1"], x["s2"] = float(x["top_score"]), float(x["second_score"])
    kn = [x for x in r if x["expected"] not in held]
    un = [x for x in r if x["expected"] in held]
    closed = sum(x["top_label"] == x["expected"] for x in kn) / len(kn)
    out = [f"{f}: closed-set top-1 {100 * closed:.1f}%"]
    rules = {"score": lambda x: x["s1"], "margin": lambda x: x["s1"] - x["s2"], "score+margin": lambda x: x["s1"] + 2 * (x["s1"] - x["s2"])}
    for name, fn in rules.items():
        for fa in FA:
            us = sorted((fn(x) for x in un), reverse=True)
            t = us[int(fa * len(us))] if us else 0
            ok = sum(fn(x) > t and x["top_label"] == x["expected"] for x in kn) / len(kn)
            wr = sum(fn(x) > t and x["top_label"] != x["expected"] for x in kn) / len(kn)
            out.append(f"   {name:<13} @FA {int(fa * 100):>2}%: correct {100 * ok:5.1f}%  wrong {100 * wr:4.1f}%  (t={t:.3f})")
    print("\n".join(out))
