//! `sub_82B370E8` — expand three parameter blocks into the per-channel slots of a speaker layout.
//!
//! Ported from `recomp/src/audio_ports/sub_82B370E8.inc`, **STATUS: thin** — verified with zero
//! divergence, on 23 comparable calls in the boot profile and 10,112 in a played one.
//!
//! The object's byte at +42 names a layout — 1, 2, 4, 6, or anything above 6 — and each layout picks
//! which source floats land in which of six 8-byte slots, in each of three groups. Every slot is
//! written as `{0x7FF7FFF1, value}`: a sentinel word, then the float. Group 0's values are
//! square-rooted on the way through. Layouts 0, 3 and 5 write nothing at all.
//!
//! Reading the six slots as `{L, C, R, Ls, Rs, LFE}` fits all five layouts, but that is a reading,
//! not a finding: nothing in the binary names them.

use crate::vmx::Fpscr;
use crate::{Guest, Result, fp};

/// `lbz r10,42(r3)` — the layout byte.
pub const LAYOUT: u32 = 42;
/// `lis r11,32759 ; ori r11,r11,65521`. A NaN bit pattern that the function treats as an opaque word.
pub const SENTINEL: u32 = ((32759u32 & 0xFFFF) << 16) + 65521;
const _: () = assert!(SENTINEL == 0x7FF7_FFF1);
/// Three groups of six slots.
pub const GROUPS: u32 = 3;
/// Slots per group; the fifth-index slot is the one the wide layouts add.
pub const SLOTS: u32 = 6;

/// Group `g`'s six 8-byte slots: +48, +96, +144.
pub const fn group_dest(group: u32) -> u32 {
    48 + 48 * group
}
/// Group `g`'s two-slot tail, filled only above layout 6: +192, +208, +224.
pub const fn group_tail(group: u32) -> u32 {
    192 + 16 * group
}
/// Group `g`'s eight source floats: +272, +304, +336. Disjoint from every slot, so no load can read
/// a byte this function stores.
pub const fn group_source(group: u32) -> u32 {
    272 + 32 * group
}

/// `(slot, source)` pairs per layout, in the order each block of the original emits them.
const LAYOUT_1: &[(u32, u32)] = &[(1, 0)];
const LAYOUT_2: &[(u32, u32)] = &[(0, 0), (2, 1)];
const LAYOUT_4: &[(u32, u32)] = &[(0, 0), (2, 1), (3, 2), (4, 3)];
const LAYOUT_WIDE: &[(u32, u32)] = &[(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)];

/// The `cmplwi`/`bne` chain on the layout byte, then `cmplwi cr6,r10,6 ; bltlr cr6`.
fn plan_for(layout: u32) -> Option<&'static [(u32, u32)]> {
    match layout {
        1 => Some(LAYOUT_1),
        2 => Some(LAYOUT_2),
        4 => Some(LAYOUT_4),
        _ if layout < 6 => None,
        _ => Some(LAYOUT_WIDE),
    }
}

/// One slot: the sentinel word, then the source float, square-rooted for group 0.
fn entry(g: &mut Guest, object: u32, group: u32, dest: u32, source: u32) -> Result<()> {
    let mut value = fp::load_single(g, object.wrapping_add(group_source(group) + 4 * source))?;
    if group == 0 {
        value = f64::from(value.sqrt() as f32); // fsqrts: the double root rounded to single
    }
    g.set_u32(object.wrapping_add(dest), SENTINEL)?; // stw r11,dest(r3)
    fp::store_single(g, object.wrapping_add(dest + 4), value)?; // stfs f,dest+4(r3)
    Ok(())
}

/// Fill the slots the object's layout names (`sub_82B370E8`). `object` is `r3`.
///
/// Group 2 is written first, then 1, then 0, each in ascending slot order; exactly 6 then puts source
/// 5 in slot 5, and above 6 puts source 7 there and fills each group's tail from sources 5 and 6.
pub fn expand_layout(g: &mut Guest, object: u32) -> Result<()> {
    let layout = u32::from(g.u8(object.wrapping_add(LAYOUT))?);
    let Some(plan) = plan_for(layout) else {
        return Ok(()); // bltlr cr6 -- layouts 0, 3 and 5
    };
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    for group in (0..GROUPS).rev() {
        for &(slot, source) in plan {
            entry(g, object, group, group_dest(group) + 8 * slot, source)?;
        }
    }
    if layout < 6 {
        return Ok(()); // layouts 1, 2 and 4 are complete
    }
    let fifth = if layout == 6 { 5 } else { 7 };
    for group in (0..GROUPS).rev() {
        entry(g, object, group, group_dest(group) + 8 * (SLOTS - 1), fifth)?;
    }
    if layout == 6 {
        return Ok(());
    }
    for group in (0..GROUPS).rev() {
        entry(g, object, group, group_tail(group), 5)?;
        entry(g, object, group, group_tail(group) + 8, 6)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBJECT: u32 = 0x4000_0000;
    const POISON: u32 = 0xDEAD_BEEF;

    /// Every slot poisoned; group 0's sources are perfect squares so their roots are exact, and the
    /// other groups hold `100 * group + index`.
    fn object(layout: u8) -> Guest {
        let mut g = Guest::single(OBJECT, 0x200);
        for at in (48..240).step_by(4) {
            g.set_u32(OBJECT + at, POISON).unwrap();
        }
        for group in 0..GROUPS {
            for k in 0..8u32 {
                let v = if group == 0 {
                    ((k + 1) * (k + 1)) as f32
                } else {
                    (100 * group + k) as f32
                };
                g.set_u32(OBJECT + group_source(group) + 4 * k, v.to_bits())
                    .unwrap();
            }
        }
        g.set_u8(OBJECT + LAYOUT, layout).unwrap();
        g
    }

    fn slot(g: &Guest, at: u32) -> (u32, f32) {
        (g.u32(OBJECT + at).unwrap(), g.f32(OBJECT + at + 4).unwrap())
    }

    #[test]
    fn layouts_zero_three_and_five_write_nothing() {
        for layout in [0u8, 3, 5] {
            let mut g = object(layout);
            expand_layout(&mut g, OBJECT).unwrap();
            for at in (48..240).step_by(4) {
                assert_eq!(
                    g.u32(OBJECT + at).unwrap(),
                    POISON,
                    "layout {layout}, +{at}"
                );
            }
        }
    }

    #[test]
    fn stereo_fills_slots_zero_and_two_and_roots_group_zero() {
        let mut g = object(2);
        expand_layout(&mut g, OBJECT).unwrap();
        assert_eq!(slot(&g, group_dest(0)), (SENTINEL, 1.0), "sqrt(1)");
        assert_eq!(
            slot(&g, group_dest(0) + 16),
            (SENTINEL, 2.0),
            "sqrt(4), from source 1"
        );
        assert_eq!(
            slot(&g, group_dest(1)),
            (SENTINEL, 100.0),
            "copied, not rooted"
        );
        assert_eq!(slot(&g, group_dest(2) + 16), (SENTINEL, 201.0));
        assert_eq!(
            g.u32(OBJECT + group_dest(1) + 8).unwrap(),
            POISON,
            "slot 1 is the mono slot"
        );
        assert_eq!(g.u32(OBJECT + group_tail(0)).unwrap(), POISON);
    }

    #[test]
    fn six_puts_source_five_in_slot_five_and_leaves_the_tail() {
        let mut g = object(6);
        expand_layout(&mut g, OBJECT).unwrap();
        assert_eq!(slot(&g, group_dest(1) + 40), (SENTINEL, 105.0));
        assert_eq!(slot(&g, group_dest(0) + 40), (SENTINEL, 6.0), "sqrt(36)");
        assert_eq!(
            slot(&g, group_dest(2) + 8),
            (SENTINEL, 201.0),
            "the wide layouts use slot 1"
        );
        for group in 0..GROUPS {
            assert_eq!(g.u32(OBJECT + group_tail(group)).unwrap(), POISON);
        }
    }

    #[test]
    fn above_six_puts_source_seven_in_slot_five_and_fills_the_tail() {
        let mut g = object(9);
        expand_layout(&mut g, OBJECT).unwrap();
        assert_eq!(slot(&g, group_dest(1) + 40), (SENTINEL, 107.0));
        assert_eq!(slot(&g, group_tail(1)), (SENTINEL, 105.0));
        assert_eq!(slot(&g, group_tail(1) + 8), (SENTINEL, 106.0));
        assert_eq!(slot(&g, group_tail(0)), (SENTINEL, 6.0));
        assert_eq!(slot(&g, group_tail(0) + 8), (SENTINEL, 7.0));
    }
}
