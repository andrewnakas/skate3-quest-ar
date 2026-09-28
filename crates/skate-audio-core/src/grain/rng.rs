//! The title's shared random-number generator, `sub_82A8AF10`.
//!
//! Six big-endian words at [`STATE`] (`lis -32003 ; addi 32116`): a multi-word add-with-carry
//! generator. Each call adds `s4 + s5`, then folds that sum through `s3`, `s2`, `s1` and `s0` with
//! the carry of every add, increments `s5`, and — only when `s5` wraps to zero — ripples a +1 up
//! through `s4 .. s0`. The result is the new `s0`.
//!
//! The state is **global**: the lifted corpus has about a hundred `bl 0x82a8af10` sites across the
//! game (camera, AI, particles, audio …), so the grain player's picks depend on every other caller
//! since boot. The dumped image supplies the state as it stood when the image was taken; a retail
//! pick *sequence* cannot be reproduced without the rest of the game (reported as unrecoverable,
//! not approximated). The *distribution* and every arithmetic step are the retail ones.

use crate::{Guest, Result};

/// `lis r11,-32003 ; addi r11,r11,32116`.
pub const STATE: u32 = 0x82FD_7D74;

const _: () = assert!(STATE == ((((-32003i32) as u32) & 0xFFFF) << 16).wrapping_add(32116));

/// `cmplw r9,rOld ; blt → 1 ; bne → 0 ; cmplwi rCarry,0 ; bne → 1 ; → 0`: the carry out of
/// `old + addend + carry_in`, decided the way the original decides it from the wrapped sum.
#[inline]
fn carry_out(sum: u32, old: u32, carry_in: u32) -> u32 {
    if sum < old {
        1
    } else if sum != old {
        0
    } else if carry_in != 0 {
        1
    } else {
        0
    }
}

/// `sub_82A8AF10`: advance the generator and return the new `s0`.
pub fn next(g: &mut Guest) -> Result<u32> {
    let s = STATE;
    // lwz r10,16 ; lwz r4,20 ; lwz r9,12 ; add r10,r10,r4 ; subfc r8,r4,r10 ; stw r10,16
    let s5 = g.u32(s + 20)?;
    let r10 = g.u32(s + 16)?.wrapping_add(s5);
    // subfc/subfe/clrlwi: 1 when the add overflowed (r10 < r4 unsigned).
    let c1 = u32::from(r10 < s5);
    g.set_u32(s + 16, r10)?;
    // add r9,r9,r10 ; add r9,r9,r8
    let r30 = g.u32(s + 12)?.wrapping_add(r10).wrapping_add(c1);
    let c2 = carry_out(r30, r10, c1);
    g.set_u32(s + 12, r30)?;
    let r31 = g.u32(s + 8)?.wrapping_add(r30).wrapping_add(c2);
    let c3 = carry_out(r31, r30, c2);
    g.set_u32(s + 8, r31)?;
    let r5 = g.u32(s + 4)?.wrapping_add(r31).wrapping_add(c3);
    let c4 = carry_out(r5, r31, c3);
    // lwz r7,0 ; mr r5,r9 ; addic. r8,r4,1 ; add r9,r7,r9 ; stw r5,4 ; stw r8,20 ; add r3,r9,r6
    let r8 = s5.wrapping_add(1);
    let mut r3 = g.u32(s)?.wrapping_add(r5).wrapping_add(c4);
    g.set_u32(s + 4, r5)?;
    g.set_u32(s + 20, r8)?;
    g.set_u32(s, r3)?;
    // The ripple, each step taken only when the previous increment wrapped to zero.
    if r8 == 0 {
        let a = r10.wrapping_add(1);
        g.set_u32(s + 16, a)?;
        if a == 0 {
            let b = r30.wrapping_add(1);
            g.set_u32(s + 12, b)?;
            if b == 0 {
                let c = r31.wrapping_add(1);
                g.set_u32(s + 8, c)?;
                if c == 0 {
                    let d = r5.wrapping_add(1);
                    g.set_u32(s + 4, d)?;
                    if d == 0 {
                        r3 = r3.wrapping_add(1);
                        g.set_u32(s, r3)?;
                    }
                }
            }
        }
    }
    Ok(r3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guest(words: [u32; 6]) -> Guest {
        let mut g = Guest::single(STATE & !0xFFF, 0x2000);
        for (i, w) in words.iter().enumerate() {
            g.set_u32(STATE + 4 * i as u32, *w).unwrap();
        }
        g
    }

    fn state(g: &Guest) -> [u32; 6] {
        std::array::from_fn(|i| g.u32(STATE + 4 * i as u32).unwrap())
    }

    /// A plain 192-bit reference: the same recurrence written with u64 carries.
    fn reference(mut s: [u32; 6]) -> (u32, [u32; 6]) {
        let s4 = u64::from(s[4]) + u64::from(s[5]);
        let mut carry = s4 >> 32;
        s[4] = s4 as u32;
        let mut prev = s[4];
        for k in [3usize, 2, 1, 0] {
            let v = u64::from(s[k]) + u64::from(prev) + carry;
            carry = v >> 32;
            s[k] = v as u32;
            prev = s[k];
        }
        s[5] = s[5].wrapping_add(1);
        if s[5] == 0 {
            for k in [4usize, 3, 2, 1, 0] {
                s[k] = s[k].wrapping_add(1);
                if s[k] != 0 {
                    break;
                }
            }
        }
        (s[0], s)
    }

    #[test]
    fn matches_the_add_with_carry_recurrence() {
        // The dumped image's state (g_82FD.bin + 0x7D74), then edge states that exercise every
        // carry and the ripple.
        let seeds = [
            [
                0xF22D_0E56,
                0x8831_26E9,
                0xC624_DD2F,
                0x0702_C49C,
                0x9E35_3F7D,
                0x6FDF_3B64,
            ],
            [
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
            ],
            [0, 0, 0, 0, 0xFFFF_FFFF, 1],
            [1, 2, 3, 0xFFFF_FFFF, 0, 0xFFFF_FFFF],
        ];
        for seed in seeds {
            let mut g = guest(seed);
            let mut s = seed;
            for _ in 0..1000 {
                let (want, next_state) = reference(s);
                assert_eq!(next(&mut g).unwrap(), want);
                assert_eq!(state(&g), next_state);
                s = next_state;
            }
        }
    }
}
