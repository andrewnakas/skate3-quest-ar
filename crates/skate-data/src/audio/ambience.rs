//! Choosing an ambience bed for a place.
//!
//! `ambienceresident.big` holds 24 five-channel beds, each about two minutes, and **their member
//! names say where they belong**: a two-digit index, a district token, and a qualifier. Downtown
//! beds are `dt`, university `univ`, industrial `indu`, plus reclaimed land, interiors and the
//! start screen.
//!
//! ## This is a stand-in, and the difference matters
//!
//! The game does not choose beds by name. It chooses them through audio metadata that is **not
//! decoded** -- the banks in `audiofiles.big` and the mix map. What this module does is read the
//! names the archive already carries and match them against a place name, which is enough for the
//! engine to have ambience now and is **not** a reconstruction of what the game does. A bed picked
//! here may not be the bed Skate 3 would play in the same spot.
//!
//! So the rule is: match on evidence in the name, and when nothing matches, return `None` and play
//! nothing. Silence is a visible gap; a confidently wrong bed is not.

/// One bed, by the member name it has in the archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bed {
    pub member: String,
    /// The district token, e.g. `dt`, `univ`, `indu`.
    pub district: String,
    /// Everything after the district, e.g. `mt_low`, `open`. Empty when the name is just a place.
    pub qualifier: String,
}

/// Split a member name into its parts. `08_univ_mt_low.snr` is district `univ`, qualifier
/// `mt_low`. A name that does not start with the two-digit index is not one of these beds.
pub fn parse_bed(member: &str) -> Option<Bed> {
    let stem = member.strip_suffix(".snr").unwrap_or(member);
    let (index, rest) = stem.split_once('_')?;
    if index.len() != 2 || !index.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (district, qualifier) = rest.split_once('_').unwrap_or((rest, ""));
    Some(Bed {
        member: member.to_string(),
        district: district.to_ascii_lowercase(),
        qualifier: qualifier.to_ascii_lowercase(),
    })
}

/// District tokens and the words in a place name that imply them.
///
/// Deliberately short. Every entry is a token that actually appears in the archive paired with a
/// word that actually appears in Skate 3's place names; there is no entry here for a district
/// nobody named, because a guess would be indistinguishable from a reading.
const DISTRICTS: &[(&str, &[&str])] = &[
    ("dt", &["downtown", "dt"]),
    ("univ", &["university", "univ", "campus"]),
    (
        "indu",
        &[
            "industrial",
            "indu",
            "factory",
            "shipyard",
            "quarry",
            "drydock",
        ],
    ),
    ("reclaimed", &["reclaimed"]),
    ("spillway", &["spillway"]),
    ("skate", &["school", "plaza"]),
    ("space", &["park"]),
];

/// Pick a bed for a place name, or `None` when nothing in the name matches.
///
/// Scoring, in order: a district whose words appear in the place name, then a qualifier word that
/// also appears, then a general-sounding qualifier when the place name gave no hint. That last
/// step is a stated preference, not a reading: with nothing to go on, a district's `main` or
/// `open` bed suits a whole district better than its apartment interior, and without it the
/// choice falls to whichever member sorts first. Ties go to the lowest index, so the choice is
/// stable rather than dependent on the archive's order.
pub fn pick<'a>(place: &str, beds: &'a [Bed]) -> Option<&'a Bed> {
    let place = place.to_ascii_lowercase();
    let words: Vec<&str> = place
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let mut best: Option<(usize, &Bed)> = None;
    for bed in beds {
        let Some((_, hints)) = DISTRICTS.iter().find(|(token, _)| *token == bed.district) else {
            continue;
        };
        if !hints.iter().any(|h| words.iter().any(|w| w == h)) {
            continue;
        }
        let mut score = 1usize;
        if !bed.qualifier.is_empty()
            && bed
                .qualifier
                .split('_')
                .any(|q| words.iter().any(|w| *w == q))
        {
            score += 2;
        } else if matches!(bed.qualifier.as_str(), "main" | "open") {
            score += 1;
        }
        let better = match &best {
            None => true,
            Some((s, current)) => score > *s || (score == *s && bed.member < current.member),
        };
        if better {
            best = Some((score, bed));
        }
    }
    best.map(|(_, bed)| bed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beds() -> Vec<Bed> {
        [
            "08_univ_mt_low.snr",
            "09_univ_campus.snr",
            "04_dt_main.snr",
            "06_dt_open.snr",
            "13_indu_quarry.snr",
            "16_spillway.snr",
            "23_Press_start_screen.snr",
        ]
        .iter()
        .filter_map(|m| parse_bed(m))
        .collect()
    }

    #[test]
    fn a_member_name_splits_into_district_and_qualifier() {
        let bed = parse_bed("08_univ_mt_low.snr").expect("a bed");
        assert_eq!(bed.district, "univ");
        assert_eq!(bed.qualifier, "mt_low");
        assert_eq!(parse_bed("16_spillway.snr").unwrap().qualifier, "");
        // Not a bed: no two-digit index. Wheel and grain members must not be read as places.
        assert!(parse_bed("Whls_spins_Jump_1.snr").is_none());
        assert!(parse_bed("univ_campus.snr").is_none());
    }

    #[test]
    fn a_place_with_no_matching_district_picks_nothing() {
        // The failure that matters: a custom map should fall silent, not inherit downtown.
        let beds = beds();
        assert!(pick("IsleOfGrom", &beds).is_none());
        assert!(pick("", &beds).is_none());
    }

    #[test]
    fn a_qualifier_word_breaks_the_tie_within_a_district() {
        let beds = beds();
        // "campus" is a university word AND a qualifier, so it beats the other university bed.
        assert_eq!(
            pick("University Campus", &beds).unwrap().member,
            "09_univ_campus.snr"
        );
        // Without it, the district still matches and the lowest index wins, stably.
        assert_eq!(
            pick("University", &beds).unwrap().member,
            "08_univ_mt_low.snr"
        );
        // A named qualifier still outranks the general-sounding preference below.
        assert_eq!(
            pick("Downtown open", &beds).unwrap().member,
            "06_dt_open.snr"
        );
    }

    #[test]
    fn a_district_with_no_hint_prefers_its_general_bed() {
        // "01_dt_apt" sorts first, but an apartment interior is a poor stand-in for all of
        // downtown; "04_dt_main" is the one a district-wide request should get.
        let beds: Vec<Bed> = ["01_dt_apt.snr", "04_dt_main.snr", "06_dt_open.snr"]
            .iter()
            .filter_map(|m| parse_bed(m))
            .collect();
        assert_eq!(pick("Downtown", &beds).unwrap().member, "04_dt_main.snr");
    }

    #[test]
    fn district_words_are_matched_whole_not_as_substrings() {
        let beds = beds();
        // "dt" inside another word must not select downtown; that is the kind of loose match
        // that would make every map sound like a city.
        assert!(pick("Verdterra", &beds).is_none());
        assert_eq!(pick("Downtown", &beds).unwrap().district, "dt");
    }
}
