//! cargo run --release -p reis-core --example build_library -- mes-sons/ models/mn10_as.rnm sons.reis
use reis_core::library::{scan_dir, LibraryBuilder, LibraryConfig};
use std::path::Path;

fn main() -> Result<(), String> {
    let a: Vec<String> = std::env::args().collect();
    let model = std::fs::read(&a[2]).ok();
    let mut b = LibraryBuilder::new(LibraryConfig::default(), model)?;
    for item in scan_dir(Path::new(&a[1]), false)? {
        let prepared = b.prepare_file(&item.path)?; // parallelisable: LibraryBuilder is Sync
        b.add(&item, prepared);
    }
    std::fs::write(&a[3], b.finish()).map_err(|e| e.to_string())?;
    Ok(())
}
