//! Gameplay packet words, transliterated from the retail updaters in single precision (`fdivs`,
//! `fmuls` round to f32; `fctiwz` truncates toward zero). Each is checked frame by frame against the
//! retail recomp capture with `tools/audio_capture_verify.py`; the tests below pin rows from it,
//! including ones where double-precision arithmetic gives a different word.

/// Image constants shared by the updaters.
pub(crate) const KMH_PER_MS: f32 = 3.6; // 0x822F8628
pub(crate) const TEN_THOUSAND: f32 = 10_000.0; // 0x821161A0
pub(crate) const THOUSAND: f32 = 1_000.0; // 0x82256FE8
pub(crate) const HALF: f32 = 0.5; // 0x8209975C
pub(crate) const SPEED_SCALE: f32 = 0.08; // 0x8208EDA4

/// PPC `fctiwz`: truncate toward zero, saturating; NaN gives `0x80000000`.
pub(crate) fn fctiwz(value: f32) -> i32 {
    if value.is_nan() {
        i32::MIN
    } else {
        value as i32
    }
}

/// `fsel`-style clamp to `0.0..=1.0` as the updaters write it.
pub(crate) fn unit(value: f32) -> f32 {
    if value < 0.0 {
        0.0
    } else if value > 1.0 {
        1.0
    } else {
        value
    }
}

/// Integer clamp into a packet word, as the updaters' `cmpwi`/`li` pairs do.
pub(crate) fn word(value: i32, low: i32, high: i32) -> u32 {
    value.clamp(low, high) as u32
}

/// `sub_824C9948` / `sub_824C4C18` rolling speed word:
/// `fctiwz(clamp(v / maxKmh × 3.6, 0, 1) × 10000)`, clamped 0..10000. `max_kmh` is the vault
/// float array `0x880C82E8EF647EC4` entry for the layer (70 for layers 0 and 3).
/// Capture: 37,086 of 37,090 updates exact (the rest are double-precision artefacts).
pub(crate) fn rolling_speed(ground_speed: f32, max_kmh: f32) -> u32 {
    word(
        fctiwz(unit(ground_speed / max_kmh * KMH_PER_MS) * TEN_THOUSAND),
        0,
        10_000,
    )
}

/// `sub_824C6198` rattle speed word: `fctiwz(clamp((v − 1) × 3.6 / D, 0, 1) × 10000)`, D = vault
/// `0x12275AA8AC4A63FB` (30 km/h).
pub(crate) fn rattle_speed(ground_speed: f32, divisor_kmh: f32) -> u32 {
    word(
        fctiwz(unit((ground_speed - 1.0) * KMH_PER_MS / divisor_kmh) * TEN_THOUSAND),
        0,
        10_000,
    )
}

/// `sub_824C28B0` grind speed: `min(fctiwz(clamp(3.6 × (v − 0.5) / T, 0, 1) × 10000), 9000)`,
/// T = vault `0x4890392C91829954` (45 km/h). Recomputed only while grinding (the component keeps
/// the last value afterwards).
pub(crate) fn grind_speed(ground_speed: f32, top_kmh: f32) -> u32 {
    word(
        fctiwz(unit(KMH_PER_MS * (ground_speed - HALF) / top_kmh) * TEN_THOUSAND),
        0,
        9_000,
    )
}

/// Skid w7 (`sub_824AF678` / `sub_824C7A20`): `fctiwz(clamp(v × 0.08, 0, 1) × 10000)`.
pub(crate) fn skid_speed(ground_speed: f32) -> u32 {
    word(
        fctiwz(unit(ground_speed * SPEED_SCALE) * TEN_THOUSAND),
        0,
        10_000,
    )
}

/// Squeaks w7 (`sub_824AFF48` / `sub_824C7DD0`): `fctiwz(clamp(v × 0.08, 0, 1) × 1000)`.
pub(crate) fn squeak_speed(ground_speed: f32) -> u32 {
    word(
        fctiwz(unit(ground_speed * SPEED_SCALE) * THOUSAND),
        0,
        1_000,
    )
}

/// Seams w8 (`sub_824C1F18`): `fctiwz(clamp((v − 0.5) × 0.08, 0, 1) × 10000)`.
pub(crate) fn seam_speed(ground_speed: f32) -> u32 {
    word(
        fctiwz(unit((ground_speed - HALF) * SPEED_SCALE) * TEN_THOUSAND),
        0,
        10_000,
    )
}

/// SenseOfSpeed intensity (`sub_824E7980` / `sub_824E7CB0`):
/// `fctiwz(clamp((v × 3.6 − low) / (high − low), 0, 1) × 1000)`. Rattle: board ground speed,
/// 30..80 km/h (vault `F57A74AFD22AD030` / `12275AA8AC4A63FB`); capture 3,313/3,313 exact.
/// Wind: COM speed, 15..55 km/h, or the bail pair while bailing.
pub(crate) fn speed_intensity(speed: f32, low_kmh: f32, high_kmh: f32) -> u32 {
    word(
        fctiwz(unit((speed * KMH_PER_MS - low_kmh) / (high_kmh - low_kmh)) * THOUSAND),
        0,
        1_000,
    )
}

/// Class_Flips w7..w9 (`sub_824CC7D8`): a dead-zoned rotation rate,
/// `v = min(fctiwz(|a| / D × 1000), 1000); v ≥ T ? v : 0`. Capture: 729/729 exact for each lane.
pub(crate) fn flip_rate(rate: f32, divisor: f32, threshold: i32) -> u32 {
    let value = fctiwz(rate.abs() / divisor * THOUSAND).min(1_000);
    if value >= threshold {
        value.max(0) as u32
    } else {
        0
    }
}

/// Treatment w7/w8 (`sub_824DD6F0`): `clamp(fctiwz(t × 1000), 0, 10000)` of KnownAir time in
/// state (+236) and time until landing (+240).
pub(crate) fn air_milliseconds(seconds: f32) -> u32 {
    word(fctiwz(seconds * THOUSAND), 0, 10_000)
}

/// Treatment w9: `fctiwz(clamp(h × 166.667, 0, 1000))` of the jump height (+260); 166.667 is
/// `0x822F9408`. Capture: 18,545/18,545 exact.
pub(crate) fn jump_height_word(height: f32) -> u32 {
    const SCALE: f32 = 166.667; // 0x822F9408
    let value = height * SCALE;
    fctiwz(if value < 0.0 {
        0.0
    } else if value > 1_000.0 {
        1_000.0
    } else {
        value
    })
    .max(0) as u32
}

/// Treatment/Flips w10: `clamp(fctiwz(timeScale × 500), 0, 1000)` (500 is `0x820BD5C4`).
pub(crate) fn time_scale_word(time_scale: f32) -> u32 {
    word(fctiwz(time_scale * 500.0), 0, 1_000)
}

/// Foot drag w7 (`sub_824AF498` / `sub_824BEEE8`): `fctiwz(clamp((v − 0.5) / 50 × 3.6, 0, 1) ×
/// 10000)`, 0.5 = vault `E5A6D8AC6EB9B5AB`, 50 = vault `B2C81577820408BE`.
pub(crate) fn foot_drag_speed(ground_speed: f32, offset: f32, top_kmh: f32) -> u32 {
    word(
        fctiwz(unit((ground_speed - offset) / top_kmh * KMH_PER_MS) * TEN_THOUSAND),
        0,
        10_000,
    )
}

/// Loose-board scrape w3 (`sub_824CB4C0`): `fctiwz(clamp((v − 0.5) / 15 × 3.6, 0, 1) × 10000)`,
/// 15 = vault `9635B780C7472A6E`.
pub(crate) fn board_slide_speed(ground_speed: f32, top_kmh: f32) -> u32 {
    word(
        fctiwz(unit((ground_speed - HALF) / top_kmh * KMH_PER_MS) * TEN_THOUSAND),
        0,
        10_000,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Rows from the retail recomp capture (2026-09-18): the audio state one frame earlier, and
    // the word retail redelivered.

    #[test]
    fn rolling_speed_matches_retail_where_double_precision_does_not() {
        // Frame 6588 (state 6587): v = 0x41182A19, retail 4891; double arithmetic gives 4890.
        assert_eq!(rolling_speed(f32::from_bits(0x4118_2A19), 70.0), 4_891);
        // Frame 16736 (state 16735): v = 0x411BF5C2, retail 5013.
        assert_eq!(rolling_speed(f32::from_bits(0x411B_F5C2), 70.0), 5_013);
        assert_eq!(rolling_speed(0.0, 70.0), 0);
        assert_eq!(rolling_speed(100.0, 70.0), 10_000);
    }

    #[test]
    fn treatment_air_words_match_retail_single_precision() {
        // Frame 5254 (state 5253): time in air 0x3F333333 (0.7 s) -> retail 700, double 699.
        assert_eq!(air_milliseconds(f32::from_bits(0x3F33_3333)), 700);
        assert_eq!(air_milliseconds(-1.0), 0);
        assert_eq!(air_milliseconds(20.0), 10_000);
        assert_eq!(jump_height_word(10.0), 1_000);
        assert_eq!(time_scale_word(1.0), 500);
    }

    #[test]
    fn flip_rates_dead_zone_below_the_threshold() {
        assert_eq!(flip_rate(3.0, 3.0, 297), 1_000);
        // 0.9f / 3.0f × 1000 is 299.99997 in single precision.
        assert_eq!(flip_rate(-0.9, 3.0, 297), 299);
        assert_eq!(flip_rate(0.8, 3.0, 297), 0);
        assert_eq!(flip_rate(100.0, 10.2, 603), 1_000);
    }

    #[test]
    fn grind_speed_caps_at_nine_thousand() {
        assert_eq!(grind_speed(0.5, 45.0), 0);
        assert_eq!(grind_speed(50.0, 45.0), 9_000);
    }

    #[test]
    fn fctiwz_truncates_toward_zero() {
        assert_eq!(fctiwz(-0.9), 0);
        assert_eq!(fctiwz(2.99), 2);
        assert_eq!(fctiwz(f32::NAN), i32::MIN);
    }
}
