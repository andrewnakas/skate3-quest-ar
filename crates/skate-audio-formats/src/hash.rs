//! The two name hashes Skate 3's audio data is keyed by.
//!
//! Both were read out of the shipping code rather than guessed, and both are checked
//! against real files by `examples/verify_banks.rs`:
//!
//! * [`member_hash`] is the key in an EB archive's entry table. It is plain djb2.
//! * [`name_id`] is the engine-wide 64-bit "hashed id". It is Bob Jenkins' `lookup8`
//!   `hash64` with the initial level baked in as `0xABCDEF00_11223344`, lifted from
//!   `sub_82B73AC0` in `generated/skate3_recomp.70.cpp` instruction by instruction.
//!
//! `name_id` is what an `.ems` emitter record stores to say which sound to play, and
//! what the game's own code carries as `lis`/`ori`/`rldimi` constants. Nothing here
//! canonicalises case: both hashes are byte-exact on the name as spelled.

/// djb2: `h = h * 33 + c`, seeded 5381. The EB entry table's `name_hash` column.
///
/// Exact on every member of `grains.big`, `ambience.big`, `ambienceresident.big`,
/// `post.big` and `wheels.big`. `audiofiles.big` is the exception: its column holds
/// small ascending values that are not this hash of the member name, and what they
/// are is not established.
pub fn member_hash(name: &[u8]) -> u32 {
    let mut h: u32 = 5381;
    for &b in name {
        h = h.wrapping_mul(33).wrapping_add(u32::from(b));
    }
    h
}

const LEVEL: u64 = 0xABCD_EF00_1122_3344;
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C13;

/// Jenkins `lookup8` mix, three 64-bit words.
fn mix(mut a: u64, mut b: u64, mut c: u64) -> (u64, u64, u64) {
    a = a.wrapping_sub(b).wrapping_sub(c);
    a ^= c >> 43;
    b = b.wrapping_sub(c).wrapping_sub(a);
    b ^= a << 9;
    c = c.wrapping_sub(a).wrapping_sub(b);
    c ^= b >> 8;
    a = a.wrapping_sub(b).wrapping_sub(c);
    a ^= c >> 38;
    b = b.wrapping_sub(c).wrapping_sub(a);
    b ^= a << 23;
    c = c.wrapping_sub(a).wrapping_sub(b);
    c ^= b >> 5;
    a = a.wrapping_sub(b).wrapping_sub(c);
    a ^= c >> 35;
    b = b.wrapping_sub(c).wrapping_sub(a);
    b ^= a << 49;
    c = c.wrapping_sub(a).wrapping_sub(b);
    c ^= b >> 11;
    a = a.wrapping_sub(b).wrapping_sub(c);
    a ^= c >> 12;
    b = b.wrapping_sub(c).wrapping_sub(a);
    b ^= a << 18;
    c = c.wrapping_sub(a).wrapping_sub(b);
    c ^= b >> 22;
    (a, b, c)
}

/// Little-endian assembly of up to eight bytes, as `lookup8` does it.
fn word(k: &[u8]) -> u64 {
    let mut v = 0u64;
    for (i, &b) in k.iter().take(8).enumerate() {
        v |= u64::from(b) << (8 * i);
    }
    v
}

/// The engine's 64-bit name id: Jenkins `lookup8` seeded with the game's level constant.
///
/// This is the value an `.ems` record carries at `+0x30` and the value the game's own
/// code assembles with `lis`/`ori`/`rldimi` before asking the object registry for a name.
pub fn name_id(name: &[u8]) -> u64 {
    let (mut a, mut b, mut c) = (LEVEL, LEVEL, GOLDEN);
    let mut k = name;
    while k.len() >= 24 {
        a = a.wrapping_add(word(&k[0..8]));
        b = b.wrapping_add(word(&k[8..16]));
        c = c.wrapping_add(word(&k[16..24]));
        (a, b, c) = mix(a, b, c);
        k = &k[24..];
    }
    c = c.wrapping_add(name.len() as u32 as u64);
    // The low byte of `c` is reserved for the length, so byte 16 lands at bit 8.
    for (i, &byte) in k.iter().enumerate() {
        let v = u64::from(byte);
        match i {
            0..=7 => a = a.wrapping_add(v << (8 * i)),
            8..=15 => b = b.wrapping_add(v << (8 * (i - 8))),
            _ => c = c.wrapping_add(v << (8 * (i - 15))),
        }
    }
    let (_, _, c) = mix(a, b, c);
    c
}

/// Convenience wrapper for `&str` callers.
pub fn name_id_str(name: &str) -> u64 {
    name_id(name.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vectors taken from real files, not from this code: the left side is the
    /// `name_hash` column of `grains.big`'s entry table, read off the disc.
    #[test]
    fn member_hash_matches_the_archive_table() {
        assert_eq!(member_hash(b"wood_ramp_soft.grain"), 0x00C8_4267);
        assert_eq!(member_hash(b"concrete_rough_soft.grain"), 0x051E_2336);
        assert_eq!(member_hash(b"asphalt_smooth_soft.grain"), 0x355A_ED25);
    }

    /// The right side is the u64 stored at `+0x30` of `.ems` emitter records that
    /// reference these banks; `trees_rustle.abk` and the rest are real members.
    #[test]
    fn name_id_matches_ems_records() {
        assert_eq!(name_id_str("trees_rustle"), 0x91C4_1263_C312_C84B);
        assert_eq!(name_id_str("transformer_small_1"), 0xE473_924E_09D4_3D2B);
        assert_eq!(name_id_str("clothes_flap"), 0xA4F3_73A1_E59B_7E39);
        assert_eq!(name_id_str("water_fountain"), 0xFAE3_503B_95E0_A3C8);
    }

    /// Constants the shipping code assembles with `lis`/`ori`/`rldimi`. The last one
    /// is 27 bytes, so it is the only vector here that runs the 24-byte block loop
    /// rather than the tail alone.
    #[test]
    fn name_id_matches_code_constants() {
        assert_eq!(name_id_str("default"), 0xD7ED_BD36_2D7D_2152);
        assert_eq!(name_id_str("challenges"), 0x8876_E0B5_5674_0E36);
        assert_eq!(name_id_str("livingworld_entities"), 0x60D7_03C7_7CAB_1631);
        assert_eq!(
            name_id_str("SetManualAndWalkAsConnector"),
            0x0661_A17A_A0E5_C774
        );
    }

    #[test]
    fn name_id_is_length_sensitive() {
        assert_ne!(name_id(b""), name_id(b"\0"));
        assert_ne!(name_id_str("cicadas"), name_id_str("cicadas "));
    }
}
