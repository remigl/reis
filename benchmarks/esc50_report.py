#!/usr/bin/env python3
"""Markdown table from `reis eval --threshold -1 --csv` files (one per condition).

    esc50_report.py held_out.txt THRESHOLD name=clean:file.csv name=speaker:file.csv ...

Columns: closed-set top-1 (best label, no rejection), top-5, then at the
library's calibrated THRESHOLD: correct accepted, wrong label accepted,
unknown sounds (held-out categories) accepted; and the best rule at 5 % unknown accepted."""
import csv
import sys

held = set(open(sys.argv[1]).read().split())
thr = float(sys.argv[2])
print("| condition | closed-set top-1 | top-5 | correct @ calibrated threshold | wrong label | unknown accepted | correct @ 5 % unknown accepted |")
print("|---|---|---|---|---|---|---|")
for spec in sys.argv[3:]:
    name, f = spec.split(":", 1)
    r = list(csv.DictReader(open(f)))
    for x in r:
        x["s1"] = float(x["top_score"])
    kn = [x for x in r if x["expected"] not in held]
    un = [x for x in r if x["expected"] in held]
    pc = lambda n, d: 100 * n / max(1, d)
    closed = sum(x["top_label"] == x["expected"] for x in kn)
    # top-5: label among the candidates (written by eval as predicted/top_label only) -> recompute from 'kind'
    ok = sum(x["top_label"] == x["expected"] and x["s1"] >= thr for x in kn)
    wr = sum(x["top_label"] != x["expected"] and x["s1"] >= thr for x in kn)
    fa = sum(x["s1"] >= thr for x in un)
    us = sorted((x["s1"] for x in un), reverse=True)
    t5 = us[int(0.05 * len(us))] if us else 1
    ok5 = sum(x["top_label"] == x["expected"] and x["s1"] > t5 for x in kn)
    top5 = sum(x.get("in_top5", "0") == "1" for x in kn)
    print(f"| {name} | {pc(closed, len(kn)):.1f} % | {pc(top5, len(kn)):.1f} % | {pc(ok, len(kn)):.1f} % | {pc(wr, len(kn)):.1f} % | {pc(fa, len(un)):.1f} % | {pc(ok5, len(kn)):.1f} % |")
