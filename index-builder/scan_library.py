#!/usr/bin/env python3
"""Step 1 — scan an audio library and write the manifest `reis build` consumes.

    scan_library.py MUSIC_DIR OUT_DIR [--art-size 160] [--art-quality 60]

For each audio file: tags via ffprobe (title, artist, album, duration,
MusicBrainz recording id when present), embedded cover extracted by ffmpeg and
re-encoded to a small square WebP. Identical covers (same album) are written
once (content hash), so the catalog stores each image a single time.

Writes OUT_DIR/meta.tsv  (file, title, artist, album, duration, lrc, art, mbid)
       OUT_DIR/art/*.webp
Needs only Python 3 stdlib + ffmpeg/ffprobe. Nothing here ships with the app.
"""
import argparse
import hashlib
import json
import os
import subprocess
import sys

AUDIO = {".mp3", ".flac", ".ogg", ".opus", ".m4a", ".aac", ".wav", ".wma", ".alac", ".aiff", ".ape", ".wv"}


def probe(path):
    r = subprocess.run(["ffprobe", "-v", "error", "-print_format", "json", "-show_format", "-show_streams", path],
                       capture_output=True, text=True)
    if r.returncode:
        return None
    j = json.loads(r.stdout)
    tags = {k.lower(): v for k, v in j.get("format", {}).get("tags", {}).items()}
    for s in j.get("streams", []):
        tags.update({k.lower(): v for k, v in s.get("tags", {}).items() if k.lower() not in tags})
    has_cover = any(s.get("disposition", {}).get("attached_pic") for s in j.get("streams", []))
    return dict(
        title=tags.get("title") or os.path.splitext(os.path.basename(path))[0],
        artist=tags.get("artist") or tags.get("album_artist") or "",
        album=tags.get("album") or "",
        duration=float(j.get("format", {}).get("duration") or 0),
        mbid=tags.get("musicbrainz_trackid") or tags.get("musicbrainz recording id") or "",
        has_cover=has_cover,
    )


def cover(path, out_dir, size, q, folder_cache):
    src = None
    r = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-an", "-frames:v", "1", "-f", "image2pipe", "-c:v", "png", "-"],
                       capture_output=True)
    if r.returncode == 0 and r.stdout:
        src = r.stdout
    else:  # folder.jpg / cover.jpg next to the file
        d = os.path.dirname(path)
        if d in folder_cache:
            return folder_cache[d]
        for n in ("cover.jpg", "folder.jpg", "cover.png", "front.jpg", "Cover.jpg", "Folder.jpg"):
            if os.path.exists(os.path.join(d, n)):
                src = open(os.path.join(d, n), "rb").read()
                break
        if src is None:
            folder_cache[d] = ""
            return ""
    r = subprocess.run(["ffmpeg", "-v", "error", "-f", "image2pipe", "-i", "-",
                        "-vf", f"scale={size}:{size}:force_original_aspect_ratio=increase,crop={size}:{size}",
                        "-c:v", "libwebp", "-quality", str(q), "-f", "webp", "-"], input=src, capture_output=True)
    if r.returncode or not r.stdout:
        return ""
    h = hashlib.sha1(r.stdout).hexdigest()[:16]
    rel = f"art/{h}.webp"
    p = os.path.join(out_dir, rel)
    if not os.path.exists(p):
        with open(p, "wb") as f:
            f.write(r.stdout)
    folder_cache[os.path.dirname(path)] = rel
    return rel


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("music_dir")
    ap.add_argument("out_dir")
    ap.add_argument("--art-size", type=int, default=160)
    ap.add_argument("--art-quality", type=int, default=60)
    a = ap.parse_args()
    os.makedirs(os.path.join(a.out_dir, "art"), exist_ok=True)
    rows, cache = [], {}
    files = sorted(os.path.join(dp, f) for dp, _, fs in os.walk(a.music_dir) for f in fs if os.path.splitext(f)[1].lower() in AUDIO)
    for i, path in enumerate(files):
        m = probe(path)
        if not m or m["duration"] < 20:
            continue
        art = cover(path, a.out_dir, a.art_size, a.art_quality, cache)
        clean = lambda s: s.replace("\t", " ").replace("\n", " ").strip()
        rows.append([os.path.abspath(path), clean(m["title"]), clean(m["artist"]), clean(m["album"]), f"{m['duration']:.2f}", "", art, m["mbid"]])
        if i % 100 == 0:
            print(f"{i}/{len(files)}", file=sys.stderr)
    with open(os.path.join(a.out_dir, "meta.tsv"), "w", encoding="utf-8") as f:
        f.write("file\ttitle\tartist\talbum\tduration\tlrc\tart\tmbid\n")
        for r in rows:
            f.write("\t".join(r) + "\n")
    print(f"{len(rows)} tracks, {len(set(r[6] for r in rows if r[6]))} unique covers", file=sys.stderr)


if __name__ == "__main__":
    main()
