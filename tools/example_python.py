#!/usr/bin/env python3
"""Minimal use of the Reis C ABI from Python (standard library only).

    python3 tools/example_python.py path/to/libreis_core.so library.reis sound.wav
"""
import array
import ctypes
import json
import sys
import wave

so, library, sound = sys.argv[1:4]
lib = ctypes.CDLL(so)
lib.reis_catalog_open.restype = ctypes.c_void_p
lib.reis_catalog_free.argtypes = [ctypes.c_void_p]
lib.reis_recognizer_new.restype = ctypes.c_void_p
lib.reis_recognizer_new.argtypes = [ctypes.c_void_p]
lib.reis_recognizer_free.argtypes = [ctypes.c_void_p]
lib.reis_recognize_i16.restype = ctypes.c_void_p
lib.reis_recognize_i16.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32]
lib.reis_string_free.argtypes = [ctypes.c_void_p]

cat = lib.reis_catalog_open(library.encode())
if not cat:
    sys.exit(f"cannot open {library}")
rec = lib.reis_recognizer_new(cat)

w = wave.open(sound)
assert w.getsampwidth() == 2, "16-bit PCM WAV expected"
pcm = array.array("h", w.readframes(w.getnframes()))
buf = (ctypes.c_int16 * len(pcm)).from_buffer(pcm)
p = lib.reis_recognize_i16(rec, buf, len(pcm) // w.getnchannels(), w.getnchannels(), w.getframerate())
result = json.loads(ctypes.string_at(p).decode())
lib.reis_string_free(p)
print(json.dumps(result["verdict"], ensure_ascii=False))

lib.reis_recognizer_free(rec)
lib.reis_catalog_free(cat)
