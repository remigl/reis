#!/usr/bin/env python3
"""Turn the synthetic music corpus of gen_corpus.py into a Reis library folder
(audio + .lrc + .json metadata + .webp artwork side by side).
    music_layout.py DB_DIR OUT_DIR"""
import csv
import json
import os
import sys

db, out = sys.argv[1:3]
os.makedirs(f"{out}/musique", exist_ok=True)
for r in csv.DictReader(open(f"{db}/meta.tsv"), delimiter="\t"):
    stem = os.path.basename(r["file"])[:-4]
    d = f"{out}/musique/{stem}"
    for src, ext in ((f"{db}/{r['file']}", "wav"), (f"{db}/lyrics/{stem}.lrc", "lrc"), (f"{db}/art/{r['album']}.webp", "webp")):
        if os.path.exists(src) and not os.path.exists(f"{d}.{ext}"):
            os.symlink(os.path.abspath(src), f"{d}.{ext}")
    with open(f"{d}.json", "w") as f:
        json.dump({"name": f"{r['artist']} - {r['title']}", "title": r["title"], "artist": r["artist"], "album": r["album"]}, f, ensure_ascii=False)
