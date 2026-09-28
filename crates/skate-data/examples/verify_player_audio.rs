//! Decode every in-scope player bank from user-owned assets and verify declared PCM timing.
//! cargo run -p skate-data --example verify_player_audio -- <assets> [cache-directory]
use skate_data::audio::catalog::PlayerAudioCatalog;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let assets = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("need prepared assets directory")?,
    );
    let cache = std::env::args().nth(2).map(PathBuf::from);
    let start = std::time::Instant::now();
    let catalog = PlayerAudioCatalog::from_assets(&assets, cache.as_deref())?;
    let mut total = 0u64;
    for (bank, _) in &catalog.banks {
        let samples: Vec<_> = catalog.samples.iter().filter(|s| &s.bank == bank).collect();
        let frames: u64 = samples.iter().map(|s| s.pcm.source.frames() as u64).sum();
        total += frames;
        println!(
            "{bank}: {} samples, {} loops, {frames} frames",
            samples.len(),
            samples.iter().filter(|s| s.header.looping).count()
        );
    }
    println!(
        "{} projects, {} banks, {} samples / {} frames, {} cache hits in {:.2}s",
        catalog.projects.len(),
        catalog.banks.len(),
        catalog.samples.len(),
        total,
        catalog.cache_hits,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
