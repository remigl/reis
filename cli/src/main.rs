//! `reis` — build a fingerprint library from reference sounds, then recognize them.

mod cmds;
mod research;
mod util;

const USAGE: &str = "reis — offline sound recognition (music, animals, any sound)

Everyday use
  reis index DIR -o lib.reis      build a library from a folder of reference sounds
        [--engines exact,semantic] [--model mn10_as.rnm] [--tags] [--augment speaker] [--window 3] [--hop 1.5]
        [--max-windows 32] [--max-secs N] [--false-accept 0.05] [--threads N]
  reis recognize lib.reis FILE... recognize files       [--json] [--segments S] [--top N]
        [--engines exact,semantic,tags] [--threshold T] [--start S] [--duration D]
  reis listen lib.reis            live, raw s16le mono PCM on stdin   [--rate 16000] [--every 1]
  reis tag FILE...                general sound tags (527 AudioSet classes), no library needed
  reis eval lib.reis DIR          accuracy on a labelled test folder  [--csv out.csv] [--per-label]
  reis info lib.reis [--refs]     content, sizes, calibration
  reis lyrics lib.reis ID         synced lyrics of a reference

Research (exact engine / neural runtime)
  reis bench lib.reis queries.tsv [--csv out.csv]       landmark benchmark (see benchmarks/)
  reis simulate lib.reis song.wav --start S [...]       real-time engine simulation
  reis scale lib.reis --copies N [--out big.reis]       index scaling test
  reis diag lib.reis query.wav ref.wav offset           peak repeatability
  reis model-check model.rnm model.ref.bin              Rust runtime vs PyTorch reference

Documentation: docs/README.md";

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.is_empty() || a[0] == "-h" || a[0] == "--help" || a[0] == "help" {
        println!("{USAGE}");
        return;
    }
    let rest = &a[1..];
    let need = |n: usize| {
        if rest.len() < n {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    match a[0].as_str() {
        "index" => {
            need(1);
            cmds::cmd_index(rest)
        }
        "recognize" | "rec" => cmds::cmd_recognize(rest),
        "listen" => {
            need(1);
            cmds::cmd_listen(rest)
        }
        "tag" => {
            need(1);
            cmds::cmd_tag(rest)
        }
        "eval" => cmds::cmd_eval(rest),
        "info" => {
            need(1);
            cmds::cmd_info(rest)
        }
        "lyrics" => {
            need(2);
            cmds::cmd_lyrics(rest)
        }
        "bench" => {
            need(2);
            research::cmd_bench(rest)
        }
        "simulate" => {
            need(2);
            research::cmd_simulate(rest)
        }
        "scale" => {
            need(1);
            research::cmd_scale(rest)
        }
        "diag" => {
            need(4);
            research::cmd_diag(rest)
        }
        "model-check" => {
            need(2);
            research::cmd_model_check(rest)
        }
        "--version" | "version" => println!("reis {}", env!("CARGO_PKG_VERSION")),
        other => {
            eprintln!("unknown command '{other}'\n\n{USAGE}");
            std::process::exit(2)
        }
    }
}
