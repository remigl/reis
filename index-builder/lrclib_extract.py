#!/usr/bin/env python3
"""Step 2 — attach synced lyrics from a local LRCLIB SQLite dump.

    lrclib_extract.py LRCLIB_DUMP.sqlite3 OUT_DIR/meta.tsv [--tolerance 2]

Matches each manifest row to LRCLIB by normalised (artist, title) and duration
(± tolerance seconds), prefers synced lyrics, writes OUT_DIR/lyrics/<n>.lrc and
fills the manifest's `lrc` column. Runs fully offline against the dump.

LICENSING: LRCLIB's *code* is MIT, but the lyrics text belongs to the
songwriters/publishers. LRCLIB grants no redistribution licence for it. The
default use of this script is personal: build a catalog for your own device.
Do not publish catalogs containing lyrics you have no rights to.
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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dump")
    ap.add_argument("meta")
    ap.add_argument("--tolerance", type=float, default=2.0)
    a = ap.parse_args()
    con = sqlite3.connect(f"file:{a.dump}?mode=ro", uri=True)
    tc, lc = columns(con, "tracks"), columns(con, "lyrics")
    # LRCLIB schema: tracks(id, name_lower, artist_name_lower, duration, last_lyrics_id, ...),
    #                lyrics(id, synced_lyrics, plain_lyrics, track_id, ...)
    t_name = "name_lower" if "name_lower" in tc else "name"
    t_artist = "artist_name_lower" if "artist_name_lower" in tc else "artist_name"
    join = "l.id = t.last_lyrics_id" if "last_lyrics_id" in tc else "l.track_id = t.id"
    synced = "synced_lyrics" if "synced_lyrics" in lc else None
    if not synced:
        sys.exit("dump has no synced_lyrics column")
    has_index = any(t_name in (r[4] or "") for r in con.execute("SELECT * FROM sqlite_master WHERE type='index' AND tbl_name='tracks'"))
    if not has_index:
        print("note: no index on track names in this dump; lookups will be slow", file=sys.stderr)
    base = os.path.dirname(os.path.abspath(a.meta))
    os.makedirs(os.path.join(base, "lyrics"), exist_ok=True)
    lines = open(a.meta, encoding="utf-8").read().splitlines()
    hdr = lines[0].split("\t")
    ix = {h: i for i, h in enumerate(hdr)}
    out, found = [lines[0]], 0
    for n, row in enumerate(lines[1:]):
        f = row.split("\t")
        while len(f) < len(hdr):
            f.append("")
        title, artist, dur = f[ix["title"]], f[ix["artist"]], float(f[ix["duration"]] or 0)
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
            rel = f"lyrics/{n:06d}.lrc"
            with open(os.path.join(base, rel), "w", encoding="utf-8") as fo:
                fo.write(cands[0][0])
            f[ix["lrc"]] = rel
            found += 1
        out.append("\t".join(f))
    open(a.meta, "w", encoding="utf-8").write("\n".join(out) + "\n")
    print(f"synced lyrics found for {found}/{len(lines) - 1} tracks", file=sys.stderr)


if __name__ == "__main__":
    main()
