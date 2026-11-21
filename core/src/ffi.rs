//! C ABI used by the Flutter app (dart:ffi) or any other host.
//! All strings are UTF-8 (ptr,len) borrowed from the memory-mapped catalog:
//! valid as long as the catalog handle is alive. Handles are opaque.

#![allow(clippy::not_unsafe_ptr_arg_deref)] // C ABI: callers pass handles obtained from this module

use crate::engine::{Engine, EngineConfig};
use crate::library::Library;
use std::ffi::{c_char, CStr};
use std::sync::Arc;

pub struct ReisCatalog(Arc<Library>);

#[repr(C)]
pub struct ReisState {
    pub phase: u32, // 0 listening, 1 locked, 2 paused
    pub song: i32,  // -1 = none
    pub position_ms: f64,
    pub confidence: f32,
    pub wants_audio: u8,
    pub uncertain: u8,
    pub level_dbfs: f32,
    pub last_score: u32,
    pub last_match_us: u64,
}

#[no_mangle]
pub extern "C" fn reis_catalog_open(path: *const c_char) -> *mut ReisCatalog {
    if path.is_null() {
        return std::ptr::null_mut();
    }
    let p = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .to_string();
    match Library::open(&p) {
        Ok(c) => Box::into_raw(Box::new(ReisCatalog(Arc::new(c)))),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn reis_catalog_free(c: *mut ReisCatalog) {
    if !c.is_null() {
        unsafe { drop(Box::from_raw(c)) }
    }
}

#[no_mangle]
pub extern "C" fn reis_catalog_song_count(c: *const ReisCatalog) -> u32 {
    unsafe { &*c }.0.song_count()
}

fn put_str(s: &str, out_len: *mut u32) -> *const u8 {
    unsafe { *out_len = s.len() as u32 };
    s.as_ptr()
}

/// field: 0 title, 1 artist, 2 album
#[no_mangle]
pub extern "C" fn reis_song_str(
    c: *const ReisCatalog,
    song: u32,
    field: u32,
    out_len: *mut u32,
) -> *const u8 {
    let c = &unsafe { &*c }.0;
    if song >= c.song_count() {
        return put_str("", out_len);
    }
    put_str(
        match field {
            0 => c.title(song),
            1 => c.artist(song),
            _ => c.album(song),
        },
        out_len,
    )
}

#[no_mangle]
pub extern "C" fn reis_song_duration_ms(c: *const ReisCatalog, song: u32) -> u32 {
    let c = &unsafe { &*c }.0;
    if song >= c.song_count() {
        0
    } else {
        c.duration_ms(song)
    }
}

/// Artwork bytes (JPEG/WebP) or null.
#[no_mangle]
pub extern "C" fn reis_song_artwork(
    c: *const ReisCatalog,
    song: u32,
    out_len: *mut u32,
) -> *const u8 {
    let c = &unsafe { &*c }.0;
    match (song < c.song_count()).then(|| c.artwork(song)).flatten() {
        Some(b) => {
            unsafe { *out_len = b.len() as u32 };
            b.as_ptr()
        }
        None => {
            unsafe { *out_len = 0 };
            std::ptr::null()
        }
    }
}

#[no_mangle]
pub extern "C" fn reis_lyrics_count(c: *const ReisCatalog, song: u32) -> u32 {
    let c = &unsafe { &*c }.0;
    if song >= c.song_count() {
        return 0;
    }
    c.lyrics(song).map(|l| l.lines.len() as u32).unwrap_or(0)
}

/// Returns line text; writes its start time. Word timings via reis_lyrics_word.
#[no_mangle]
pub extern "C" fn reis_lyrics_line(
    c: *const ReisCatalog,
    song: u32,
    i: u32,
    out_t_ms: *mut u32,
    out_len: *mut u32,
) -> *const u8 {
    let c = &unsafe { &*c }.0;
    if let Some(l) = (song < c.song_count()).then(|| c.lyrics(song)).flatten() {
        if let Some(line) = l.lines.get(i as usize) {
            unsafe { *out_t_ms = line.0 };
            return put_str(line.1, out_len);
        }
    }
    unsafe { *out_t_ms = 0 };
    put_str("", out_len)
}

#[no_mangle]
pub extern "C" fn reis_lyrics_word_count(c: *const ReisCatalog, song: u32, i: u32) -> u32 {
    let c = &unsafe { &*c }.0;
    (song < c.song_count())
        .then(|| c.lyrics(song))
        .flatten()
        .and_then(|l| l.lines.get(i as usize).map(|x| x.2.len() as u32))
        .unwrap_or(0)
}

/// Word j of line i: start time and byte end offset inside the line text.
#[no_mangle]
pub extern "C" fn reis_lyrics_word(
    c: *const ReisCatalog,
    song: u32,
    i: u32,
    j: u32,
    out_t_ms: *mut u32,
    out_byte_end: *mut u32,
) -> u8 {
    let c = &unsafe { &*c }.0;
    if let Some(l) = (song < c.song_count()).then(|| c.lyrics(song)).flatten() {
        if let Some(w) = l.lines.get(i as usize).and_then(|x| x.2.get(j as usize)) {
            unsafe {
                *out_t_ms = w.t_ms;
                *out_byte_end = w.byte_end;
            }
            return 1;
        }
    }
    0
}

/// Active line index at position, -1 before the first line / no lyrics.
#[no_mangle]
pub extern "C" fn reis_lyrics_index_at(c: *const ReisCatalog, song: u32, pos_ms: u32) -> i32 {
    let c = &unsafe { &*c }.0;
    (song < c.song_count())
        .then(|| c.lyrics(song))
        .flatten()
        .and_then(|l| l.line_at(pos_ms))
        .map(|i| i as i32)
        .unwrap_or(-1)
}

/// Real-time engine (exact matching + position). Null if the library was
/// built without the exact engine.
#[no_mangle]
pub extern "C" fn reis_engine_new(c: *const ReisCatalog) -> *mut Engine {
    let cat = unsafe { &*c }.0.clone();
    if !cat.has_exact() {
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(Engine::new(cat, EngineConfig::default())))
}

/// 1 if the library contains the engine: which = 0 exact, 1 semantic.
#[no_mangle]
pub extern "C" fn reis_catalog_has(c: *const ReisCatalog, which: u32) -> u8 {
    let c = &unsafe { &*c }.0;
    (if which == 0 {
        c.has_exact()
    } else {
        c.has_semantic()
    }) as u8
}

#[no_mangle]
pub extern "C" fn reis_engine_free(e: *mut Engine) {
    if !e.is_null() {
        unsafe { drop(Box::from_raw(e)) }
    }
}

#[no_mangle]
pub extern "C" fn reis_engine_reset(e: *mut Engine) {
    unsafe { &mut *e }.reset()
}

/// Interleaved 16-bit PCM; channels are downmixed.
#[no_mangle]
pub extern "C" fn reis_engine_push_i16(
    e: *mut Engine,
    pcm: *const i16,
    frames: u32,
    channels: u32,
    rate: u32,
    clock_end_ms: f64,
) {
    let e = unsafe { &mut *e };
    let ch = channels.max(1) as usize;
    let s = unsafe { std::slice::from_raw_parts(pcm, frames as usize * ch) };
    let mono: Vec<f32> = s
        .chunks_exact(ch)
        .map(|f| f.iter().map(|&v| v as f32).sum::<f32>() / (ch as f32 * 32768.0))
        .collect();
    e.push_pcm(&mono, rate, clock_end_ms);
}

#[no_mangle]
pub extern "C" fn reis_engine_push_f32(
    e: *mut Engine,
    pcm: *const f32,
    frames: u32,
    rate: u32,
    clock_end_ms: f64,
) {
    let e = unsafe { &mut *e };
    let s = unsafe { std::slice::from_raw_parts(pcm, frames as usize) };
    e.push_pcm(s, rate, clock_end_ms);
}

#[no_mangle]
pub extern "C" fn reis_engine_tick(e: *mut Engine, now_ms: f64, out: *mut ReisState) {
    let st = unsafe { &mut *e }.tick(now_ms);
    if out.is_null() {
        return;
    }
    unsafe {
        *out = ReisState {
            phase: st.phase as u32,
            song: st.song.map(|s| s as i32).unwrap_or(-1),
            position_ms: st.position_ms,
            confidence: st.confidence,
            wants_audio: st.wants_audio as u8,
            uncertain: st.uncertain as u8,
            level_dbfs: st.level_dbfs,
            last_score: st.last_score,
            last_match_us: st.last_match_us,
        }
    }
}

// ------------------------------------------------------------------ generic API
// Any host (app, script, other language) can use the whole recogniser with
// four calls: open library -> new recognizer -> recognize PCM (JSON) -> free.

use crate::recognize::{Recognizer, RecognizerConfig};
use std::ffi::CString;

pub struct ReisRecognizer(Recognizer);

/// Label (category) of a reference, e.g. "animaux/chien".
#[no_mangle]
pub extern "C" fn reis_ref_label(c: *const ReisCatalog, id: u32, out_len: *mut u32) -> *const u8 {
    let c = &unsafe { &*c }.0;
    put_str(
        c.refs
            .get(id as usize)
            .map(|r| r.label.as_str())
            .unwrap_or(""),
        out_len,
    )
}

/// Any metadata value of a reference as text ("" if absent). `key` is UTF-8, NUL-terminated.
#[no_mangle]
pub extern "C" fn reis_ref_meta(
    c: *const ReisCatalog,
    id: u32,
    key: *const c_char,
    out_len: *mut u32,
) -> *const u8 {
    let c = &unsafe { &*c }.0;
    let k = unsafe { CStr::from_ptr(key) }.to_string_lossy();
    put_str(
        c.refs
            .get(id as usize)
            .and_then(|r| r.meta_str(&k))
            .unwrap_or(""),
        out_len,
    )
}

#[no_mangle]
pub extern "C" fn reis_recognizer_new(c: *const ReisCatalog) -> *mut ReisRecognizer {
    let lib = unsafe { &*c }.0.clone();
    match Recognizer::new(lib, RecognizerConfig::default()) {
        Ok(r) => Box::into_raw(Box::new(ReisRecognizer(r))),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn reis_recognizer_free(r: *mut ReisRecognizer) {
    if !r.is_null() {
        unsafe { drop(Box::from_raw(r)) }
    }
}

/// Recognize interleaved 16-bit PCM (channels are downmixed). Same result as reis_recognize_f32.
#[no_mangle]
pub extern "C" fn reis_recognize_i16(
    r: *const ReisRecognizer,
    pcm: *const i16,
    frames: u32,
    channels: u32,
    rate: u32,
) -> *mut c_char {
    let ch = channels.max(1) as usize;
    let s = unsafe { std::slice::from_raw_parts(pcm, frames as usize * ch) };
    let mono: Vec<f32> = s
        .chunks_exact(ch)
        .map(|f| f.iter().map(|&v| v as f32).sum::<f32>() / (ch as f32 * 32768.0))
        .collect();
    reis_recognize_f32(r, mono.as_ptr(), mono.len() as u32, rate)
}

/// Recognize mono f32 PCM. Returns a NUL-terminated JSON string (see
/// docs/API.md) to release with `reis_string_free`.
#[no_mangle]
pub extern "C" fn reis_recognize_f32(
    r: *const ReisRecognizer,
    pcm: *const f32,
    frames: u32,
    rate: u32,
) -> *mut c_char {
    let r = &unsafe { &*r }.0;
    let x = unsafe { std::slice::from_raw_parts(pcm, frames as usize) };
    let res = r.recognize(x, rate);
    CString::new(r.to_json(&res).to_string_compact())
        .unwrap_or_default()
        .into_raw()
}

#[no_mangle]
pub extern "C" fn reis_string_free(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)) }
    }
}

// ------------------------------------------------------------------ building on the device
// Lets an app create a library from audio it decoded itself (e.g. Android
// MediaCodec), without ffmpeg nor a desktop computer.
//
//   b = reis_builder_new("{\"engines\":[\"exact\"]}", model, model_len)
//   reis_builder_add_i16(b, pcm, frames, rate, "{\"name\":…,\"label\":…,\"meta\":{…},\"lyrics\":\"[00:01.00]…\"}", art, art_len)
//   reis_builder_finish(b, "/path/library.reis")   // also frees b

use crate::json::Json;
use crate::library::{LibraryBuilder, LibraryConfig};
use std::cell::RefCell;

thread_local! {
    static LAST_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

fn set_error(e: impl Into<String>) {
    LAST_ERROR.with(|l| *l.borrow_mut() = e.into());
}

/// Last error message of this thread (NUL-terminated, free with reis_string_free).
#[no_mangle]
pub extern "C" fn reis_last_error() -> *mut c_char {
    CString::new(LAST_ERROR.with(|l| l.borrow().clone())).unwrap_or_default().into_raw()
}

pub struct ReisBuilder(LibraryBuilder);

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().to_string()
    }
}

/// `config_json`: same keys as reis.json (engines, exact, semantic, augment…), may be NULL.
/// `model`: bytes of a .rnm file, required when the semantic engine is enabled.
#[no_mangle]
pub extern "C" fn reis_builder_new(config_json: *const c_char, model: *const u8, model_len: usize) -> *mut ReisBuilder {
    let mut cfg = LibraryConfig::default();
    let txt = cstr(config_json);
    if !txt.trim().is_empty() {
        match Json::parse(&txt).and_then(|j| cfg.apply_json(&j)) {
            Ok(()) => {}
            Err(e) => {
                set_error(e);
                return std::ptr::null_mut();
            }
        }
    }
    let m = (!model.is_null() && model_len > 0).then(|| unsafe { std::slice::from_raw_parts(model, model_len) }.to_vec());
    match LibraryBuilder::new(cfg, m) {
        Ok(b) => Box::into_raw(Box::new(ReisBuilder(b))),
        Err(e) => {
            set_error(e);
            std::ptr::null_mut()
        }
    }
}

fn add_prepared(b: &mut LibraryBuilder, prep: crate::library::Prepared, info_json: *const c_char, art: *const u8, art_len: usize) -> i32 {
    let info = Json::parse(&cstr(info_json)).unwrap_or(Json::Obj(vec![]));
    let name = info.get_str("name").unwrap_or("").to_string();
    let label = info.get_str("label").unwrap_or("").to_string();
    let file = info.get_str("file").unwrap_or("").to_string();
    let meta = info.get("meta").and_then(|m| m.obj()).map(|o| o.to_vec()).unwrap_or_default();
    let lyrics = info.get_str("lyrics").map(String::from);
    let art = (!art.is_null() && art_len > 0).then(|| unsafe { std::slice::from_raw_parts(art, art_len) }.to_vec());
    b.add_parts(&name, &label, &file, &meta, lyrics.as_deref(), art, prep) as i32
}

/// Add one reference from interleaved 16-bit PCM. `info_json`: {"name","label","file","meta":{…},"lyrics":"LRC text"}.
/// `art`: optional small image (JPEG/WebP/PNG). Returns the reference id, or -1.
#[no_mangle]
pub extern "C" fn reis_builder_add_i16(b: *mut ReisBuilder, pcm: *const i16, frames: u32, channels: u32, rate: u32, info_json: *const c_char, art: *const u8, art_len: usize) -> i32 {
    let b = &mut unsafe { &mut *b }.0;
    let ch = channels.max(1) as usize;
    if pcm.is_null() || frames == 0 || rate == 0 {
        set_error("empty audio");
        return -1;
    }
    let s = unsafe { std::slice::from_raw_parts(pcm, frames as usize * ch) };
    let mono: Vec<f32> = s.chunks_exact(ch).map(|f| f.iter().map(|&v| v as f32).sum::<f32>() / (ch as f32 * 32768.0)).collect();
    if mono.len() < rate as usize / 4 {
        set_error("shorter than 0.25 s");
        return -1;
    }
    let prep = b.prepare(&mono, rate);
    add_prepared(b, prep, info_json, art, art_len)
}

/// Add one reference from a file path (WAV natively; other formats need ffmpeg — desktop only).
#[no_mangle]
pub extern "C" fn reis_builder_add_file(b: *mut ReisBuilder, path: *const c_char, info_json: *const c_char, art: *const u8, art_len: usize) -> i32 {
    let b = &mut unsafe { &mut *b }.0;
    match b.prepare_file(std::path::Path::new(&cstr(path))) {
        Ok(prep) => add_prepared(b, prep, info_json, art, art_len),
        Err(e) => {
            set_error(e);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn reis_builder_count(b: *const ReisBuilder) -> u32 {
    unsafe { &*b }.0.len() as u32
}

/// Write the library (atomically: temp file + rename) and free the builder. 0 = ok.
#[no_mangle]
pub extern "C" fn reis_builder_finish(b: *mut ReisBuilder, out_path: *const c_char) -> i32 {
    let b = unsafe { Box::from_raw(b) }.0;
    if b.is_empty() {
        set_error("no reference was added");
        return -1;
    }
    let out = cstr(out_path);
    let tmp = format!("{out}.part");
    let bytes = b.finish();
    match std::fs::write(&tmp, &bytes).and_then(|_| std::fs::rename(&tmp, &out)) {
        Ok(()) => 0,
        Err(e) => {
            set_error(format!("{out}: {e}"));
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn reis_builder_free(b: *mut ReisBuilder) {
    if !b.is_null() {
        unsafe { drop(Box::from_raw(b)) }
    }
}
