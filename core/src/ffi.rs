//! C ABI used by the Flutter app (dart:ffi) or any other host.
//! All strings are UTF-8 (ptr,len) borrowed from the memory-mapped catalog:
//! valid as long as the catalog handle is alive. Handles are opaque.

#![allow(clippy::not_unsafe_ptr_arg_deref)] // C ABI: callers pass handles obtained from this module

use crate::catalog::Catalog;
use crate::engine::{Engine, EngineConfig};
use std::ffi::{c_char, CStr};
use std::sync::Arc;

pub struct ReisCatalog(Arc<Catalog>);

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
    match Catalog::open(&p) {
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

#[no_mangle]
pub extern "C" fn reis_engine_new(c: *const ReisCatalog) -> *mut Engine {
    let cat = unsafe { &*c }.0.clone();
    Box::into_raw(Box::new(Engine::new(cat, EngineConfig::default())))
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
