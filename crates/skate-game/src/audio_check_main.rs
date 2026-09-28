//! Play one of the game's audio streams through the engine's real audio path, and exit.
//!
//! The plugin that plays game audio lives inside a binary that needs a set-up asset pipeline to
//! boot, which makes the audio path awkward to exercise on a development checkout. This runs the
//! same plugin, the same Bevy audio source and the same mixer, with nothing else: no window, no
//! renderer, no assets. It is how the claim "this plays" gets checked rather than asserted.
//!
//!   SKATE_AUDIO_PLAY=<archive>:<entry>:<channels>:<rate>[:<blocks>] \
//!     skate3-audio-check [seconds]
//!
//!   skate3-audio-check --tone [seconds]
//!   skate3-audio-check --graph-probe [seconds]
//!
//! Or set `SKATE_AUDIO_BANK_SAMPLE=<audiofiles.big>|<bank.abk>|<sample>` to
//! audition an inline bank sample through the same source and mixer.
//!
//! It exits when the stream has finished, or after `seconds` (default 20) so it can never hang a
//! script. Recording the output device's monitor while this runs is what turns "the sink reports
//! playing" into evidence that samples reached the device.

mod skate_audio;

use std::time::{Duration, Instant};

use bevy::MinimalPlugins;
use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::asset::AssetPlugin;
use bevy::audio::{AudioPlugin, AudioSink};
use bevy::log::LogPlugin;
use bevy::prelude::*;

#[derive(Resource)]
struct Deadline {
    started: Instant,
    limit: Duration,
    /// Set once a sink has existed, so "the sink went away" means finished rather than not yet.
    played: bool,
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let tone = arguments
        .first()
        .is_some_and(|argument| argument == "--tone");
    let graph_probe = arguments
        .first()
        .is_some_and(|argument| argument == "--graph-probe");
    let seconds: u64 = arguments
        .iter()
        .find(|argument| argument.as_str() != "--tone" && argument.as_str() != "--graph-probe")
        .and_then(|argument| argument.parse().ok())
        .unwrap_or(20);
    let ambience = std::env::var("SKATE_AMBIENCE").ok();
    let bank_sample = std::env::var("SKATE_AUDIO_BANK_SAMPLE").ok();
    if tone {
        // The plugin owns the source so this follows the same route through Bevy as decoded PCM.
        unsafe { std::env::set_var("SKATE_AUDIO_TONE", "440") };
    }
    if graph_probe {
        unsafe { std::env::set_var("SKATE_AUDIO_GRAPH_PROBE", "1") };
    }
    if !tone
        && !graph_probe
        && std::env::var("SKATE_AUDIO_PLAY").is_err()
        && ambience.is_none()
        && bank_sample.is_none()
    {
        eprintln!("set SKATE_AUDIO_PLAY=<archive>:<entry>:<channels>:<rate>[:<blocks>]");
        eprintln!("or  SKATE_AMBIENCE=<resident.big>:<payload.big>:<place>  to resolve a bed");
        eprintln!("or  SKATE_AUDIO_BANK_SAMPLE=<audiofiles.big>|<bank.abk>|<sample>");
        eprintln!("or  run skate3-audio-check --tone  to test the Windows output device");
        std::process::exit(2);
    }
    // Resolving before the app starts keeps the check honest: a place with no bed should say so
    // and exit, not sit through the deadline looking like a playback failure.
    if let Some(spec) = &ambience {
        let (resident, payload, place) = match parse_ambience_spec(spec) {
            Ok(parts) => parts,
            _ => {
                eprintln!(
                    "SKATE_AMBIENCE needs <resident.big>:<payload.big>:<place> (or use | as the separator)"
                );
                std::process::exit(2);
            }
        };
        match skate_audio::resolve_ambience(
            std::path::Path::new(resident),
            std::path::Path::new(payload),
            place,
        ) {
            Ok(request) => {
                println!(
                    "ambience for {place:?}: {} in {} (looping={})",
                    request.entry, payload, request.looping
                );
                // Hand it to the plugin through the same env hook the rest of this uses.
                unsafe {
                    std::env::set_var(
                        "SKATE_AUDIO_PLAY",
                        format!(
                            "{payload}:{}:{}:{}:{}",
                            request.entry, request.channels, request.sample_rate, 40
                        ),
                    );
                }
            }
            Err(e) => {
                println!("no ambience for {place:?}: {e}");
                std::process::exit(3);
            }
        }
    }
    if !tone && !graph_probe && !skate_audio::decoder_available() {
        eprintln!("no decoder on PATH; set SKATE_FFMPEG");
        std::process::exit(2);
    }

    App::new()
        // A fixed tick keeps this from spinning a core while the mixer does the real work.
        .add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(16))))
        .add_plugins((
            LogPlugin::default(),
            AssetPlugin::default(),
            AudioPlugin::default(),
        ))
        .add_plugins(skate_audio::SkateAudioPlugin)
        .insert_resource(Deadline {
            started: Instant::now(),
            limit: Duration::from_secs(seconds),
            played: false,
        })
        .add_systems(Update, finish)
        .run();
}

/// Exit when the stream has played out, or when the deadline passes.
fn finish(
    mut deadline: ResMut<Deadline>,
    sinks: Query<&AudioSink>,
    mut exit: MessageWriter<AppExit>,
) {
    let live = sinks.iter().count();
    if live > 0 {
        deadline.played = true;
    } else if deadline.played {
        info!(
            "skate-audio-check: the stream finished after {:.1?}",
            deadline.started.elapsed()
        );
        exit.write(AppExit::Success);
        return;
    }
    if deadline.started.elapsed() >= deadline.limit {
        if deadline.played {
            info!("skate-audio-check: still playing at the deadline; stopping");
        } else {
            warn!(
                "skate-audio-check: nothing ever played within {:?}",
                deadline.limit
            );
        }
        exit.write(if deadline.played {
            AppExit::Success
        } else {
            AppExit::error()
        });
    }
}

/// Parse two archive paths and an ambience place without splitting Windows drive letters.
///
/// `|` is the unambiguous form for unusual paths. The colon form is kept for the documented
/// command line and recognises the `:C:\\` boundary between two absolute Windows paths.
fn parse_ambience_spec(spec: &str) -> Result<(&str, &str, &str), ()> {
    if let Some((resident, rest)) = spec.split_once('|') {
        let (payload, place) = rest.split_once('|').ok_or(())?;
        return (!resident.is_empty() && !payload.is_empty() && !place.is_empty())
            .then_some((resident, payload, place))
            .ok_or(());
    }

    let (archives, place) = spec.rsplit_once(':').ok_or(())?;
    let separator = archives
        .char_indices()
        .find_map(|(index, character)| {
            if character != ':' {
                return None;
            }
            let mut suffix = archives[index + character.len_utf8()..].chars();
            match (suffix.next(), suffix.next(), suffix.next()) {
                (Some(drive), Some(':'), Some('\\' | '/')) if drive.is_ascii_alphabetic() => {
                    Some(index)
                }
                _ => None,
            }
        })
        .or_else(|| archives.find(':'))
        .ok_or(())?;
    let (resident, payload_with_separator) = archives.split_at(separator);
    let payload = &payload_with_separator[1..];
    (!resident.is_empty() && !payload.is_empty() && !place.is_empty())
        .then_some((resident, payload, place))
        .ok_or(())
}

#[cfg(test)]
mod tests {
    use super::parse_ambience_spec;

    #[test]
    fn ambience_spec_preserves_both_windows_drive_letters() {
        assert_eq!(
            parse_ambience_spec(
                r"C:\\Skate\ambienceresident.big:C:\\Skate\ambience.big:univ_mt_low"
            ),
            Ok((
                r"C:\\Skate\ambienceresident.big",
                r"C:\\Skate\ambience.big",
                "univ_mt_low"
            )),
        );
    }
}
