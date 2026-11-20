#!/usr/bin/env python3
"""Add synced lyrics (.lrc next to each audio file) from a local LRCLIB dump.

    lrclib_extract.py LRCLIB_DUMP.sqlite3 MUSIC_DIR [--tolerance 2] [--overwrite]

For every audio file in MUSIC_DIR (recursively), artist / title / duration are
read with ffprobe, matched against LRCLIB by normalised artist + title and
duration (± tolerance s), and the synced lyrics are written as
`<same name>.lrc` — the sidecar `reis index` picks up automatically.
Runs fully offline against the dump. Existing .lrc files are kept unless
--overwrite.

LICENSING: LRCLIB's *code* is MIT, but the lyrics text belongs to the
songwriters/publishers; LRCLIB grants no redistribution licence for it. Build
libraries with lyrics for your own devices; do not publish them.
"""
import argparse
import os
import re
import sqlite3
import sys
import unicodedata


def norm(s):
    s = unicodedata.normalize("NFKD", s or "").encode("ascii", "ignore").decode().lower()
    s = re.sub(r"\(.*?\)|\[.*?\]", " ", s)            # (feat. x), [remastered]
    s = re.sub(r"\b(feat|ft|featuring)\b.*", " ", s)
    s = re.sub(r"[^a-z0-9]+", " ", s)
    return " ".join(s.split())


def columns(con, table):
    return [r[1] for r in con.execute(f"PRAGMA table_info({table})")]


AUDIO = {".mp3", ".flac", ".ogg", ".opus", ".m4a", ".aac", ".wav", ".wma", ".aiff", ".aif", ".wv", ".ape"}


def probe(path):
    import json
    import subprocess
    r = subprocess.run(["ffprobe", "-v", "error", "-print_format", "json", "-show_format", path], capture_output=True, text=True)
    if r.returncode:
        return None
    f = json.loads(r.stdout).get("format", {})
    t = {k.lower(): v for k, v in f.get("tags", {}).items()}
    return t.get("artist") or t.get("album_artist") or "", t.get("title") or "", float(f.get("duration") or 0)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dump")
    ap.add_argument("music_dir")
    ap.add_argument("--tolerance", type=float, default=2.0)
    ap.add_argument("--overwrite", action="store_true")
    a = ap.parse_args()
    con = sqlite3.connect(f"file:{a.dump}?mode=ro", uri=True)
    tc, lc = columns(con, "tracks"), columns(con, "lyrics")
    t_name = "name_lower" if "name_lower" in tc else "name"
    t_artist = "artist_name_lower" if "artist_name_lower" in tc else "artist_name"
    join = "l.id = t.last_lyrics_id" if "last_lyrics_id" in tc else "l.track_id = t.id"
    if "synced_lyrics" not in lc:
        sys.exit("dump has no synced_lyrics column")
    synced = "synced_lyrics"
    files = sorted(os.path.join(dp, f) for dp, _, fs in os.walk(a.music_dir) for f in fs if os.path.splitext(f)[1].lower() in AUDIO)
    found = 0
    for path in files:
        out = os.path.splitext(path)[0] + ".lrc"
        if os.path.exists(out) and not a.overwrite:
            continue
        p = probe(path)
        if not p or not p[1]:
            continue
        artist, title, dur = p
        q = (f"SELECT l.{synced}, t.duration FROM tracks t JOIN lyrics l ON {join} "
             f"WHERE t.{t_name} = ? AND t.{t_artist} = ? AND l.{synced} IS NOT NULL AND l.{synced} != ''")
        cands = con.execute(q, (title.lower(), artist.lower())).fetchall()
        if not cands:  # looser: normalised title, artist prefix
            cands = [c for c in con.execute(
                f"SELECT l.{synced}, t.duration, t.{t_name}, t.{t_artist} FROM tracks t JOIN lyrics l ON {join} "
                f"WHERE t.{t_artist} LIKE ? AND l.{synced} IS NOT NULL", (artist.lower()[:20] + "%",)).fetchall()
                     if norm(c[2]) == norm(title) and norm(c[3]).startswith(norm(artist)[:10])]
        cands = [c for c in cands if dur == 0 or abs((c[1] or 0) - dur) <= a.tolerance]
        if cands:
            cands.sort(key=lambda c: abs((c[1] or 0) - dur))
            with open(out, "w", encoding="utf-8") as fo:
                fo.write(cands[0][0])
            found += 1
    print(f"synced lyrics written for {found}/{len(files)} files", file=sys.stderr)


if __name__ == "__main__":
    main()
