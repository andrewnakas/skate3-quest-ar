//! Describe every stream in a retail Skate 3 audio archive.
//!
//! The unit tests in `audio::tests` are synthetic and prove only self-consistency. This is the
//! program that runs the same code over real data, which in this project is what has caught
//! every format bug. Point it at your own copy of the game.
//!
//!   cargo run -p skate-data --example describe_audio -- <file.big|file.sns> [...]

use skate_data::audio::{self, Container};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: describe_audio <file.big|file.sns> [...]");
        std::process::exit(2);
    }
    let mut total = 0usize;
    let mut seconds = 0.0f64;
    for path in &args {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{path}: {e}");
                continue;
            }
        };
        match audio::describe_any(&data) {
            Ok(Container::Streams(streams)) => {
                println!("{path}: {} stream(s)", streams.len());
                for (name, info) in &streams {
                    println!(
                        "  {name:<40} {:?} {} Hz {}ch {} contexts {:.2}s",
                        info.codec,
                        info.sample_rate,
                        info.channels,
                        info.contexts,
                        info.duration_secs()
                    );
                    total += 1;
                    seconds += info.duration_secs();
                }
            }
            Ok(Container::SubSounds(members)) => {
                let subs: usize = members.iter().map(|(_, v)| v.len()).sum();
                println!(
                    "{path}: {} .sth member(s), {subs} sub-sound(s)",
                    members.len()
                );
                for (name, infos) in members.iter().take(3) {
                    let first = infos.first();
                    println!(
                        "  {name:<40} {} sub-sounds{}",
                        infos.len(),
                        match first {
                            Some(i) => format!(
                                " e.g. {:?} {} Hz {}ch {:.2}s",
                                i.codec,
                                i.sample_rate,
                                i.channels,
                                i.duration_secs()
                            ),
                            None => String::new(),
                        }
                    );
                }
                if members.len() > 3 {
                    println!("  ... {} more member(s)", members.len() - 3);
                }
                total += subs;
                seconds += members
                    .iter()
                    .flat_map(|(_, v)| v.iter())
                    .map(|i| i.duration_secs())
                    .sum::<f64>();
            }
            Ok(Container::Music {
                segments,
                num_samples,
                streams,
            }) => {
                let rate = streams.first().map_or(0, |s| s.sample_rate);
                let secs = if rate == 0 {
                    0.0
                } else {
                    num_samples as f64 / f64::from(rate)
                };
                println!("{path}: music, {segments} segment(s), {num_samples} samples, {secs:.1}s");
                for info in streams.iter().take(2) {
                    println!(
                        "  {:?} {} Hz {}ch {} contexts",
                        info.codec, info.sample_rate, info.channels, info.contexts
                    );
                }
                total += segments;
                seconds += secs;
            }
            Ok(Container::Payloads { names, .. }) => {
                println!(
                    "{path}: {} payload(s), headers in a paired archive (e.g. *resident.big)",
                    names.len()
                );
            }
            Ok(Container::Banks { kinds }) => {
                let listed: Vec<String> = kinds.iter().map(|(k, n)| format!("{n} .{k}")).collect();
                println!("{path}: banks, not decoded ({})", listed.join(", "));
            }
            Err(e) => println!("{path}: no audio ({e})"),
        }
    }
    println!("\n{total} stream(s), {seconds:.1}s of audio described");
}
