//! Endless Tricks rungs: the flip ladder past retail's quad.
//!
//! **Not retail.** Skate 3's scorable table is 332 entries and tops out at `kickflip4` (135),
//! `heelflip4` (134), `n_kickflip4` (137) and `n_heelflip4` (136). Nothing here is added to that
//! table: `catalog::IDENTIFIERS`, `conversions::LINKS` and `SCORABLE_COUNT` are recovered data
//! and stay exactly as they are.
//!
//! Instead each extra rung **borrows the retail quad's ledger identity** and overrides only what
//! has to differ -- the published name, the displayed label and the points. That works because
//! the flip ladder *converts* rather than accumulates (`Carrier::convert_to`): the only things
//! that ever reach `ScoreHolder` are the surviving carrier's `Scorable` and its reward, and the
//! quad's `Scorable` is already the right ledger identity for a fifth flip. Same class 3, same
//! score type 2, same repetition bucket, same conversion source. So `Scorable::valid()` passes,
//! `LINKS[135]` is in range and `by_id(135)` resolves -- none of the out-of-range hazards that
//! minting a 333rd id would create.
use crate::animation::{output::attributes::AttributeName, skeleton_input::name::encode};

/// One non-retail rung.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rung {
    /// Which flip this is: 5 for the first past a kickflip quad, 2 for the first past a 360 flip.
    pub rung: u32,
    /// The top rung retail itself authors for this family -- 4 for the cycle ladders, 1 for the
    /// single-clip tricks. Rungs are counted from here, and so are the points.
    pub base_rung: u32,
    /// Retail family stem, matching the authored `ScoringTrick` name lowercased.
    pub family: &'static str,
    /// Mod-only identifier, e.g. `kickflip5`. Never enters `catalog::IDENTIFIERS`.
    pub identifier: &'static str,
    /// The retail scorable whose ledger identity this rung borrows.
    pub base_id: usize,
    /// Bare token as `compose_trick_name` expects it -- no leading `#`, and deliberately not an
    /// `ID_TRICK_*` key. `apt_text::localize` splits on whitespace and echoes any token it has no
    /// entry for, so plain words render as written while an unknown `ID_` key would print raw.
    pub label: &'static str,
}

/// The highest rung the tables carry; `TrainerTuning::endless_flip_max` is clamped to it.
pub const MAX_RUNG: u32 = 16;

/// Retail's authored kickflip ladder steps +50 a rung (100/150/200/250). Extra rungs continue
/// that step from whatever the borrowed retail scorable is worth, so each family keeps its own
/// authored base rather than a number invented here. Linear is deliberate: the ladder converts,
/// so only the final rung banks, and a superlinear curve would let one drop dwarf a whole line.
pub fn points_over(base_points: i32, rung: &Rung) -> i32 {
    base_points + 50 * (rung.rung - rung.base_rung) as i32
}

macro_rules! rung {
    ($family:literal, $base:literal, $from:literal, $n:literal, $suffix:literal, $label:expr) => {
        Rung {
            rung: $n,
            base_rung: $from,
            family: $family,
            identifier: concat!($family, $suffix),
            base_id: $base,
            label: $label,
        }
    };
}

/// Cycle-ladder families: retail authors four rungs, so the extension starts at the fifth and
/// keeps counting by name until the counts stop meaning anything.
macro_rules! cycle_family {
    ($family:literal, $base:literal, $word:literal) => {
        [
            rung!($family, $base, 4, 5, "5", concat!("QUINTUPLE ", $word)),
            rung!($family, $base, 4, 6, "6", concat!("SEXTUPLE ", $word)),
            rung!($family, $base, 4, 7, "7", concat!("SEPTUPLE ", $word)),
            rung!($family, $base, 4, 8, "8", concat!("OCTUPLE ", $word)),
            rung!($family, $base, 4, 9, "9", concat!("ENDLESS ", $word)),
            rung!($family, $base, 4, 10, "10", concat!("ENDLESS ", $word)),
            rung!($family, $base, 4, 11, "11", concat!("ENDLESS ", $word)),
            rung!($family, $base, 4, 12, "12", concat!("ENDLESS ", $word)),
            rung!($family, $base, 4, 13, "13", concat!("ENDLESS ", $word)),
            rung!($family, $base, 4, 14, "14", concat!("ENDLESS ", $word)),
            rung!($family, $base, 4, 15, "15", concat!("ENDLESS ", $word)),
            rung!($family, $base, 4, 16, "16", concat!("ENDLESS ", $word)),
        ]
    };
}

/// Single-clip families: retail authors one rung, so the extension starts at the second and names
/// the rungs by the board rotation they add -- a doubled 360 flip is a 720, not a "double".
macro_rules! degree_family {
    ($family:literal, $base:literal, $word:literal, $endless:literal) => {
        [
            rung!($family, $base, 1, 2, "2", concat!("720 ", $word)),
            rung!($family, $base, 1, 3, "3", concat!("1080 ", $word)),
            rung!($family, $base, 1, 4, "4", concat!("1440 ", $word)),
            rung!($family, $base, 1, 5, "5", concat!("1800 ", $word)),
            rung!($family, $base, 1, 6, "6", concat!("2160 ", $word)),
            rung!($family, $base, 1, 7, "7", concat!("2520 ", $word)),
            rung!($family, $base, 1, 8, "8", concat!("2880 ", $word)),
            rung!($family, $base, 1, 9, "9", $endless),
            rung!($family, $base, 1, 10, "10", $endless),
            rung!($family, $base, 1, 11, "11", $endless),
            rung!($family, $base, 1, 12, "12", $endless),
            rung!($family, $base, 1, 13, "13", $endless),
            rung!($family, $base, 1, 14, "14", $endless),
            rung!($family, $base, 1, 15, "15", $endless),
            rung!($family, $base, 1, 16, "16", $endless),
        ]
    };
}

/// `T_Kickflip.xml`, included once per family, each borrowing its authored quad.
pub static CYCLE_RUNGS: [[Rung; 12]; 4] = [
    cycle_family!("kickflip", 135, "KICKFLIP"),
    cycle_family!("heelflip", 134, "HEELFLIP"),
    cycle_family!("n_kickflip", 137, "NOLLIE KICKFLIP"),
    cycle_family!("n_heelflip", 136, "NOLLIE HEELFLIP"),
];

/// `T_TrickWithDarkCatch.xml`, which authors no cycle at all -- one clip, one scorable.
pub static DEGREE_RUNGS: [[Rung; 15]; 4] = [
    degree_family!("360flip", 85, "FLIP", "ENDLESS 360 FLIP"),
    degree_family!("laserflip", 99, "LASERFLIP", "ENDLESS LASERFLIP"),
    degree_family!("n_360flip", 106, "NOLLIE FLIP", "ENDLESS NOLLIE 360 FLIP"),
    degree_family!("n_laserflip", 120, "NOLLIE LASERFLIP", "ENDLESS NOLLIE LASERFLIP"),
];

fn all() -> impl Iterator<Item = &'static Rung> {
    CYCLE_RUNGS.iter().flatten().chain(DEGREE_RUNGS.iter().flatten())
}

/// Resolve a rung from a name published by the graph, e.g. `encode(b"Kickflip5")`.
///
/// `encode` case-folds, so the graph's `Kickflip5` and this table's `kickflip5` are one key --
/// the same reason the authored `trick="Kickflip4"` resolves against `catalog`'s `kickflip4`.
pub fn by_name(name: AttributeName) -> Option<&'static Rung> {
    all().find(|r| encode(r.identifier.as_bytes()) == name)
}

/// Resolve a rung by number within a family stem, e.g. `("kickflip", 5)`.
pub fn by_family(family: &str, rung: u32) -> Option<&'static Rung> {
    all().find(|r| r.rung == rung && r.family.eq_ignore_ascii_case(family))
}

/// Resolve a rung by number and the retail scorable it borrows.
pub fn by_rung(rung: u32, base_id: usize) -> Option<&'static Rung> {
    all().find(|r| r.rung == rung && r.base_id == base_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rungs_continue_the_authored_step_and_borrow_a_retail_quad() {
        // The authored quad is 250; the first extra rung is one +50 step past it.
        assert_eq!(points_over(250, by_family("kickflip", 5).unwrap()), 300);
        assert_eq!(points_over(250, by_family("kickflip", 16).unwrap()), 850);
        // A single-clip family counts from its one authored rung instead.
        assert_eq!(points_over(400, by_family("360flip", 2).unwrap()), 450);
        assert_eq!(by_family("360flip", 2).unwrap().label, "720 FLIP");
        assert_eq!(by_family("360flip", 9).unwrap().label, "ENDLESS 360 FLIP");
        assert_eq!(by_family("laserflip", 3).unwrap().label, "1080 LASERFLIP");
        assert_eq!(by_family("laserflip", 12).unwrap().label, "ENDLESS LASERFLIP");
        for rung in all() {
            assert!(
                (2..=MAX_RUNG).contains(&rung.rung),
                "{} is outside the extension range",
                rung.identifier
            );
            // Borrowing a retail id is what keeps `Scorable::valid()` true and `LINKS` in range.
            assert!(
                rung.base_id < crate::scoring::SCORABLE_COUNT,
                "{} does not borrow a retail quad",
                rung.identifier
            );
            // An `ID_`-prefixed label would reach the HUD as a raw unlocalised token.
            assert!(
                !rung.label.contains("ID_"),
                "{} would render as a raw key",
                rung.identifier
            );
        }
    }

    #[test]
    fn counted_names_stop_at_the_octuple_and_everything_above_reads_endless() {
        // Named counts run out at eight; past that the trick is the mod's own name rather than a
        // number nobody is counting in the air.
        assert_eq!(by_family("kickflip", 8).unwrap().label, "OCTUPLE KICKFLIP");
        for rung in 9..=MAX_RUNG {
            assert_eq!(
                by_family("heelflip", rung).unwrap().label,
                "ENDLESS HEELFLIP",
                "rung {rung} should read endless"
            );
        }
        assert_eq!(
            by_family("n_kickflip", 9).unwrap().label,
            "ENDLESS NOLLIE KICKFLIP"
        );
        // Labels repeat above the boundary, so identifiers stay the unique key.
        assert_ne!(
            by_family("kickflip", 9).unwrap().identifier,
            by_family("kickflip", 10).unwrap().identifier
        );
    }

    #[test]
    fn names_resolve_case_insensitively_the_way_the_graph_publishes_them() {
        let rung = by_name(encode(b"Kickflip5")).expect("graph spelling");
        assert_eq!(rung.identifier, "kickflip5");
        assert_eq!(rung.base_id, 135);
        // No retail scorable is ever mistaken for an extension rung.
        assert!(by_name(encode(b"kickflip4")).is_none());
        assert!(by_name(encode(b"ollie")).is_none());
    }
}
