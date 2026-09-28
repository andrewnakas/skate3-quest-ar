//! What does each landing voice actually sound like, in level terms?
//!
//! A retail landing is five voices: the fixed impact (`0x447`), the 2×2 air-time ladder
//! (`0x35C`…`0x35F`), `sub_824B8D48`'s class voice, and two collision voices. Measured off a
//! playtest, retail's landings step **+4.8 dB (class 1) and +6.4 dB (class 2)** over class 0,
//! while this engine manages only +1.9 / +1.3. The MixMap send accounts for 3.0 dB of retail's
//! step, so the rest has to come from the sample content — which is what this measures.
//!
//! For each voice it resolves the Splice members the way the sink does, decodes each member's
//! stream, and reports the gain the sink would apply, the decoded peak, and the two combined.
//!
//!     cargo run --release -p skate-game --example landing_sample_levels -- [assets dir]

use skate_audio_core::authored::oneshot::Rand;
use skate_data::audio::splice::{LandingTuning, SpliceBanks, SpliceState};
use skate_data::collections::Collections;
use std::path::{Path, PathBuf};

const DEFAULT_ASSETS: &str = r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets";

fn db(v: f32) -> f32 {
    if v > 0.0 { 20.0 * v.log10() } else { -99.0 }
}

/// Decode one member's stream and return (peak, rms) in 0..1.
fn measure(bytes: &[u8], at: usize) -> Option<(f32, f32)> {
    let info = match skate_data::audio::describe(bytes, at) {
        Ok(info) => info,
        Err(e) => {
            eprintln!("      describe @{at:#x}: {e}");
            return None;
        }
    };
    let mut decoder = skate_data::audio::ffmpeg::FfmpegDecoder::for_stream(&info);
    let pcm = match skate_data::audio::decode_stream(bytes, at, &mut decoder) {
        Ok(pcm) => pcm,
        Err(e) => {
            eprintln!("      decode @{at:#x}: {e}");
            return None;
        }
    };
    if pcm.is_empty() {
        eprintln!("      decode @{at:#x}: empty");
        return None;
    }
    let mut peak = 0.0f32;
    let mut energy = 0.0f64;
    for s in &pcm {
        let v = f32::from(*s) / 32768.0;
        peak = peak.max(v.abs());
        energy += f64::from(v) * f64::from(v);
    }
    Some((peak, (energy / pcm.len() as f64).sqrt() as f32))
}

/// Every stream start in the bank, sorted. A bank's streams are packed end to end, so the block
/// walker needs the *next* start as its end — handed the whole bank it reads past this stream and
/// into the next one's bytes, where the block sizes are garbage.
fn stream_starts(banks: &SpliceBanks, bank: &str) -> Vec<usize> {
    let Some(splc) = banks.bank(bank) else {
        return Vec::new();
    };
    let mut starts: Vec<usize> = (0..u16::MAX)
        .map_while(|index| splc.stream_offset(index))
        .collect();
    starts.sort_unstable();
    starts.dedup();
    starts
}

/// Resolve a sample the way the sink does and report the loudest member.
fn voice(banks: &SpliceBanks, bank: &str, sample: u16, label: &str, starts: &[usize]) {
    let mut state = SpliceState::default();
    let mut rand = Rand::new(1);
    let members = match banks.resolve(bank, sample, &mut state, &mut rand) {
        Ok(m) => m,
        Err(e) => {
            println!("  {label:<22} sample {sample:#06x}  UNRESOLVED: {e}");
            return;
        }
    };
    let Some(raw) = banks.bank_bytes(bank) else {
        println!("  {label:<22} sample {sample:#06x}  bank {bank} not loaded");
        return;
    };
    // The audible level of the voice is the loudest member it opened.
    let mut best: Option<(f32, f32, f32)> = None;
    for m in &members {
        let at = m.stream_offset as usize;
        let end = starts
            .iter()
            .copied()
            .find(|&s| s > at)
            .unwrap_or(raw.len());
        let Some((peak, _rms)) = measure(&raw[..end], at) else {
            continue;
        };
        let effective = peak * m.values.gain;
        if best.is_none_or(|(b, _, _)| effective > b) {
            best = Some((effective, peak, m.values.gain));
        }
    }
    // The authored gain is the part that does not need a decoder: it is member gain × the
    // container value × CONTACT_TRIM, i.e. exactly what the sink hands `play_oneshot`. If retail's
    // class step is authored as level, it is visible here.
    let sum: f32 = members.iter().map(|m| m.values.gain).sum();
    let loudest = members.iter().fold(0.0f32, |a, m| a.max(m.values.gain));
    let decoded = match best {
        Some((eff, peak, _)) => format!("  peak {:+6.1} dBFS -> {:+6.1} dBFS", db(peak), db(eff)),
        None => String::from("  (not decoded)"),
    };
    // Whether the members *stack* is the question the sum cannot answer: a sum of 4.055 across six
    // members is only +12.2 dB if they overlap. `play_oneshot` holds a member for its `delay`
    // before opening it, so a spread of delays wider than the samples are long means the peak
    // never sees more than one of them.
    let delays: Vec<String> = members
        .iter()
        .map(|m| format!("{:.3}", m.values.delay))
        .collect();
    let widest = members.iter().fold(0.0f32, |a, m| a.max(m.values.delay));
    println!(
        "  {label:<22} sample {sample:#06x}  members {:<2}  loudest gain {loudest:.3} ({:+6.1} dB)  sum {sum:.3} ({:+6.1} dB){decoded}",
        members.len(),
        db(loudest),
        db(sum),
    );
    println!(
        "  {:<22} delays [{}] s, widest {widest:.3} s",
        "",
        delays.join(" ")
    );
}

fn main() {
    let assets = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ASSETS));
    let vault = Collections::load(&assets).expect("vault");
    let landing = LandingTuning::load(&vault).expect("landing tuning");
    let banks = SpliceBanks::load(
        &Path::new(&assets).join("private/stock/data/audio/audiofiles.big"),
        &[skate_data::audio::splice::COLLISIONS_BANK],
        &[skate_data::audio::splice::COLLISIONS_BANK],
    )
    .expect("banks");
    let bank = landing.bank();
    let starts = stream_starts(&banks, bank);

    println!("\nThe impact voice (fixed for every landing):");
    voice(&banks, bank, landing.sample, "impact", &starts);

    println!(
        "\nThe ladder voice (deck test x air >= {:.2} s):",
        landing.ladder_seconds
    );
    for test in 0..2 {
        for hard in 0..2 {
            let label = format!("test {test}, {}", if hard == 1 { "hard" } else { "soft" });
            voice(&banks, bank, landing.ladder[test][hard], &label, &starts);
        }
    }

    println!(
        "\nThe class voice (`sub_824BA3F0`, kind 0) -- this is where retail's step should be:"
    );
    if let Some(splc) = banks.bank(bank) {
        println!(
            "  bank {bank}: {} records, {} containers, {} groups",
            splc.records.len(),
            splc.containers.len(),
            splc.groups.len()
        );
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for (i, r) in splc.records.iter().enumerate() {
            if r.children == 0 {
                match runs.last_mut() {
                    Some(last) if last.1 + 1 == i => last.1 = i,
                    _ => runs.push((i, i)),
                }
            }
        }
        println!("  records with zero groups: {} runs", runs.len());
        for (a, b) in runs.iter().take(12) {
            println!("    {a:#06x}..={b:#06x} ({} records)", b - a + 1);
        }
    }
    if let Some(splc) = banks.bank(bank) {
        for id in [
            0x41bu16, 0x41c, 0x41d, 0x431, 0x432, 0x433, 0x426, 0x428, 0x43e,
        ] {
            let n = splc.records.len();
            let idx = usize::from(id);
            if idx < n {
                println!(
                    "  id {id:#06x} -> record {idx} children {}",
                    splc.records[idx].children
                );
            } else if idx - n < splc.containers.len() {
                let c = &splc.containers[idx - n];
                let kids: Vec<String> = c
                    .ids
                    .iter()
                    .map(|&k| {
                        let ki = usize::from(k);
                        if ki < n {
                            format!("{k:#06x}(rec,ch={})", splc.records[ki].children)
                        } else {
                            format!("{k:#06x}(CONTAINER {})", ki - n)
                        }
                    })
                    .collect();
                println!(
                    "  id {id:#06x} -> container {} mode {} kids [{}]",
                    idx - n,
                    c.mode,
                    kids.join(", ")
                );
            }
        }
    }
    // DIAGNOSTIC: a zero-group record still carries its five floats. If those carry a redirect
    // (to another record, or to another bank) that is what the empty class-2 rows really mean;
    // if they are the same shape as a normal record's, the records are authored-silent.
    if let Some(splc) = banks.bank(bank) {
        // DIAGNOSTIC: 12 of a record's 36 bytes are unparsed (+0..+3 and +28..+35). If the
        // container gain base/range live there rather than at +12/+16, `container_value` returns a
        // constant 1.0 and every landing comes out the same level -- which is what the ladder
        // measures (2.5 dB of landing-to-landing spread against retail's 8 dB).
        if let Some(raw) = banks.bank_bytes(bank) {
            use skate_audio_formats::splc::{HEADER_BYTES, RECORD_BYTES};
            let f = |at: usize| f32::from_be_bytes(raw[at..at + 4].try_into().unwrap());
            let u = |at: usize| u32::from_be_bytes(raw[at..at + 4].try_into().unwrap());
            println!("  raw record words (+0 +4 +8 +12 +16 +20 +24 +28 +32), floats beneath:");
            for i in [0x024eusize, 0x024f, 0x0298, 0x029b, 0x026e, 0x0277] {
                let at = HEADER_BYTES + RECORD_BYTES * i;
                let words: Vec<String> = (0..9).map(|k| format!("{:08x}", u(at + 4 * k))).collect();
                let floats: Vec<String> =
                    (0..9).map(|k| format!("{:>8.3}", f(at + 4 * k))).collect();
                println!("    rec {i:#06x}  {}", words.join(" "));
                println!("              {}", floats.join(" "));
            }
        }
        println!("  zero-group records vs their neighbours (id, children, the five fields):");
        let show = |i: usize| {
            let r = &splc.records[i];
            println!(
                "    rec {i:#06x} id {:#06x} ch {:2}  u8 {:12.5}  base {:12.5}  range {:12.5}  u20 {:12.5}  u24 {:12.5}",
                r.id,
                r.children,
                r.unknown_8,
                r.value_base,
                r.value_range,
                r.unknown_20,
                r.unknown_24
            );
        };
        for i in [0x0277usize, 0x0278, 0x0279, 0x02a0, 0x02a1, 0x02a2] {
            show(i);
        }
        println!("    -- normal ones for comparison --");
        for i in [0x026eusize, 0x029b, 0x024e, 0x0298] {
            show(i);
        }
    }
    // DIAGNOSTIC: every mode row, not just the one `class_sample` would pick, so a row that is
    // entirely unresolvable is distinguishable from a single bad id.
    for (mode, row) in landing.class_modes.iter().enumerate() {
        println!("  mode {mode} ids {:04x?}", row);
    }
    // DIAGNOSTIC: one draw is not the answer -- the container picks one record kid of several and
    // each group draws one member, so the resolved gain varies per landing. Average many draws to
    // see whether the *authored content* actually steps with the class, independently of the
    // 3.0 dB send that `landing_send_scale` applies on top.
    println!("  resolved gain over 400 draws (the content's own class step, send excluded):");
    for class in 0..3u32 {
        let mut sums = Vec::new();
        let mut counts = Vec::new();
        for seed in 1..401u32 {
            let Some(sample) = landing.class_sample(0, class, 0) else {
                continue;
            };
            let mut state = SpliceState::default();
            let mut rand = Rand::new(seed);
            if let Ok(members) = banks.resolve(bank, sample, &mut state, &mut rand) {
                sums.push(members.iter().map(|m| m.values.gain).sum::<f32>());
                counts.push(members.len() as f32);
            }
        }
        if sums.is_empty() {
            continue;
        }
        let mean = sums.iter().sum::<f32>() / sums.len() as f32;
        let members = counts.iter().sum::<f32>() / counts.len() as f32;
        sums.sort_by(f32::total_cmp);
        println!(
            "    class {class}: mean gain {mean:.3} ({:+5.1} dB)  min {:.3}  max {:.3}  mean members {members:.2}",
            db(mean),
            sums[0],
            sums[sums.len() - 1],
        );
    }
    for class in 0..3u32 {
        for category in 0..4u8 {
            let Some(sample) = landing.class_sample(0, class, category) else {
                println!("  class {class} cat {category}: no sample");
                continue;
            };
            voice(
                &banks,
                bank,
                sample,
                &format!("class {class}, cat {category}"),
                &starts,
            );
        }
    }
    println!();
}
