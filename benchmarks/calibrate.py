#!/usr/bin/env python3
"""Pick the acceptance rule (score >= A and score >= R*(second+1)) that maximises
correct accepts with false positives (negatives + wrong songs) <= MAX_FP.
Usage: calibrate.py bench.csv [MAX_FP=0.005]"""
import csv, sys
from collections import defaultdict
rows = list(csv.DictReader(open(sys.argv[1])))
maxfp = float(sys.argv[2]) if len(sys.argv) > 2 else 0.005
for r in rows:
    r['s'] = int(r['score']); r['s2'] = int(r['second'])
pos = [r for r in rows if r['expected'] != '-1']; neg = [r for r in rows if r['expected'] == '-1']
best = None
for A in range(4, 40):
    for R10 in range(10, 41):
        R = R10 / 10
        acc = lambda r: r['s'] >= A and r['s'] >= R * (r['s2'] + 1)
        fp = sum(acc(r) for r in neg) + sum(acc(r) and r['got'] != r['expected'] for r in pos)
        fpr = fp / (len(neg) + len(pos))
        ok = sum(acc(r) and r['got'] == r['expected'] for r in pos)
        if fpr <= maxfp and (best is None or ok > best[0]):
            best = (ok, A, R, fp)
ok, A, R, fp = best
print(f"best rule: score>={A} and score>={R}*(second+1)  -> top-1 {100*ok/len(pos):.1f}%  FP events {fp}/{len(neg)+len(pos)}")
acc = lambda r: r['s'] >= A and r['s'] >= R * (r['s2'] + 1)
g = defaultdict(lambda: [0, 0])
for r in pos:
    k = r['cond']; g[k][1] += 1; g[k][0] += acc(r) and r['got'] == r['expected']
print('  ' + ' '.join(f"{k}:{100*v[0]/v[1]:.0f}" for k, v in sorted(g.items())))
gd = defaultdict(lambda: [0, 0])
for r in pos:
    k = int(r['dur']); gd[k][1] += 1; gd[k][0] += acc(r) and r['got'] == r['expected']
print('  by duration: ' + ' '.join(f"{k}s:{100*v[0]/v[1]:.0f}" for k, v in sorted(gd.items())))
