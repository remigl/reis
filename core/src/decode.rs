//! Reading audio files and their tags on a desktop machine (index building).
//!
//! WAV is decoded natively; every other format (MP3, FLAC, OGG, Opus, M4A,
//! AIFF, …) goes through `ffmpeg` if it is installed. Tags (title, artist,
//! album) come from `ffprobe`. Nothing here is needed on a phone.

use crate::json::Json;
use std::path::Path;
use std::process::{Command, Stdio};

pub const AUDIO_EXTS: &[&str] = &[
    "wav", "mp3", "flac", "ogg", "oga", "opus", "m4a", "aac", "aif", "aiff", "wma", "ape", "wv",
    "mka", "webm", "mp4",
];

pub fn is_audio(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Mono f32 at `rate`. `max_secs` > 0 truncates.
pub fn load(path: &Path, rate: u32, max_secs: f64) -> Result<Vec<f32>, String> {
    let is_wav = path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("wav"))
        .unwrap_or(false);
    if is_wav {
        if let Ok((x, sr)) = crate::audio::read_wav(path.to_str().unwrap_or("")) {
            let x = if max_secs > 0.0 {
                x[..x.len().min((max_secs * sr as f64) as usize)].to_vec()
            } else {
                x
            };
            return Ok(crate::audio::resample(&x, sr, rate));
        }
    }
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-nostdin", "-i"]).arg(path);
    if max_secs > 0.0 {
        cmd.args(["-t", &max_secs.to_string()]);
    }
    let out = cmd
        .args([
            "-vn",
            "-ac",
            "1",
            "-ar",
            &rate.to_string(),
            "-f",
            "f32le",
            "-",
        ])
        .stderr(Stdio::null())
        .output()
        .map_err(|e| {
            format!(
                "{}: cannot run ffmpeg ({e}); install ffmpeg or use WAV files",
                path.display()
            )
        })?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err(format!(
            "{}: ffmpeg could not decode the file",
            path.display()
        ));
    }
    Ok(out
        .stdout
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect())
}

/// Tags reported by ffprobe (lower-cased keys), empty if unavailable.
pub fn tags(path: &Path) -> Vec<(String, String)> {
    let Ok(out) = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_format"])
        .arg(path)
        .stderr(Stdio::null())
        .output()
    else {
        return vec![];
    };
    let Ok(j) = Json::parse(&String::from_utf8_lossy(&out.stdout)) else {
        return vec![];
    };
    j.get("format")
        .and_then(|f| f.get("tags"))
        .and_then(|t| t.obj())
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.str().map(|s| (k.to_lowercase(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Small square WebP thumbnail via ffmpeg; original bytes if ffmpeg is missing.
pub fn thumbnail(path: &Path, size: u32) -> Option<Vec<u8>> {
    let vf = format!("scale={size}:{size}:force_original_aspect_ratio=increase,crop={size}:{size}");
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &vf,
            "-c:v",
            "libwebp",
            "-quality",
            "60",
            "-f",
            "webp",
            "-",
        ])
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => Some(o.stdout),
        _ => std::fs::read(path).ok(),
    }
}
