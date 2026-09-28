//! The voice module classes as the audio system knows them: registration (`sub_824A2FD8` and its
//! callees), cooking a class's parameter defaults (`sub_82B463D8`), and reading one parameter's
//! defaults into a block (`sub_82B46260`).
//!
//! **Unverified new work.** These run on the game thread and have no C++ bodies; each is
//! transcribed from its lifted body and unit-tested.
//!
//! | function | here |
//! |---|---|
//! | `sub_824A2FD8`, register the device's classes once | [`register_voice_classes`] |
//! | `sub_82481B08`, the system's class registry at `+56` | [`class_registry`] |
//! | `sub_82B46770`, register a class, or find it by its `+36` id | [`register_class`] |
//! | `sub_82B463D8`, cook a class's row defaults into tagged values | [`cook_class`] |
//! | `sub_82B48840` and `sub_82B3CCC8`, the second list at `+60` | [`second_list`], [`register_in_second_list`] |
//! | `sub_82B46260`, one parameter's default slots | [`class_defaults`] |
//!
//! **The row table.** `[class+20]` holds 40-byte rows: byte `+1` is a kind and `+8` an 8-byte slot.
//! The first `[class+41]` rows come first, then `[class+42]` rows an instance copies when it is
//! constructed ([`crate::modules::copy_class_rows`]), then for each parameter `i < [class+43]` the
//! `[[class+24] + 8i]` rows it takes. A class ships with a double in every slot; cooking rewrites
//! each slot by its kind:
//!
//! | kind | slot after cooking |
//! |---|---|
//! | 0, 1 | [`TAG_SINGLE`], then the double rounded to a single |
//! | 2 | the double, rewritten |
//! | 3 | [`TAG_STRING`], then 0 |
//! | 4 | [`TAG_POINTER`], then 0 |
//! | 5 | [`TAG_INTEGER`], then the low word of the double truncated |
//!
//! **Locks.** `sub_82481B08` brackets its allocation with the system's lock (`[system+84]`, or
//! `sub_82F9CB44` on `[system+96]`, and the matching unlock). The calls are not made here and the
//! lock fields are not read.

use crate::fp::{fctidz, frsp, load_double, rlwinm, store_single};
use crate::modules::SYSTEM;
use crate::patch::Heap;
use crate::{Guest, Result};

/// `lis 32759 ; ori 65521`.
pub const TAG_SINGLE: u32 = 0x7FF7_FFF1;
/// `ori 65523`.
pub const TAG_STRING: u32 = 0x7FF7_FFF3;
/// `ori 65524`.
pub const TAG_POINTER: u32 = 0x7FF7_FFF4;
/// `ori 65525`.
pub const TAG_INTEGER: u32 = 0x7FF7_FFF5;

/// `lis -31992 ; addi 10471`: the registration flag byte; the registry and class slots follow it.
pub const REGISTERED: u32 = 0x8308_28E7;
/// `stw r3,13(r30)`.
pub const REGISTRY_SLOT: u32 = REGISTERED + 13;
/// `lis -31993 ; lwz 30188`: the default output bus.
pub const DEFAULT_BUS: u32 = 0x8307_75EC;
/// `lis -31987 ; lwz -580`: the object whose `[+44]` holds the default bus.
pub const BUS_ROOT: u32 = 0x830C_FDBC;
/// `lis 21349 ; ori 28208`: "Sen0", the id the Send class is found by.
pub const SEND_ID: u32 = 0x5365_6E30;
/// `lis -32003 ; addi -11104`: the item registered in the system's second list.
pub const SECOND_LIST_ITEM: u32 = 0x82FC_D4A0;

/// The class slots `sub_824A2FD8` fills, with the descriptor each is registered from, in its order.
pub const CLASSES: [(u32, u32, &str); 8] = [
    (REGISTERED + 17, 0x82FC_E3BC, "Gain"),
    (REGISTERED + 21, 0x82FC_E910, "HighPassIir2"),
    (REGISTERED + 25, 0x82FC_F3D8, "LowPassIir2"),
    (REGISTERED + 29, 0x82FD_11D4, "Pan2D1"),
    (REGISTERED + 37, 0x82FD_2B50, "SndPlayer1"),
    (REGISTERED + 41, 0x82FD_1844, "Rechannel"),
    (REGISTERED + 45, 0x82FD_18B0, "Resample"),
    (REGISTERED + 49, 0x82FD_1788, "PeakingIir2"),
];
/// `stw r10,33(r30)`: the Send class, found in the registry rather than registered.
pub const SEND_SLOT: u32 = REGISTERED + 33;

const _: () = {
    const fn lis(hi: i32, lo: i32) -> u32 {
        (((hi & 0xFFFF) << 16) as u32).wrapping_add(lo as u32)
    }
    assert!(
        REGISTERED == lis(-31992, 10471)
            && DEFAULT_BUS == lis(-31993, 30188)
            && BUS_ROOT == lis(-31987, -580)
    );
    assert!(SEND_ID == (21349 << 16) | 28208 && SECOND_LIST_ITEM == lis(-32003, -11104));
    assert!(
        CLASSES[0].1 == lis(-32003, -7236)
            && CLASSES[1].1 == lis(-32003, -5872)
            && CLASSES[2].1 == lis(-32003, -3112)
    );
    assert!(
        CLASSES[3].1 == lis(-32003, 4564)
            && CLASSES[4].1 == lis(-32003, 11088)
            && CLASSES[5].1 == lis(-32003, 6212)
    );
    assert!(CLASSES[6].1 == lis(-32003, 6320) && CLASSES[7].1 == lis(-32003, 6024));
    assert!(CLASSES[0].0 == 0x8308_28F8 && SEND_SLOT == 0x8308_2908 && CLASSES[7].0 == 0x8308_2918);
};

/// `rlwinm rX,rY,2,0,29 ; add ; rlwinm rX,rX,3,0,28`: forty times a word, as the originals form it.
fn forty(word: u64) -> u64 {
    rlwinm(word + rlwinm(word, 2, 0xFFFF_FFFC), 3, 0xFFFF_FFF8)
}

/// `sub_82B463D8`: cook every row of `class` once (its `+46` byte is the done flag).
pub fn cook_class(g: &mut Guest, class: u32) -> Result<()> {
    if g.u8(class.wrapping_add(46))? != 0 {
        return Ok(());
    }
    let instance_rows = g.u8(class.wrapping_add(42))? as u64; // lbz r8,42(r3)
    let leading_rows = g.u8(class.wrapping_add(41))? as u64; // lbz r11,41(r3)
    let params = g.u8(class.wrapping_add(43))? as u32; // lbz r6,43(r3)
    g.set_u8(class.wrapping_add(46), 1)?;
    let mut fixed = instance_rows + leading_rows; // r4
    let (mut firsts, mut seconds) = (0u64, 0u64); // r10, r9
    let mut paired = 0u32; // r5
    if params as i32 >= 2 {
        let pairs = (params.wrapping_sub(2) >> 1) + 1; // rlwinm r8,r8,31,1,31 ; addi r8,r8,1
        paired = pairs.wrapping_mul(2);
        let mut at = g.u32(class.wrapping_add(24))?.wrapping_sub(8);
        for _ in 0..pairs {
            firsts += g.u32(at.wrapping_add(8))? as u64; // lwz r7,8(r11)
            at = at.wrapping_add(16); // lwzu r8,16(r11)
            seconds += g.u32(at)? as u64;
        }
    }
    if paired < params {
        let counts = g.u32(class.wrapping_add(24))?;
        fixed += g.u32(counts.wrapping_add(paired.wrapping_mul(8)))? as u64;
    }
    let total = (seconds + firsts + fixed) as u32; // add r11,r9,r10 ; add r7,r11,r4
    let mut done = 0u32; // r6
    if total as i32 >= 4 {
        let groups = (total.wrapping_sub(4) >> 2) + 1; // rlwinm r11,r11,30,2,31 ; addi 1
        done = groups.wrapping_mul(4);
        let mut offset = 0u32; // r8
        for _ in 0..groups {
            let table = g.u32(class.wrapping_add(20))?;
            cook_row(g, offset.wrapping_add(table))?;
            let table = g.u32(class.wrapping_add(20))?;
            cook_row(g, offset.wrapping_add(table).wrapping_add(40))?;
            let table = g.u32(class.wrapping_add(20))?;
            cook_row(
                g,
                offset
                    .wrapping_add(120)
                    .wrapping_add(table)
                    .wrapping_sub(40),
            )?;
            let table = g.u32(class.wrapping_add(20))?;
            cook_row(g, offset.wrapping_add(120).wrapping_add(table))?;
            offset = offset.wrapping_add(160);
        }
    }
    if done < total {
        let mut offset = forty(done as u64) as u32; // r9
        for _ in 0..total - done {
            let table = g.u32(class.wrapping_add(20))?;
            cook_row(g, offset.wrapping_add(table))?;
            offset = offset.wrapping_add(40);
        }
    }
    Ok(())
}

/// One row's slot, by the kind byte at `row + 1`.
fn cook_row(g: &mut Guest, row: u32) -> Result<()> {
    let slot = row.wrapping_add(8); // addi r11,r10,8
    let kind = g.u8(row.wrapping_add(1))?; // lbz r10,1(r10)
    let value = load_double(g, slot)?; // lfd f0,0(r11)
    match kind {
        0 | 1 => {
            store_single(g, slot.wrapping_add(4), frsp(value))?; // frsp ; stfs f0,4(r11)
            g.set_u32(slot, TAG_SINGLE)?;
        }
        2 => {
            let bits = g.u64(slot)?; // stfd f0,0(r11): the same eight bytes back
            g.set_u64(slot, bits)?;
        }
        5 => {
            g.set_u32(slot.wrapping_add(4), fctidz(value) as u32)?; // fctidz ; stfiwx f0,r11,r27
            g.set_u32(slot, TAG_INTEGER)?;
        }
        4 => {
            g.set_u32(slot.wrapping_add(4), 0)?;
            g.set_u32(slot, TAG_POINTER)?;
        }
        3 => {
            g.set_u32(slot.wrapping_add(4), 0)?;
            g.set_u32(slot, TAG_STRING)?;
        }
        _ => {}
    }
    Ok(())
}

/// `sub_82B46260`: copy parameter `index`'s default slots of `class` into `out`, 8 bytes each.
pub fn class_defaults(g: &mut Guest, class: u32, index: u32, out: u32) -> Result<()> {
    let instance_rows = g.u8(class.wrapping_add(42))? as u64;
    let at_count = rlwinm(index as u64, 3, 0xFFFF_FFF8); // r28
    let leading_rows = g.u8(class.wrapping_add(41))? as u64;
    let table = g.u32(class.wrapping_add(20))? as u64;
    let counts = g.u32(class.wrapping_add(24))?; // r31
    let mut first = forty(instance_rows + leading_rows) + table; // r30
    let (mut firsts, mut seconds) = (0u64, 0u64); // r8, r7
    let mut paired = 0u32; // r29
    if index as i32 >= 2 {
        let pairs = (index.wrapping_sub(2) >> 1) + 1;
        paired = pairs.wrapping_mul(2);
        let mut at = counts.wrapping_sub(8);
        for _ in 0..pairs {
            let a = g.u32(at.wrapping_add(8))? as u64; // lwz r9,8(r11)
            at = at.wrapping_add(16);
            let b = g.u32(at)? as u64; // lwzu r10,16(r11)
            firsts += forty(a);
            seconds += forty(b);
        }
    }
    if paired < index {
        let c = g.u32(counts.wrapping_add(paired.wrapping_mul(8)))? as u64;
        first += forty(c);
    }
    let count = g.u32(counts.wrapping_add(at_count as u32))? as u64; // lwzx r11,r28,r31
    let source = seconds + firsts + first; // add r10,r7,r8 ; add r10,r10,r30
    let end = rlwinm(count, 3, 0xFFFF_FFF8) + out as u64;
    if out < end as u32 {
        let words = ((end.wrapping_sub(out as u64).wrapping_sub(1) as u32) >> 3) + 1;
        let mut from = (source as u32).wrapping_sub(32);
        let mut to = out.wrapping_sub(8);
        for _ in 0..words {
            from = from.wrapping_add(40); // ldu r9,40(r11)
            let word = g.u64(from)?;
            to = to.wrapping_add(8); // stdu r9,8(r10)
            g.set_u64(to, word)?;
        }
    }
    Ok(())
}

/// `sub_82B46770`: register `class` in `registry` (`{head, tail, count, 0, system}`, linked through
/// each class's `+32`), unless a class with the same `+36` id is there already, which it returns.
pub fn register_class(g: &mut Guest, registry: u32, class: u32) -> Result<u32> {
    let head = g.u32(registry)?;
    if head != 0 {
        let id = g.u32(class.wrapping_add(36))?;
        let mut node = head;
        loop {
            let existing = node.wrapping_sub(32);
            node = g.u32(node)?;
            if g.u32(existing.wrapping_add(36))? == id {
                return Ok(existing);
            }
            if node == 0 {
                break;
            }
        }
    }
    cook_class(g, class)?; // bl sub_82B463D8, which leaves r3 the class
    let link = class.wrapping_add(32);
    let head = g.u32(registry)?;
    g.set_u32(class.wrapping_add(32), head)?;
    if g.u32(registry.wrapping_add(4))? == 0 {
        g.set_u32(registry.wrapping_add(4), link)?;
    }
    let count = g.u32(registry.wrapping_add(8))?;
    g.set_u32(registry, link)?;
    g.set_u32(registry.wrapping_add(8), count.wrapping_add(1))?;
    Ok(class)
}

/// Find the registered class whose four-character id is `id`.
///
/// This is the lookup loop in `sub_82490CA0`, factored out so the bus-manager constructor can
/// retain its registry order without registering classes that the title only expects to find.
pub fn find_class(g: &Guest, registry: u32, id: u32) -> Result<u32> {
    let mut link = g.u32(registry)?;
    while link != 0 {
        let class = link.wrapping_sub(32);
        if g.u32(class.wrapping_add(36))? == id {
            return Ok(class);
        }
        link = g.u32(link)?;
    }
    Err(crate::Error::new(
        id,
        "required audio class is not registered",
    ))
}

/// `sub_82481B08`: the system's class registry at `+56`, allocated (20 bytes) the first time.
pub fn class_registry<H: Heap + ?Sized>(g: &mut Guest, heap: &mut H, system: u32) -> Result<u32> {
    if g.u32(system.wrapping_add(56))? == 0 {
        let owner = g.u32(SYSTEM)?;
        let registry = heap.alloc(g, 20, 16)?;
        if registry != 0 {
            g.set_u32(registry, 0)?;
            g.set_u32(registry.wrapping_add(4), 0)?;
            g.set_u32(registry.wrapping_add(8), 0)?;
            g.set_u32(registry.wrapping_add(16), owner)?;
            g.set_u32(registry.wrapping_add(12), 0)?;
        }
        g.set_u32(system.wrapping_add(56), registry)?;
    }
    g.u32(system.wrapping_add(56))
}

/// `sub_82B48840`: the system's second list at `+60`, allocated (16 bytes) the first time.
pub fn second_list<H: Heap + ?Sized>(g: &mut Guest, heap: &mut H, system: u32) -> Result<u32> {
    if g.u32(system.wrapping_add(60))? == 0 {
        let owner = g.u32(SYSTEM)?;
        let list = heap.alloc(g, 16, 16)?;
        if list != 0 {
            g.set_u32(list, 0)?;
            g.set_u32(list.wrapping_add(4), 0)?;
            g.set_u32(list.wrapping_add(8), 0)?;
            g.set_u32(list.wrapping_add(12), owner)?;
        }
        g.set_u32(system.wrapping_add(60), list)?;
    }
    g.u32(system.wrapping_add(60))
}

/// `sub_82B3CCC8`: [`register_class`]'s pattern for the second list, linked through `+16` and
/// keyed by `+20`, without cooking.
pub fn register_in_second_list(g: &mut Guest, list: u32, item: u32) -> Result<u32> {
    let head = g.u32(list)?;
    if head != 0 {
        let id = g.u32(item.wrapping_add(20))?;
        let mut node = head;
        loop {
            let existing = node.wrapping_sub(16);
            node = g.u32(node)?;
            if g.u32(existing.wrapping_add(20))? == id {
                return Ok(existing);
            }
            if node == 0 {
                break;
            }
        }
    }
    g.set_u32(item.wrapping_add(16), head)?;
    let link = item.wrapping_add(16);
    if g.u32(list.wrapping_add(4))? == 0 {
        g.set_u32(list.wrapping_add(4), link)?;
    }
    let count = g.u32(list.wrapping_add(8))?;
    g.set_u32(list, link)?;
    g.set_u32(list.wrapping_add(8), count.wrapping_add(1))?;
    Ok(item)
}

/// `sub_824A2FD8`: register the voice device's classes and fill [`CLASSES`]' slots and
/// [`SEND_SLOT`]. The device open calls it while the [`REGISTERED`] byte is 0.
pub fn register_voice_classes<H: Heap + ?Sized>(g: &mut Guest, heap: &mut H) -> Result<()> {
    let system = g.u32(SYSTEM)?; // lwz r29,30252(r11)
    let registry = class_registry(g, heap, system)?;
    let bus = g.u32(DEFAULT_BUS)?;
    g.set_u32(REGISTRY_SLOT, registry)?;
    if bus == 0 {
        let root = g.u32(BUS_ROOT)?;
        let table = g.u32(root.wrapping_add(44))?;
        let bus = g.u32(table)?;
        g.set_u32(DEFAULT_BUS, bus)?;
    }
    for (slot, class, _) in CLASSES {
        let registered = register_class(g, registry, class)?;
        g.set_u32(slot, registered)?;
    }
    let mut send = 0;
    let mut node = g.u32(registry)?;
    while node != 0 {
        let class = node.wrapping_sub(32);
        node = g.u32(node)?;
        if g.u32(class.wrapping_add(36))? == SEND_ID {
            send = class;
            break;
        }
    }
    g.set_u32(SEND_SLOT, send)?;
    let list = second_list(g, heap, system)?;
    register_in_second_list(g, list, SECOND_LIST_ITEM)?;
    g.set_u8(REGISTERED, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::BumpHeap;

    const MEM: u32 = 0x5000_0000;
    const CLASS: u32 = MEM + 0x100;
    const TABLE: u32 = MEM + 0x200;
    const COUNTS: u32 = MEM + 0x600;
    const OUT: u32 = MEM + 0x700;
    const SYS: u32 = MEM + 0x800;

    fn guest() -> Guest {
        Guest::single(MEM, 0x2000)
    }

    /// A class with `leading` + `instance` rows and per-parameter `counts`, each row's slot holding
    /// `base + row` as a double and the kinds given (cycled).
    fn class(
        g: &mut Guest,
        leading: u8,
        instance: u8,
        counts: &[u32],
        kinds: &[u8],
        base: f64,
    ) -> u32 {
        g.set_u8(CLASS + 41, leading).unwrap();
        g.set_u8(CLASS + 42, instance).unwrap();
        g.set_u8(CLASS + 43, counts.len() as u8).unwrap();
        g.set_u32(CLASS + 20, TABLE).unwrap();
        g.set_u32(CLASS + 24, COUNTS).unwrap();
        for (i, c) in counts.iter().enumerate() {
            g.set_u32(COUNTS + 8 * i as u32, *c).unwrap();
        }
        let rows = leading as u32 + instance as u32 + counts.iter().sum::<u32>();
        for r in 0..rows {
            g.set_u8(TABLE + 40 * r + 1, kinds[r as usize % kinds.len()])
                .unwrap();
            g.set_u64(TABLE + 40 * r + 8, (base + r as f64).to_bits())
                .unwrap();
        }
        rows
    }

    #[test]
    fn cooking_rewrites_each_slot_by_kind_once() {
        let mut g = guest();
        let rows = class(&mut g, 1, 1, &[1, 2], &[0, 2, 5, 4, 3], 10.5);
        assert_eq!(rows, 5, "four in the grouped loop, one in the tail");
        cook_class(&mut g, CLASS).unwrap();
        let slot = |g: &Guest, r: u32| {
            (
                g.u32(TABLE + 40 * r + 8).unwrap(),
                g.u32(TABLE + 40 * r + 12).unwrap(),
            )
        };
        assert_eq!(slot(&g, 0), (TAG_SINGLE, 10.5f32.to_bits()));
        assert_eq!(
            g.u64(TABLE + 48).unwrap(),
            11.5f64.to_bits(),
            "kind 2 keeps the double"
        );
        assert_eq!(slot(&g, 2), (TAG_INTEGER, 12));
        assert_eq!(slot(&g, 3), (TAG_POINTER, 0));
        assert_eq!(slot(&g, 4), (TAG_STRING, 0));
        assert_eq!(g.u8(CLASS + 46).unwrap(), 1);
        g.set_u64(TABLE + 8, 1.0f64.to_bits()).unwrap();
        cook_class(&mut g, CLASS).unwrap();
        assert_eq!(
            g.u64(TABLE + 8).unwrap(),
            1.0f64.to_bits(),
            "the flag stops a second pass"
        );
    }

    #[test]
    fn a_class_with_one_parameter_cooks_its_tail_rows() {
        let mut g = guest();
        class(&mut g, 0, 2, &[1], &[1], 0.25);
        cook_class(&mut g, CLASS).unwrap();
        for r in 0..3 {
            assert_eq!(g.u32(TABLE + 40 * r + 8).unwrap(), TAG_SINGLE, "row {r}");
        }
        assert_eq!(g.u32(TABLE + 40 * 3 + 8).unwrap(), 0, "and no further");
    }

    #[test]
    fn defaults_start_after_the_fixed_rows_and_earlier_parameters() {
        let counts = [1, 2, 3, 1];
        for (index, first_row) in [(0u32, 2u32), (1, 3), (2, 5), (3, 8)] {
            let mut g = guest();
            class(&mut g, 1, 1, &counts, &[2], 100.0);
            class_defaults(&mut g, CLASS, index, OUT).unwrap();
            let n = counts[index as usize];
            for k in 0..n {
                assert_eq!(
                    g.u64(OUT + 8 * k).unwrap(),
                    (100.0 + (first_row + k) as f64).to_bits(),
                    "parameter {index} word {k}"
                );
            }
            assert_eq!(
                g.u64(OUT + 8 * n).unwrap(),
                0,
                "parameter {index} copies {n}"
            );
        }
    }

    #[test]
    fn a_class_is_registered_once_by_id() {
        let mut g = guest();
        let registry = MEM + 0x40;
        let (a, b) = (MEM + 0x1000, MEM + 0x1100);
        for c in [a, b] {
            g.set_u8(c + 46, 1).unwrap();
            g.set_u32(c + 36, 0x1234).unwrap();
        }
        assert_eq!(register_class(&mut g, registry, a).unwrap(), a);
        assert_eq!(
            register_class(&mut g, registry, b).unwrap(),
            a,
            "same id: the first"
        );
        assert_eq!(
            [
                g.u32(registry).unwrap(),
                g.u32(registry + 4).unwrap(),
                g.u32(registry + 8).unwrap()
            ],
            [a + 32, a + 32, 1]
        );
    }

    #[test]
    fn the_device_classes_fill_their_slots_and_find_send_by_id() {
        let mut g = guest();
        g.put(SYSTEM, SYS.to_be_bytes().to_vec());
        g.put(REGISTERED, vec![0; 64]);
        g.put(DEFAULT_BUS, vec![0; 4]);
        g.put(BUS_ROOT, (MEM + 0x900).to_be_bytes().to_vec());
        g.set_u32(MEM + 0x900 + 44, MEM + 0x940).unwrap();
        g.set_u32(MEM + 0x940, 0xB0B0_0001).unwrap();
        for (i, (_, class, _)) in CLASSES.iter().enumerate() {
            let mut bytes = vec![0u8; 48];
            bytes[46] = 1; // already cooked
            bytes[36..40].copy_from_slice(&(0x100 + i as u32).to_be_bytes());
            g.put(*class, bytes);
        }
        g.put(SECOND_LIST_ITEM, vec![0; 24]);
        // Send was registered earlier, by another path.
        let send = MEM + 0x1200;
        g.set_u32(send + 36, SEND_ID).unwrap();
        let mut heap = BumpHeap {
            next: MEM + 0x1800,
            end: MEM + 0x2000,
        };
        let registry = class_registry(&mut g, &mut heap, SYS).unwrap();
        g.set_u8(send + 46, 1).unwrap();
        register_class(&mut g, registry, send).unwrap();

        register_voice_classes(&mut g, &mut heap).unwrap();
        assert_eq!(g.u32(REGISTRY_SLOT).unwrap(), registry);
        for (slot, class, name) in CLASSES {
            assert_eq!(g.u32(slot).unwrap(), class, "{name}");
        }
        assert_eq!(g.u32(SEND_SLOT).unwrap(), send);
        assert_eq!(g.u32(DEFAULT_BUS).unwrap(), 0xB0B0_0001);
        assert_eq!(g.u32(registry + 8).unwrap(), 9);
        let list = g.u32(SYS + 60).unwrap();
        assert_eq!(
            [
                g.u32(list).unwrap(),
                g.u32(list + 8).unwrap(),
                g.u32(list + 12).unwrap()
            ],
            [SECOND_LIST_ITEM + 16, 1, SYS]
        );
        assert_eq!(g.u8(REGISTERED).unwrap(), 1);
    }
}
