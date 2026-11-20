//! cargo run --release -p reis-core --example recognize -- sons.reis extrait.wav
use reis_core::{Library, Recognizer, RecognizerConfig, Verdict};
use std::sync::Arc;

fn main() -> Result<(), String> {
    let a: Vec<String> = std::env::args().collect();
    let lib = Arc::new(Library::open(&a[1])?);
    let rec = Recognizer::new(lib.clone(), RecognizerConfig::default())?;
    let (pcm, rate) = reis_core::audio::read_wav(&a[2]).map_err(|e| e.to_string())?;
    let r = rec.recognize(&pcm, rate);
    match r.verdict.unwrap() {
        Verdict::Exact { ref_id, offset_s } => {
            println!("{} à {offset_s:.2} s", lib.refs[ref_id as usize].name)
        }
        Verdict::Similar { label, score } => println!("{label} ({score:.2})"),
        Verdict::Tag { label, prob } => println!("ressemble à {label} ({prob:.2})"),
        _ => println!("rien"),
    }
    Ok(())
}
