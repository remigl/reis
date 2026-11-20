//! Helpers shared by the CLI commands.

use reis_core::library::Library;
use std::path::Path;

pub fn arg(args: &[String], k: &str) -> Option<String> {
    args.iter()
        .position(|a| a == k)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
pub fn flag(args: &[String], k: &str) -> bool {
    args.iter().any(|a| a == k)
}
pub fn arg_parse<T: std::str::FromStr>(args: &[String], k: &str, default: T) -> T {
    arg(args, k).and_then(|v| v.parse().ok()).unwrap_or(default)
}
/// Positional arguments (everything that is not a `--flag value` pair).
pub fn positional(args: &[String], flags_with_value: &[&str]) -> Vec<String> {
    let mut out = vec![];
    let mut i = 0;
    while i < args.len() {
        if flags_with_value.contains(&args[i].as_str()) {
            i += 2;
            continue;
        }
        if !args[i].starts_with("--") {
            out.push(args[i].clone());
        }
        i += 1;
    }
    out
}

pub fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("reis: {msg}");
    std::process::exit(1)
}

pub fn open_lib(p: &str) -> Library {
    Library::open(p).unwrap_or_else(|e| die(e))
}

pub fn load_audio(path: &Path, rate: u32, max_secs: f64) -> std::io::Result<Vec<f32>> {
    reis_core::decode::load(path, rate, max_secs).map_err(std::io::Error::other)
}

pub fn mem_report() -> String {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let g = |k: &str| {
        s.lines()
            .find(|l| l.starts_with(k))
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("?")
            .to_string()
    };
    format!(
        "VmHWM {} KB (RssAnon {} KB heap, RssFile {} KB mmap page cache)",
        g("VmHWM"),
        g("RssAnon"),
        g("RssFile")
    )
}

pub fn peak_rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find(|l| l.starts_with("VmHWM")).map(|l| {
                l.split_whitespace()
                    .nth(1)
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0)
            })
        })
        .unwrap_or(0)
}

pub fn pct(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// Simple progress line on stderr.
pub fn progress(done: usize, total: usize, what: &str) {
    if total > 0 && (done == total || done % 20 == 0) {
        eprint!("\r  {what} {done}/{total}");
        if done == total {
            eprintln!();
        }
    }
}
