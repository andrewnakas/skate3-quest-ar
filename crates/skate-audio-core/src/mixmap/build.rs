//! Building the MixMap host from `MixMapSK8.mxb`: the manager's init, the host constructor, the
//! sizing pass, the allocations, the per-instance record builders, the reference expansion, the
//! controller count and the key resolution that wires every stage's inputs to pointers.
//!
//! Call order, as `sub_82484FE8` → `sub_8294B918`, `sub_8294BA18` run it:
//!
//! ```text
//! sub_8294B918  manager init: descriptor X (128 B) at mgr+8, idle object Y (28 B) at mgr+28
//! sub_8294BA18  host (564 B, vtable 0x82316B78) at X+0
//!   sub_8294BDC8
//!     host vfunc4 = sub_8294CC48 → sub_8294CDE8 (sizing; sub_8294CC90 zeroes the counters)
//!     sub_8294D438  allocations
//!     sub_8294E008  per slot × instance: 96-byte instance (sub_82951DA0), then
//!                   sub_82951E00 (A: input products; sub_8294DD20, sub_8294DE10)
//!                   sub_82952DD8 (B: position lookups)
//!                   sub_829528A0 (F: envelopes; sub_8294DF08)
//!     sub_8294E970  size of the 64-byte block pool (host+380)
//!     sub_829524F0 (C: sums) and sub_82952680 (E: outputs) per instance
//!     sub_8294F528  sub_82953080 / sub_829531F8 reference expansion,
//!                   sub_8294E120 controller count and array,
//!                   sub_8294F228 controllers per output (sub_8294CB48) and every key → pointer
//!                   (sub_8294EDF8)
//!   sub_8294C348  (Y; inert unless Y+4 == 2, which nothing in this load sets)
//! ```
//!
//! Indirect calls resolve by address: the host's own vtable slots and the 96-byte instance's are
//! called directly; the manager's slots 4 and 8 (the game's `sub_82485850` / `sub_82485980`) go
//! through [`Listener`]; the guest allocator (`*0x83084184`, slots 24/28) is [`Heap`].
//!
//! **The allocator does not zero** in retail. Everything these builders read back is written first,
//! with one exception worth naming: `sub_8294E970` walks the input-entry array to its *capacity*
//! (host+208), not to the count `sub_8294DD20` filled. [`check_capacities`] reports whether the two
//! agree for a given file, and for `MixMapSK8.mxb` they do.

use super::Listener;
use super::controller::{CONTROLLER_VTABLE, set_inputs, set_outputs};
use super::offsets as H;
use super::tables::{K_FRAMES_TO_MS, K_ONE, K_ZERO, mb_to_lin};
use crate::fp::{load_single, mul_single, store_single, word_to_single};
use crate::patch::Heap;
use crate::vmx::Fpscr;
use crate::{Error, Guest, Result};

/// The host vtable.
pub const HOST_VTABLE: u32 = 0x8231_6B78;
/// The 96-byte instance vtable (`sub_82951D58` dtor, `sub_82951DA0` init).
pub const INSTANCE_VTABLE: u32 = 0x8231_6B84;
/// `lis -31992` + 16780: the word an unresolved input key points at.
pub const NULL_INPUT: u32 = 0x8308_418C;
/// `lis -31992` + 16776: `sub_8294B918` publishes the descriptor here.
pub const DESCRIPTOR_GLOBAL: u32 = 0x8308_4188;

/// The key mask every controller comparison uses: `rlwinm 0,0,27 ; rlwinm 0,8,2`.
pub const KEY_MASK: u32 = 0xE0FF_FFF0;

#[inline]
fn rd(g: &Guest, a: u32) -> Result<u32> {
    g.u32(a)
}
#[inline]
fn rdi(g: &Guest, a: u32) -> Result<i32> {
    Ok(g.u32(a)? as i32)
}
#[inline]
fn wr(g: &mut Guest, a: u32, v: u32) -> Result<()> {
    g.set_u32(a, v)
}
#[inline]
fn bump(g: &mut Guest, a: u32) -> Result<u32> {
    let v = g.u32(a)?;
    g.set_u32(a, v.wrapping_add(1))?;
    Ok(v)
}
/// `host[(slot + 2) × 4]`: the instance count of a slot (host+8+4·slot).
#[inline]
fn instances(g: &Guest, h: u32, slot: u32) -> Result<u32> {
    g.u32(h.wrapping_add(slot.wrapping_add(2) << 2))
}
fn alloc(g: &mut Guest, heap: &mut dyn Heap, size: u32) -> Result<u32> {
    let at = heap.alloc(g, size, 16)?;
    if at == 0 && size != 0 {
        return Err(Error::new(
            0x825E_7458,
            format!("guest allocator out of memory ({size} B)"),
        ));
    }
    Ok(at)
}

// ---------------------------------------------------------------------- manager and host

/// `sub_8294B918(mgr, params)` — the manager's init. `slot_count` is the params' `+4` (14 in
/// `sub_82484FE8`); the params' allocator (`+8`, stored to `0x83084184`) is [`Heap`] here.
pub fn init_manager(g: &mut Guest, heap: &mut dyn Heap, mgr: u32, slot_count: u32) -> Result<()> {
    let x = alloc(g, heap, 128)?;
    wr(g, DESCRIPTOR_GLOBAL, x)?;
    for i in 0..25 {
        wr(g, x + 12 + 4 * i, 0)?; // stwu r9,4(r11) × 25 from X+8
    }
    wr(g, x + 4, 0)?;
    wr(g, x, 0)?;
    wr(g, x + 8, 0)?;
    g.set_u8(x + 112, 0)?;
    wr(g, x + 124, 0)?;
    wr(g, mgr + 8, x)?;
    wr(g, x + 8, slot_count)?;
    let y = alloc(g, heap, 28)?;
    for i in 0..7 {
        wr(g, y + 4 * i, 0)?;
    }
    wr(g, mgr + 28, y)?;
    wr(g, y, 0)?;
    wr(g, y + 4, 1)?;
    for off in [8, 12, 16, 20] {
        wr(g, y + off, 0)?;
    }
    g.set_u8(y + 24, 0)?;
    Ok(())
}

/// `sub_8294BA18(mgr, data)` — construct the host and build it. Returns the host (0 when the
/// manager has no descriptor, as the original returns without building).
pub fn build_host(
    g: &mut Guest,
    heap: &mut dyn Heap,
    listener: &mut dyn Listener,
    mgr: u32,
    data: u32,
) -> Result<u32> {
    let x = rd(g, mgr + 8)?;
    if x == 0 {
        return Ok(0);
    }
    wr(g, x + 124, x)?;
    g.set_u8(x + 112, 0)?;
    let h = alloc(g, heap, 564)?;
    wr(g, h, HOST_VTABLE)?;
    wr(g, x, h)?;
    wr(g, h + H::DESCRIPTOR, x)?;
    wr(g, h + 152, 0)?;
    wr(g, h + H::INSTANCES_TOTAL, 0)?;
    let one = rd(g, K_ONE)?; // lfs/stfs of 1.0: a bit copy
    wr(g, h + 560, one)?;
    wr(g, h + 556, one)?;
    let slots = rd(g, x + 8)?;
    wr(g, h + 4, slots)?;
    wr(g, x + 116, 0)?;
    wr(g, x + 4, data)?;
    wr(g, h + H::MANAGER, mgr)?;
    build_descriptor(g, heap, listener, x)?;
    let y = rd(g, mgr + 28)?;
    if y != 0 {
        idle_bind(g, y, rd(g, x)?)?;
    }
    Ok(h)
}

/// `sub_8294C348(Y, host)`: only acts when `Y+4 == 2` and `Y+20 != 0`, which requires a second
/// data set nothing in the retail load path installs (Y+4 is 1 from [`init_manager`]).
fn idle_bind(g: &Guest, y: u32, host: u32) -> Result<()> {
    if rd(g, y + 4)? != 2 || host == 0 || rd(g, y + 20)? == 0 {
        return Ok(());
    }
    Err(Error::new(
        0x8294_C348,
        "MixMap Y object in state 2: not ported (never reached by sub_82484FE8)",
    ))
}

/// `sub_8294BDC8(X)`.
fn build_descriptor(
    g: &mut Guest,
    heap: &mut dyn Heap,
    listener: &mut dyn Listener,
    x: u32,
) -> Result<()> {
    let h = rd(g, x)?;
    let data = rd(g, x + 4)?;
    size_host(g, heap, listener, h, data, h)?; // host vfunc4
    allocate(g, heap, h)?;
    let slots = rd(g, x + 8)?;
    for s in 0..slots {
        let data_slots = rd(g, rd(g, h + H::DATA)? + 4)?;
        let v = if (s as i32) < data_slots as i32 {
            rd(g, h + 8 + 4 * s)?
        } else {
            0
        };
        wr(g, x + 12 + 4 * s, v)?;
    }
    for s in 0..rd(g, x + 8)? {
        let count = rd(g, x + 12 + 4 * s)?;
        let mut j = 0u32;
        while (j as i32) < count as i32 {
            build_instance(g, h, s, count, j)?;
            j += 1;
        }
    }
    size_block_pool(g, heap, h)?;
    for s in 0..rd(g, x + 8)? {
        let mut j = 0u32;
        while (j as i32) < rd(g, h + 8 + 4 * s)? as i32 {
            let base = rd(g, rd(g, h + 152)? + 4 * s)?;
            let inst = rd(g, base + 92)?.wrapping_add(96 * j);
            build_sums(g, inst)?;
            build_outputs(g, inst)?;
            j += 1;
        }
    }
    link(g, heap, listener, h)?;
    g.set_u8(x + 112, 1)?;
    Ok(())
}

// ---------------------------------------------------------------------- sizing

/// `sub_8294CC48(host, data, r5)` + `sub_8294CC90` + `sub_8294CDE8`: count every array.
fn size_host(
    g: &mut Guest,
    heap: &mut dyn Heap,
    listener: &mut dyn Listener,
    h: u32,
    data: u32,
    r5: u32,
) -> Result<()> {
    wr(g, h + 140, r5)?;
    wr(g, h + 144, data)?;
    wr(g, h + H::DATA, data)?;
    wr(g, h + 120, rd(g, data)?)?;
    let mut i = 0u32;
    if rdi(g, data + 4)? > 0 {
        loop {
            wr(g, h + 8 + 4 * i, 0)?;
            i += 1;
            if !((i as i32) < rdi(g, rd(g, h + H::DATA)? + 4)?) {
                break;
            }
        }
    }
    // sub_8294CDE8
    let slot_table = data.wrapping_add(rd(g, data + 8)?);
    zero_counters(g, h)?;
    {
        let d = rd(g, h + H::DATA)?;
        let mut entry = d.wrapping_add(rd(g, d + 8)?);
        let mut s = 0u32;
        while (s as i32) < rdi(g, rd(g, h + H::DATA)? + 4)? {
            wr(g, h + 8 + 4 * s, 0)?;
            if rdi(g, entry)? != -1 {
                let n = listener.instance_count(g, s)?; // [host+108] vfunc 8
                wr(g, h + 8 + 4 * s, n)?;
            }
            s += 1;
            entry += 4;
        }
    }
    wr(g, h + H::INSTANCES_TOTAL, 0)?;
    let slots = rdi(g, data + 4)?;
    let mut s: u32 = 0;
    while (s as i32) < slots {
        let entry = slot_table + 4 * s;
        let off = rdi(g, entry)?;
        if off != -1 {
            let sec = data.wrapping_add(off as u32);
            let count = rd(g, h + 8 + 4 * s)?;
            let total = rd(g, h + H::INSTANCES_TOTAL)?.wrapping_add(count);
            wr(g, h + H::INSTANCES_TOTAL, total)?;
            size_section(g, heap, h, sec, s, count)?;
        }
        s += 1;
    }
    let mut total = rd(g, h + H::INPUT_CAP)?;
    for k in 0..10 {
        total = total.wrapping_add(rd(g, h + 220 + 8 * k)?);
        wr(g, h + H::INPUT_CAP, total)?;
    }
    Ok(())
}

/// `sub_8294CC90`.
fn zero_counters(g: &mut Guest, h: u32) -> Result<()> {
    for off in [192, 464, 196, 316, 320, 324, 212, 216] {
        wr(g, h + off, 0)?;
    }
    for k in 0..10 {
        wr(g, h + 220 + 8 * k, 0)?;
        wr(g, h + 224 + 8 * k, 0)?;
    }
    for off in [
        468, 480, 180, 184, 188, 352, 356, 360, 364, 368, 372, 124, 128, 308, 312, 472, 300, 328,
        332, 336, 476, 304, 340, 344, 348, 524, 488, 544, 548, 552, 540, 536, 532, 528, 492, 496,
        500, 504, 452, 456, 460, 484, 200, 204, 208, 508, 512, 156,
    ] {
        wr(g, h + off, 0)?;
    }
    for off in (380..=448).step_by(4) {
        wr(g, h + off, 0)?;
    }
    for off in [516, 520, 376] {
        wr(g, h + off, 0)?;
    }
    Ok(())
}

/// Sum over a reference list of `same slot ? 1 : instances(slot)` — the sizing pass's inner loop.
fn ref_weight(g: &Guest, h: u32, first: u32, n: u32, own: u32) -> Result<u32> {
    let mut sum = 0u32;
    for m in 0..n {
        let s = u32::from(g.u16(first + 4 * m)?) & 0xFF;
        sum = sum.wrapping_add(if s == own { 1 } else { instances(g, h, s)? });
    }
    Ok(sum)
}

fn size_section(
    g: &mut Guest,
    heap: &mut dyn Heap,
    h: u32,
    sec: u32,
    slot: u32,
    count: u32,
) -> Result<()> {
    let add = |g: &mut Guest, off: u32, v: u32| -> Result<()> {
        let cur = g.u32(h + off)?;
        g.set_u32(h + off, cur.wrapping_add(v))
    };
    let a_off = rdi(g, sec + 4)?;
    // A: input products.
    if a_off != -1 {
        let a = sec.wrapping_add(a_off as u32);
        let na = rd(g, a)?;
        let temp = alloc(g, heap, na << 2)?;
        add(g, 464, count.wrapping_mul(na))?;
        add(g, 196, na)?;
        add(g, 460, count.wrapping_mul(rd(g, a + 4)?))?;
        let mut rec = a + 16;
        let mut j = 0u32;
        while (j as i32) < na as i32 {
            let w = rd(g, rec)?;
            let n = u32::from(g.u16(rec + 4)?) & 0xF;
            let kind = (((w as i32) >> 24) as u32) & 0xF;
            wr(g, temp + 4 * j, w)?;
            let mut unique = true;
            for k in 0..j {
                if rd(g, temp + 4 * k)? == w {
                    unique = false;
                }
            }
            if unique {
                add(g, 220 + 8 * kind, count)?;
            }
            let weight = ref_weight(g, h, rec + 8, n, slot)?;
            add(g, 212, count.wrapping_mul(weight))?;
            rec = rec.wrapping_add((n + 2) << 2);
            j += 1;
        }
        heap.free(g, temp)?; // allocator vfunc 28
    }
    // B: position lookups.
    let b_off = rdi(g, sec + 8)?;
    if b_off != -1 {
        let b = sec.wrapping_add(b_off as u32);
        let nb = rd(g, b)? & 0xFF;
        add(g, 468, count.wrapping_mul(nb))?;
        add(g, 308, nb)?;
        // The unrolled walk steps 24 × nibble per record; the builder steps 24 × nibble + 4. Kept.
        let r6 = rdi(g, b)?;
        let mut p = b + 16;
        let (mut r9, mut r8, mut r4, mut r5) = (0u32, 0u32, 0u32, 0i32);
        if r6 >= 2 {
            let iters = ((r6 - 2) as u32 >> 1) + 1;
            r5 = (iters << 1) as i32;
            for _ in 0..iters {
                let a = u32::from(g.u8(p)?) & 0xF;
                r9 = r9.wrapping_add(a);
                p = p.wrapping_add(a * 24);
                let c = u32::from(g.u8(p)?) & 0xF;
                r8 = r8.wrapping_add(c);
                p = p.wrapping_add(c * 24);
            }
        }
        if r5 < r6 {
            r4 = u32::from(g.u8(p)?) & 0xF;
        }
        add(g, 484, r9.wrapping_add(r8).wrapping_add(r4))?;
    }
    // F: envelopes.
    let f_off = rdi(g, sec + 24)?;
    if f_off != -1 {
        let f = sec.wrapping_add(f_off as u32);
        let nf = rd(g, f)?;
        add(g, 480, count.wrapping_mul(nf))?;
        add(g, 312, nf)?;
        let mut sum = 0u32;
        let mut rec = f + 16;
        for _ in 0..(if (nf as i32) > 0 { nf } else { 0 }) {
            let n = u32::from(g.u16(rec + 4)?) & 0xF;
            sum = sum.wrapping_add(ref_weight(g, h, rec + 24, n, slot)?);
            rec = rec.wrapping_add((n + 6) << 2);
        }
        add(g, 212, count.wrapping_mul(sum))?;
    }
    // C: sums.
    let c_off = rdi(g, sec + 12)?;
    if c_off != -1 {
        let c = sec.wrapping_add(c_off as u32);
        let nc = rd(g, c)?;
        add(g, 300, nc)?;
        add(g, 472, count.wrapping_mul(nc))?;
        let mut rec = c + 16;
        for _ in 0..(if (nc as i32) > 0 { nc } else { 0 }) {
            let n = u32::from(g.u16(rec)?) & 0xFF;
            let weight = ref_weight(g, h, rec + 8, n, slot)?;
            add(g, 496, count.wrapping_mul(weight))?;
            rec = rec.wrapping_add(8 + 4 * n);
        }
    }
    // E: outputs.
    let e_off = rdi(g, sec + 16)?;
    if e_off != -1 {
        let e = sec.wrapping_add(e_off as u32);
        let blocks = rd(g, e + 4)?;
        add(g, 504, blocks.wrapping_mul(count).wrapping_add(blocks))?;
        let ne = rd(g, e)?;
        add(g, 304, ne)?;
        add(g, 476, count.wrapping_mul(ne))?;
        let mut rec = e + 16;
        for _ in 0..(if (ne as i32) > 0 { ne } else { 0 }) {
            let n = u32::from(g.u16(rec)?) & 0xFF;
            let weight = ref_weight(g, h, rec + 12, n, slot)?;
            add(g, 488, count.wrapping_mul(weight))?;
            rec = rec.wrapping_add(12 + 4 * n);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------- allocation

/// `sub_8294D438`, in the original's order.
fn allocate(g: &mut Guest, heap: &mut dyn Heap, h: u32) -> Result<()> {
    if rd(g, h + 152)? == 0 {
        let n = rd(g, rd(g, h + H::DATA)? + 4)?;
        let at = alloc(g, heap, n << 2)?;
        wr(g, h + 152, at)?;
    }
    let mut i = 0u32;
    while (i as i32) < rdi(g, rd(g, h + H::DATA)? + 4)? {
        wr(g, rd(g, h + 152)? + 4 * i, 0)?;
        i += 1;
    }
    let plan: [(u32, u32, u32); 21] = [
        (488, 4, 516),
        (496, 4, 520),
        (504, 64, 376),
        (168, 96, 156),
        (212, 4, 384),
        (208, 16, 388),
        (196, 16, 392),
        (464, 12, 396),
        (464, 8, 400),
        (300, 12, 428),
        (472, 8, 432),
        (472, 8, 436),
        (304, 16, 440),
        (476, 20, 444),
        (476, 8, 448),
        (308, 20, 420),
        (468, 64, 424),
        (468, 8, 416),
        (312, 12, 408),
        (480, 48, 412),
        (480, 8, 404),
    ];
    for (count_off, stride, dest) in plan {
        let size = rd(g, h + count_off)?.wrapping_mul(stride);
        let at = alloc(g, heap, size)?;
        wr(g, h + dest, at)?;
        if dest == 156 {
            let mut k = 0u32;
            while (k as i32) < rdi(g, h + H::INSTANCES_TOTAL)? {
                wr(g, at + 96 * k, INSTANCE_VTABLE)?;
                k += 1;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------- instances

/// `sub_82951DA0(obj, host, slot, count, j)` — the 96-byte instance's slot 4.
fn instance_init(g: &mut Guest, obj: u32, h: u32, slot: u32, count: u32, j: u32) -> Result<()> {
    wr(g, obj + 4, h)?;
    wr(g, obj + 56, slot)?;
    wr(g, obj + 68, count)?;
    wr(g, obj + 64, j)?;
    if j == 0 {
        return wr(g, obj + 60, 1);
    }
    let base = rd(g, obj + 92)?;
    let v = rd(g, base + 60)?.wrapping_add(1);
    wr(g, base + 60, v)?;
    if (j as i32) <= 0 {
        return Ok(());
    }
    for k in 0..j {
        let base = rd(g, obj + 92)?;
        let v = rd(g, base + 60)?;
        wr(g, base + 96 * k + 60, v)?;
    }
    Ok(())
}

/// `sub_8294E008(host, slot, count, j)`.
fn build_instance(g: &mut Guest, h: u32, slot: u32, count: u32, j: u32) -> Result<()> {
    let table = rd(g, h + 152)?;
    if rd(g, table + 4 * slot)? == 0 {
        let used = rd(g, h + 524)?;
        wr(g, h + 524, used + 96)?;
        let obj = rd(g, h + 156)?.wrapping_add(used);
        wr(g, table + 4 * slot, obj)?;
        instance_init(g, obj, h, slot, count, j)?;
    }
    let base = rd(g, rd(g, h + 152)? + 4 * slot)?;
    wr(g, base + 92, base)?;
    let target = if j == 0 {
        base
    } else {
        let hh = rd(g, base + 4)?;
        let used = rd(g, hh + 524)?;
        wr(g, hh + 524, used + 96)?;
        let t = rd(g, hh + 156)?.wrapping_add(used);
        wr(g, t + 92, base)?;
        t
    };
    let (hh, s) = (rd(g, base + 4)?, rd(g, base + 56)?);
    instance_init(g, target, hh, s, 1, j)?;
    let data = rd(g, h + H::DATA)?;
    let inst = rd(g, rd(g, rd(g, h + 152)? + 4 * slot)? + 92)?.wrapping_add(96 * j);
    let sec = data.wrapping_add(rd(g, data.wrapping_add(rd(g, data + 8)?) + 4 * slot)?);
    wr(g, inst + 28, sec)?;
    build_products(g, inst)?;
    build_lookups(g, inst)?;
    build_envelopes(g, inst)?;
    Ok(())
}

/// `sub_8294DD20(host, key)` — find or append the input entry `{key, ptr, mB, lin}` for a key, in
/// its kind's run of the 16-byte input array. Returns the entry (0 without an array).
fn input_entry(g: &mut Guest, h: u32, key: u32) -> Result<u32> {
    let arr = rd(g, h + 388)?;
    if arr == 0 {
        return Ok(0);
    }
    let kind = (((key as i32) >> 24) as u32) & 0xF;
    let mut before = 0u32;
    for m in 0..kind {
        before = before.wrapping_add(rd(g, h + 220 + 8 * m)?);
    }
    let start = arr.wrapping_add(before << 4);
    let used_at = h + 224 + 8 * kind;
    let used = rdi(g, used_at)?;
    let mut m = 0i32;
    while m < used {
        let e = start.wrapping_add((m as u32) << 4);
        if rd(g, e)? == key {
            return Ok(e);
        }
        m += 1;
        if !(m < rdi(g, used_at)?) {
            break;
        }
    }
    let e = start.wrapping_add((used.max(0) as u32) << 4);
    wr(g, used_at, (used + 1) as u32)?;
    wr(g, e, key)?;
    wr(g, e + 4, 0)?;
    wr(g, e + 8, 0)?;
    wr(g, e + 12, 32767)?;
    Ok(e)
}

/// `sub_8294DE10` (refs at `rec+8`) and `sub_8294DF08` (refs at `rec+24`, with an early exit when
/// the reference array is full): expand a record's references over the referenced slot's
/// instances into host+384, write the expanded count into `[rec+4]`'s top byte, return the start.
fn expand_refs(
    g: &mut Guest,
    h: u32,
    rec: u32,
    j: u32,
    first: u32,
    full_check: bool,
) -> Result<u32> {
    if full_check && rd(g, h + 216)? == rd(g, h + 212)? {
        return Ok(0);
    }
    let n = u32::from(g.u16(rec + 4)?) & 0x1F;
    if n == 0 {
        return Ok(0);
    }
    let arr = rd(g, h + 384)?;
    let start = arr.wrapping_add(rd(g, h + 216)? << 2);
    let own = u32::from(g.u16(rec)?) & 0xFF;
    let mut r9 = 0u32;
    for m in 0..n {
        let w = rd(g, first + 4 * m)?;
        let s = (((w as i32) >> 16) as u32) & 0xFF;
        if s == own {
            let at = rd(g, h + 384)?.wrapping_add(rd(g, h + 216)?.wrapping_add(r9) << 2);
            wr(g, at, (j << 11) | w)?;
            r9 += 1;
        } else {
            let mut i = 0u32;
            while (i as i32) < instances(g, h, s)? as i32 {
                let at = rd(g, h + 384)?.wrapping_add(rd(g, h + 216)?.wrapping_add(r9) << 2);
                wr(g, at, (i << 11) | w)?;
                r9 += 1;
                i += 1;
            }
        }
    }
    let w = rd(g, rec + 4)?;
    wr(g, rec + 4, (w & 0x00FF_FFFF) | (r9 << 24))?;
    let v = rd(g, h + 216)?.wrapping_add(r9);
    wr(g, h + 216, v)?;
    Ok(start)
}

/// The constant-depth word both builders compute from a record's `+4` half: bit 15 clear → the low
/// 15 bits are an attenuation `v` (stored, and converted from `-v`); set → the half sign-extended.
/// Returns `(stored, 32767 - mb_to_lin(x))`.
fn depth(g: &Guest, w: u32) -> Result<(Option<u32>, u32)> {
    if w & 0x8000 == 0 {
        let v = w & 0x7FFF;
        let r = mb_to_lin(g, (v as i32).wrapping_neg())?;
        Ok((Some(v), 32767i32.wrapping_sub(r) as u32))
    } else {
        let r = mb_to_lin(g, (w | 0xFFFF_0000) as i32)?;
        Ok((None, 32767i32.wrapping_sub(r) as u32))
    }
}

/// The instance-independent key the A builder files its 16-byte constants under.
fn product_key(w0: u32, data_word0: u32, index: u32) -> u32 {
    ((((w0 as i32) >> 16) as u32) & 0xE000)
        | ((data_word0 << 8) & 0xFFFF_FF00)
        | (w0 & 0x0FFF_0000)
        | index
}

/// `sub_82951E00(inst)` — A records: one product per record, fed by an input entry.
fn build_products(g: &mut Guest, inst: u32) -> Result<()> {
    let sec = rd(g, inst + 28)?;
    wr(g, inst + 72, 0)?;
    let off = rdi(g, sec + 4)?;
    if off < 0 {
        return Ok(());
    }
    let a = sec.wrapping_add(off as u32);
    wr(g, inst + 32, a)?;
    if rdi(g, a)? <= 0 {
        return Ok(());
    }
    let h = rd(g, inst + 4)?;
    wr(
        g,
        inst + 8,
        rd(g, h + 400)?.wrapping_add(rd(g, h + 316)? << 3),
    )?;
    let mut rec = a + 16;
    let mut k = 0u32;
    loop {
        let w0 = rd(g, rec)?;
        let j = rd(g, inst + 64)?;
        let r25 = rd(g, rec + 4)?;
        let key = (w0 & 0xFFFF_07FF) | ((j << 11) & 0xFFFF_F800);
        let entry = input_entry(g, h, key)?;
        let refs = expand_refs(g, h, rec, j, rec + 8, false)?;
        let idx = bump(g, h + 316)?;
        let e = rd(g, h + 400)?.wrapping_add(idx << 3);
        let data0 = rd(g, h + 120)?;
        if j == 0 {
            let n = bump(g, h + 320)?;
            wr(g, e, rd(g, h + 392)?.wrapping_add(n << 4))?;
        } else {
            let key2 = product_key(w0, data0, k);
            let mut m = 0u32;
            while (m as i32) < rdi(g, h + 320)? {
                let d = rd(g, h + 392)?.wrapping_add(m << 4);
                if rd(g, d + 4)? == key2 {
                    wr(g, e, d)?;
                }
                m += 1;
            }
        }
        let n2 = bump(g, h + 324)?;
        wr(g, e + 4, rd(g, h + 396)?.wrapping_add(n2 * 12))?;
        let def = rd(g, e)?;
        wr(g, def + 4, product_key(rd(g, rec)?, rd(g, h + 120)?, k))?;
        wr(g, def, rec)?;
        wr(g, def + 8, 0)?;
        wr(g, def + 12, 0)?;
        let (stored, d) = depth(g, rd(g, rec + 4)?)?;
        if let Some(v) = stored {
            wr(g, def + 8, v)?;
        }
        wr(g, def + 12, d)?;
        let e2 = rd(g, e + 4)?;
        wr(g, e2 + 8, 0)?;
        wr(g, e2, entry)?;
        wr(g, e2 + 4, refs)?;
        k += 1;
        rec = rec.wrapping_add(((((r25 as i32) >> 16) as u32 & 0x1F) + 2) << 2);
        let c = rd(g, inst + 72)?.wrapping_add(1);
        wr(g, inst + 72, c)?;
        if !((k as i32) < rdi(g, rd(g, inst + 32)?)?) {
            break;
        }
    }
    Ok(())
}

/// `sub_82952DD8(inst)` — B records: a 20-byte variant pointer, an 8-byte entry, a 64-byte state.
fn build_lookups(g: &mut Guest, inst: u32) -> Result<()> {
    let _fpscr = Fpscr::capture();
    let sec = rd(g, inst + 28)?;
    wr(g, inst + 76, 0)?;
    let off = rdi(g, sec + 8)?;
    if off < 0 {
        return Ok(());
    }
    let b = sec.wrapping_add(off as u32);
    wr(g, inst + 36, b)?;
    if rdi(g, b)? <= 0 {
        return Ok(());
    }
    let h = rd(g, inst + 4)?;
    wr(
        g,
        inst + 20,
        rd(g, h + 416)?.wrapping_add(rd(g, h + 352)? << 3),
    )?;
    let one = rd(g, K_ONE)?;
    let mut rec = b + 16;
    let mut k = 0u32;
    loop {
        let j = rd(g, inst + 64)?;
        let def = if j != 0 {
            rd(g, rd(g, rd(g, inst + 92)? + 20)?.wrapping_add(k << 3))?
        } else {
            let n = bump(g, h + 356)?;
            let d = rd(g, h + 420)?.wrapping_add(n * 20);
            wr(g, d, rec)?;
            wr(g, d + 16, 0)?;
            wr(g, d + 8, 0)?;
            wr(g, d + 12, 0)?;
            d
        };
        let e = rd(g, h + 416)?.wrapping_add(bump(g, h + 352)? << 3);
        let st = rd(g, h + 424)?.wrapping_add(bump(g, h + 360)? << 6);
        wr(g, e, def)?;
        wr(g, def + 4, rec + 4)?;
        wr(g, e + 4, st)?;
        wr(g, st + 8, 0)?;
        wr(g, st + 12, 0)?;
        wr(g, st + 16, 32767)?;
        wr(g, st + 20, 0)?;
        wr(g, st + 28, one)?;
        wr(g, st + 24, one)?;
        let r = rd(g, def)?;
        let key = rd(g, r)?;
        wr(g, st, (key & 0xFFFF_07FF) | ((j << 11) & 0xFFFF_F800))?;
        let p = rd(g, def + 4)?;
        for (i, at) in [8u32, 12, 16, 20].into_iter().enumerate() {
            let lo = rd(g, p + at)? & 0x7FFF;
            let hi = u32::from(g.u16(p + at)?) & 0x7FFF;
            store_single(g, st + 32 + 8 * i as u32, word_to_single(lo))?;
            store_single(g, st + 36 + 8 * i as u32, word_to_single(hi))?;
        }
        wr(
            g,
            st + 4,
            0x6000_0000 | ((j << 11) & 0x1FFF_F800) | (key & 0x1FFF_FFFF),
        )?;
        let c = rd(g, inst + 76)?.wrapping_add(1);
        wr(g, inst + 76, c)?;
        k += 1;
        let n = u32::from(g.u8(rec)?) & 0xF;
        rec = rec.wrapping_add(n * 24 + 4);
        if !((k as i32) < rdi(g, rd(g, inst + 36)?)?) {
            break;
        }
    }
    Ok(())
}

/// `sub_829528A0(inst)` — F records: the envelopes.
fn build_envelopes(g: &mut Guest, inst: u32) -> Result<()> {
    let sec = rd(g, inst + 28)?;
    wr(g, inst + 80, 0)?;
    let off = rdi(g, sec + 24)?;
    if off < 0 {
        return Ok(());
    }
    let f = sec.wrapping_add(off as u32);
    wr(g, inst + 40, f)?;
    if rdi(g, f)? <= 0 {
        return Ok(());
    }
    let h = rd(g, inst + 4)?;
    wr(
        g,
        inst + 24,
        rd(g, h + 404)?.wrapping_add(rd(g, h + 364)? << 3),
    )?;
    let zero = rd(g, K_ZERO)?;
    let mut rec = f + 16;
    let mut k = 0u32;
    let fix = |g: &mut Guest, at: u32| -> Result<()> {
        let v = g.u32(at)?;
        if v & 0xFFF == 0 {
            g.set_u32(at, v | 1)?;
        }
        Ok(())
    };
    loop {
        let j = rd(g, inst + 64)?;
        let def = if j != 0 {
            rd(g, rd(g, rd(g, inst + 92)? + 24)?.wrapping_add(k << 3))?
        } else {
            let n = bump(g, h + 368)?;
            let d = rd(g, h + 408)?.wrapping_add(n * 12);
            wr(g, d, rec)?;
            d
        };
        let e = rd(g, h + 404)?.wrapping_add(bump(g, h + 364)? << 3);
        let n = rd(g, h + 372)?;
        let st = rd(g, h + 412)?.wrapping_add(n * 48);
        wr(g, st, 0)?;
        wr(g, st + 4, zero)?;
        wr(g, st + 8, 0)?;
        wr(g, st + 12, zero)?;
        wr(g, st + 28, 0)?;
        wr(g, st + 24, 0)?;
        wr(g, h + 372, n + 1)?;
        wr(g, e, def)?;
        let r = rd(g, def)?;
        let kind = u32::from(g.u8(r)?) & 0xF;
        match kind {
            1 => {
                fix(g, r + 12)?;
                fix(g, r + 16)?;
                fix(g, r + 20)?;
            }
            0 | 2 => {
                fix(g, r + 12)?;
                fix(g, r + 20)?;
            }
            _ => {}
        }
        wr(g, def + 4, 0)?;
        wr(g, def + 8, 0)?;
        let (stored, d) = depth(g, rd(g, r + 4)?)?;
        if let Some(v) = stored {
            wr(g, def + 4, v)?;
        }
        wr(g, def + 8, d)?;
        wr(g, e + 4, st)?;
        wr(g, st + 4, zero)?;
        wr(g, st + 12, zero)?;
        wr(g, st + 8, 0)?;
        wr(g, st + 28, 0)?;
        wr(g, st, 0)?;
        wr(g, st + 24, 0)?;
        wr(g, st + 16, (j << 11) | rd(g, r + 8)?)?;
        let refs = expand_refs(g, h, r, j, r + 24, true)?;
        wr(g, st + 20, refs)?;
        let c = rd(g, inst + 80)?.wrapping_add(1);
        wr(g, inst + 80, c)?;
        k += 1;
        let n = u32::from(g.u16(rd(g, def)? + 4)?) & 0xF;
        rec = rec.wrapping_add((n + 6) << 2);
        if !((k as i32) < rdi(g, rd(g, inst + 40)?)?) {
            break;
        }
    }
    Ok(())
}

/// The internal-node key both the C and E builders give a record.
fn node_key(w0: u32, index: u32) -> u32 {
    0x2000_0000 | ((w0 & 0xFF00) << 8) | (w0 & 0x1000_0000) | index
}

/// `sub_829524F0(inst)` — C records: clamped sums.
fn build_sums(g: &mut Guest, inst: u32) -> Result<()> {
    let sec = rd(g, inst + 28)?;
    let off = rdi(g, sec + 12)?;
    wr(g, inst + 84, 0)?;
    if off < 0 {
        return Ok(());
    }
    let c = sec.wrapping_add(off as u32);
    wr(g, inst + 84, 0)?;
    wr(g, inst + 48, c)?;
    if rdi(g, c)? <= 0 {
        return Ok(());
    }
    let h = rd(g, inst + 4)?;
    wr(
        g,
        inst + 12,
        rd(g, h + 436)?.wrapping_add(rd(g, h + 328)? << 3),
    )?;
    let mut rec = c + 16;
    let mut k = 0u32;
    loop {
        let j = rd(g, inst + 64)?;
        let (def, e, s) = if j != 0 {
            let def = rd(g, rd(g, rd(g, inst + 92)? + 12)?.wrapping_add(k << 3))?;
            let e = rd(g, h + 436)?.wrapping_add(bump(g, h + 328)? << 3);
            let s = rd(g, h + 432)?.wrapping_add(bump(g, h + 336)? << 3);
            (def, e, s)
        } else {
            let def = rd(g, h + 428)?.wrapping_add(bump(g, h + 332)? * 12);
            let e = rd(g, h + 436)?.wrapping_add(bump(g, h + 328)? << 3);
            let s = rd(g, h + 432)?.wrapping_add(bump(g, h + 336)? << 3);
            let w0 = rd(g, rec)?;
            wr(g, def + 4, node_key(w0, k))?;
            wr(g, def + 8, u32::from(g.u16(rec)?) & 0xFF)?;
            wr(g, def, rec)?;
            (def, e, s)
        };
        wr(g, e, def)?;
        wr(g, s, 0)?;
        wr(g, s + 4, 0)?;
        wr(g, e + 4, s)?;
        k += 1;
        let cnt = rd(g, inst + 84)?.wrapping_add(1);
        wr(g, inst + 84, cnt)?;
        let n = u32::from(g.u16(rec)?) & 0xFF;
        rec = rec.wrapping_add((n + 2) << 2);
        if !((k as i32) < rdi(g, rd(g, inst + 48)?)?) {
            break;
        }
    }
    Ok(())
}

/// `sub_82952680(inst)` — E records: outputs, each into a 64-byte block of host+376 (a new block
/// whenever the destination key changes).
fn build_outputs(g: &mut Guest, inst: u32) -> Result<()> {
    let sec = rd(g, inst + 28)?;
    wr(g, inst + 88, 0)?;
    let off = rdi(g, sec + 16)?;
    if off < 0 {
        return Ok(());
    }
    let e_tab = sec.wrapping_add(off as u32);
    let h = rd(g, inst + 4)?;
    wr(g, inst + 44, e_tab)?;
    let old = rd(g, h + 552)?;
    wr(g, h + 552, old.wrapping_add(rd(g, e_tab + 4)? << 4))?;
    wr(g, inst + 88, 0)?;
    wr(g, inst + 52, rd(g, h + 376)?.wrapping_add(old << 2))?;
    if rdi(g, e_tab)? <= 0 {
        return Ok(());
    }
    let mut rec = e_tab + 16;
    let (mut prev, mut block_off, mut k) = (0u32, 0u32, 0u32);
    wr(
        g,
        inst + 16,
        rd(g, h + 448)?.wrapping_add(rd(g, h + 340)? << 3),
    )?;
    loop {
        let j = rd(g, inst + 64)?;
        let (def, e, st) = if j != 0 {
            let def = rd(g, rd(g, rd(g, inst + 92)? + 16)?.wrapping_add(k << 3))?;
            let e = rd(g, h + 448)?.wrapping_add(bump(g, h + 340)? << 3);
            let st = rd(g, h + 444)?.wrapping_add(bump(g, h + 348)? * 20);
            (def, e, st)
        } else {
            let def = rd(g, h + 440)?.wrapping_add(bump(g, h + 344)? << 4);
            let e = rd(g, h + 448)?.wrapping_add(bump(g, h + 340)? << 3);
            let st = rd(g, h + 444)?.wrapping_add(bump(g, h + 348)? * 20);
            wr(g, def, rec)?;
            let w0 = rd(g, rec)?;
            wr(g, def + 12, 0)?;
            wr(g, def + 4, node_key(w0, k))?;
            wr(g, def + 8, u32::from(g.u16(rec)?) & 0xFF)?;
            (def, e, st)
        };
        wr(g, e, def)?;
        wr(g, st + 8, (-10000i32) as u32)?;
        wr(g, st + 12, 0)?;
        wr(g, st + 4, 0)?;
        let r = rd(g, def)?;
        let dest = rd(g, r + 8)?;
        wr(g, st, (j << 11) | dest)?;
        let base = rd(g, inst + 52)?.wrapping_add(block_off);
        let block = if prev == dest {
            base.wrapping_sub(64)
        } else {
            block_off += 64;
            base
        };
        wr(g, st + 16, block)?;
        k += 1;
        wr(g, e + 4, st)?;
        prev = rd(g, rd(g, def)? + 8)?;
        let c = rd(g, inst + 88)?.wrapping_add(1);
        wr(g, inst + 88, c)?;
        let n = u32::from(g.u16(rec)?) & 0xFF;
        rec = rec.wrapping_add((n + 3) << 2);
        if !((k as i32) < rdi(g, rd(g, inst + 44)?)?) {
            break;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------- block pool size

/// Whether `key & KEY_MASK` is absent from the reference array.
fn not_in_refs(g: &Guest, h: u32, count: u32, key: u32) -> Result<bool> {
    let refs = rd(g, h + 384)?;
    for m in 0..count {
        if rd(g, refs + 4 * m)? & KEY_MASK == key {
            return Ok(false);
        }
    }
    Ok(true)
}

fn controller_top(top: u32) -> bool {
    top == 0x6000_0000 || top == 0x4000_0000 || top == 0x8000_0000
}

/// `sub_8294E970` — count the distinct keys that need a 64-byte block and allocate host+380.
fn size_block_pool(g: &mut Guest, heap: &mut dyn Heap, h: u32) -> Result<()> {
    let inputs = rd(g, h + 388)?;
    let n_in = rdi(g, h + H::INPUT_CAP)?;
    let mut r21 = 0u32;
    for i in 0..n_in.max(0) as u32 {
        let key = rd(g, inputs + 16 * i)?;
        let mut unique = controller_top(key & 0xE000_0000);
        if unique {
            let k = key & KEY_MASK;
            for m in 0..i {
                if rd(g, inputs + 16 * m)? & KEY_MASK == k {
                    unique = false;
                }
            }
        }
        if unique {
            r21 += 1;
        }
    }
    let n_refs = rd(g, h + 212)?;
    let refs = rd(g, h + 384)?;
    let mut r23 = 0u32;
    for i in 0..(n_refs as i32).max(0) as u32 {
        let w = rd(g, refs + 4 * i)?;
        let mut unique = controller_top(w & 0xE000_0000);
        if unique {
            for m in 0..i {
                if rd(g, refs + 4 * m)? & KEY_MASK == w & KEY_MASK {
                    unique = false;
                }
            }
        }
        if unique {
            r23 += 1;
        }
    }
    let n_b = rd(g, h + 468)?;
    let b = rd(g, h + 416)?;
    let b_key = |g: &Guest, i: u32| -> Result<u32> { g.u32(g.u32(b + 8 * i + 4)?) };
    let mut r26 = 0u32;
    for i in 0..(n_b as i32).max(0) as u32 {
        let k = b_key(g, i)? & KEY_MASK;
        let mut unique = true;
        for m in 0..i {
            if b_key(g, m)? & KEY_MASK == k {
                unique = false;
            }
        }
        if unique && (n_refs as i32) > 0 {
            unique = not_in_refs(g, h, rd(g, h + 212)?, k)?;
        }
        if unique {
            r26 += 1;
        }
    }
    let mut r31 = 0u32;
    let slots = rd(g, rd(g, h + H::DATA)? + 4)?;
    for s in 0..(slots as i32).max(0) as u32 {
        let base = rd(g, rd(g, h + 152)? + 4 * s)?;
        if base == 0 {
            continue;
        }
        let count = rdi(g, base + 80)?;
        let first = rd(g, base + 92)?;
        let f_key = |g: &Guest, k: u32| -> Result<u32> {
            let k = k & 0xFF;
            let e = if first != 0 && (k as i32) < rdi(g, first + 80)? {
                g.u32(first + 24)?.wrapping_add(k << 3)
            } else {
                0
            };
            Ok(g.u32(g.u32(g.u32(e)?)? + 8)? & KEY_MASK)
        };
        for k in 0..count.max(0) as u32 {
            let key = f_key(g, k)?;
            let mut unique = true;
            for m in 0..k {
                if f_key(g, m)? == key {
                    unique = false;
                }
            }
            if unique && (n_refs as i32) > 0 {
                unique = not_in_refs(g, h, rd(g, h + 212)?, key)?;
            }
            if unique && (n_b as i32) > 0 {
                for m in 0..rd(g, h + 468)? {
                    if b_key(g, m)? == key {
                        unique = false;
                    }
                }
            }
            if unique {
                r31 = r31.wrapping_add(rd(g, rd(g, base + 92)? + 60)?);
            }
        }
    }
    let total = r31.wrapping_add(r26).wrapping_add(r23).wrapping_add(r21);
    let at = alloc(g, heap, total << 6)?;
    wr(g, h + 380, at)?;
    POOL_CAPACITY.with(|c| c.set(total));
    Ok(())
}

thread_local! {
    /// The block pool's capacity from the last [`size_block_pool`], for [`check_capacities`] only.
    static POOL_CAPACITY: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

// ---------------------------------------------------------------------- linking

/// `sub_8294F528(host)`.
fn link(g: &mut Guest, heap: &mut dyn Heap, listener: &mut dyn Listener, h: u32) -> Result<()> {
    let slots = rd(g, rd(g, h + H::DATA)? + 4)?;
    for s in 0..(slots as i32).max(0) as u32 {
        let base = rd(g, rd(g, h + 152)? + 4 * s)?;
        if base == 0 {
            continue;
        }
        let mut i = 0u32;
        while (i as i32) < rdi(g, rd(g, rd(g, h + 152)? + 4 * s)? + 60)? {
            let b = rd(g, rd(g, h + 152)? + 4 * s)?;
            expand_sum_refs(g, rd(g, b + 92)?.wrapping_add(96 * i))?;
            let b = rd(g, rd(g, h + 152)? + 4 * s)?;
            expand_output_refs(g, rd(g, b + 92)?.wrapping_add(96 * i))?;
            i += 1;
        }
    }
    count_controllers(g, heap, h)?;
    resolve_all(g, listener, h)
}

/// The shared expansion loop of `sub_82953080` / `sub_829531F8`: write `total` pointers-to-be from
/// the words at `words`, one per same-slot reference, one per instance of a foreign slot.
fn expand_into(g: &mut Guest, inst: u32, mut words: u32, mut out: u32, total: u32) -> Result<u32> {
    let h = rd(g, inst + 4)?;
    let own = rd(g, inst + 56)?;
    let j = rd(g, inst + 64)?;
    let mut r9 = 0i32;
    if (total as i32) <= 0 {
        return Ok(words);
    }
    loop {
        let w = rd(g, words)?;
        words += 4;
        let s = (((w as i32) >> 16) as u32) & 0xFF;
        let r7 = w & 0xFFFF_07FF;
        if s == own {
            wr(g, out, ((j << 11) & 0xFFFF_F800) | r7)?;
            out += 4;
        } else {
            r9 -= 1;
            let cnt = instances(g, h, s)? as i32;
            if cnt > 0 {
                r9 += cnt;
                for i in 0..cnt as u32 {
                    wr(g, out, ((i << 11) & 0xFFFF_F800) | r7)?;
                    out += 4;
                }
            }
        }
        r9 += 1;
        if !(r9 < total as i32) {
            break;
        }
    }
    Ok(words)
}

/// `sub_82953080(inst)` — expand each sum's references into host+520.
fn expand_sum_refs(g: &mut Guest, inst: u32) -> Result<()> {
    let h = rd(g, inst + 4)?;
    let own = rd(g, inst + 56)?;
    let mut k = 0u32;
    while (k as i32) < rdi(g, inst + 84)? {
        let e = rd(g, inst + 12)?.wrapping_add(k << 3);
        let def = rd(g, e)?;
        let rec = rd(g, def)?;
        let n = u32::from(g.u16(rec)?) & 0xFF;
        let mut total = 0u32;
        for m in 0..n {
            let s = (((rd(g, rec + 8 + 4 * m)? as i32) >> 16) as u32) & 0xFF;
            total = total.wrapping_add(if s == own { 1 } else { instances(g, h, s)? });
        }
        let used = rd(g, h + 548)?;
        wr(g, h + 548, used.wrapping_add(total))?;
        let ptr = rd(g, h + 520)?.wrapping_add(used << 2);
        wr(g, rd(g, e + 4)?, ptr)?;
        wr(g, def + 8, total)?;
        let out = rd(g, rd(g, e + 4)?)?;
        expand_into(g, inst, rec + 8, out, total)?;
        k += 1;
    }
    Ok(())
}

/// `sub_829531F8(inst)` — expand each output's references: `0x80000000` keys first into
/// `st+12`, the rest into `st+4`, both in host+516; `def+12` gets the section's G descriptor.
fn expand_output_refs(g: &mut Guest, inst: u32) -> Result<()> {
    let h = rd(g, inst + 4)?;
    let own = rd(g, inst + 56)?;
    let j = rd(g, inst + 64)?;
    let sec = rd(g, inst + 28)?;
    let mut desc = sec.wrapping_add(rd(g, sec + 20)?);
    let mut k = 0u32;
    while (k as i32) < rdi(g, inst + 88)? {
        let e = rd(g, inst + 16)?.wrapping_add(k << 3);
        let def = rd(g, e)?;
        let rec = rd(g, def)?;
        wr(g, def + 12, desc)?;
        let words = rd(g, desc)? & 0xFF;
        let n = u32::from(g.u16(rec)?) & 0xFF;
        let (mut special, mut normal) = (0u32, 0u32);
        for m in 0..n {
            let w = rd(g, rec + 12 + 4 * m)?;
            if w & 0xE000_0000 == 0x8000_0000 {
                special += 1;
            } else {
                let s = (((w as i32) >> 16) as u32) & 0xFF;
                normal = normal.wrapping_add(if s == own { 1 } else { instances(g, h, s)? });
            }
        }
        let used = rd(g, h + 544)?;
        wr(g, h + 544, used.wrapping_add(special).wrapping_add(normal))?;
        let st = rd(g, e + 4)?;
        let base = rd(g, h + 516)?.wrapping_add(used << 2);
        wr(g, st + 4, base)?;
        if (special as i32) > 0 {
            wr(g, st + 12, rd(g, st + 4)?.wrapping_add(normal << 2))?;
        } else {
            wr(g, st + 12, 0)?;
        }
        wr(g, def + 8, (special << 16) | normal)?;
        let mut words_at = rec + 12;
        let sp = rd(g, st + 12)?;
        for m in 0..special {
            let w = rd(g, words_at)?;
            words_at += 4;
            wr(g, sp + 4 * m, (w & 0xFFFF_07FF) | ((j << 11) & 0xFFFF_F800))?;
        }
        let out = rd(g, st + 4)?;
        expand_into(g, inst, words_at, out, normal)?;
        desc = desc.wrapping_add((words + 1) << 2);
        k += 1;
    }
    Ok(())
}

/// `sub_8294E120` — count the distinct controller keys and allocate the controller array
/// (host+160) and the 16-byte controllers (host+164).
fn count_controllers(g: &mut Guest, heap: &mut dyn Heap, h: u32) -> Result<()> {
    let n_e = rdi(g, h + 476)?.max(0) as u32;
    let e_arr = rd(g, h + 448)?;
    let e_key =
        |g: &Guest, i: u32| -> Result<u32> { Ok(g.u32(g.u32(e_arr + 8 * i + 4)?)? & KEY_MASK) };
    let n_in = rdi(g, h + H::INPUT_CAP)?.max(0) as u32;
    let inputs = rd(g, h + 388)?;
    let in_key = |g: &Guest, i: u32| -> Result<u32> { Ok(g.u32(inputs + 16 * i)? & KEY_MASK) };
    let n_refs = rdi(g, h + 212)?.max(0) as u32;
    let refs = rd(g, h + 384)?;
    let ref_key = |g: &Guest, i: u32| -> Result<u32> { Ok(g.u32(refs + 4 * i)? & KEY_MASK) };
    let n_b = rdi(g, h + 468)?.max(0) as u32;
    let b_arr = rd(g, h + 416)?;
    let b_key =
        |g: &Guest, i: u32| -> Result<u32> { Ok(g.u32(g.u32(b_arr + 8 * i + 4)?)? & KEY_MASK) };
    let n_f = rdi(g, h + 480)?.max(0) as u32;
    let f_arr = rd(g, h + 404)?;
    let f_key = |g: &Guest, i: u32| -> Result<u32> {
        Ok(g.u32(g.u32(f_arr + 8 * i + 4)? + 16)? & KEY_MASK)
    };
    let any = |g: &Guest, n: u32, f: &dyn Fn(&Guest, u32) -> Result<u32>, k: u32| -> Result<bool> {
        for m in 0..n {
            if f(g, m)? == k {
                return Ok(true);
            }
        }
        Ok(false)
    };
    let (mut r18, mut r17, mut r22) = (0u32, 0u32, 0u32);
    let mut classify = |top: u32| match top {
        0x4000_0000 => r18 += 1,
        0x6000_0000 => r17 += 1,
        0x8000_0000 => r22 += 1,
        _ => {}
    };
    for i in 0..n_e {
        let k = e_key(g, i)?;
        if !any(g, i, &e_key, k)? {
            classify(k & 0xE000_0000);
        }
    }
    for i in 0..n_in {
        let raw = g.u32(inputs + 16 * i)?;
        let k = raw & KEY_MASK;
        if controller_top(raw & 0xE000_0000) && !any(g, i, &in_key, k)? && !any(g, n_e, &e_key, k)?
        {
            classify(raw & 0xE000_0000);
        }
    }
    for i in 0..n_refs {
        let raw = g.u32(refs + 4 * i)?;
        let k = raw & KEY_MASK;
        if controller_top(raw & 0xE000_0000)
            && !any(g, i, &ref_key, k)?
            && !any(g, n_e, &e_key, k)?
            && !any(g, n_in, &in_key, k)?
        {
            classify(raw & 0xE000_0000);
        }
    }
    for i in 0..n_b {
        let k = b_key(g, i)?;
        if !any(g, i, &b_key, k)?
            && !any(g, n_e, &e_key, k)?
            && !any(g, n_in, &in_key, k)?
            && !any(g, n_refs, &ref_key, k)?
        {
            r22 += 1;
        }
    }
    let mut classify = |top: u32| match top {
        0x4000_0000 => r18 += 1,
        0x6000_0000 => r17 += 1,
        0x8000_0000 => r22 += 1,
        _ => {}
    };
    for i in 0..n_f {
        let k = f_key(g, i)?;
        let top = k & 0xE000_0000;
        if controller_top(top)
            && !any(g, i, &f_key, k)?
            && !any(g, n_e, &e_key, k)?
            && !any(g, n_in, &in_key, k)?
            && !any(g, n_refs, &ref_key, k)?
            && !any(g, n_b, &b_key, k)?
        {
            classify(top);
        }
    }
    wr(g, h + 176, r22)?;
    let total = r22.wrapping_add(r17).wrapping_add(r18);
    wr(g, h + H::CTRL_CAP, total)?;
    let ptrs = alloc(g, heap, total << 2)?;
    wr(g, h + H::CTRL_PTRS, ptrs)?;
    let storage = alloc(g, heap, total << 4)?;
    wr(g, h + 164, storage)?;
    for i in 0..total {
        let c = rd(g, h + 164)?.wrapping_add(16 * i);
        let c = if c != 0 {
            wr(g, c, CONTROLLER_VTABLE)?;
            wr(g, c + 8, 0)?;
            wr(g, c + 12, 0)?;
            c
        } else {
            0
        };
        wr(g, rd(g, h + H::CTRL_PTRS)? + 4 * i, c)?;
        wr(g, rd(g, rd(g, h + H::CTRL_PTRS)? + 4 * i)? + 4, 0xFFFF_FFFF)?;
    }
    Ok(())
}

/// `sub_8294CB48(host, key, block)` — find the controller for a key or take the next free one; set
/// its output block if it has none, and hand a new one to the manager (vfunc 4). Always 1.
pub fn find_or_create_output(
    g: &mut Guest,
    listener: &mut dyn Listener,
    h: u32,
    key: u32,
    block: u32,
) -> Result<u32> {
    let k = key & KEY_MASK;
    let count = rd(g, h + H::CTRL_COUNT)?;
    let ptrs = rd(g, h + H::CTRL_PTRS)?;
    let mut i = 0u32;
    let mut found = false;
    while (i as i32) < count as i32 {
        if rd(g, rd(g, ptrs + 4 * i)? + 4)? == k {
            found = true;
            break;
        }
        i += 1;
    }
    if !found {
        wr(g, h + H::CTRL_COUNT, count.wrapping_add(1))?;
        let ctrl = rd(g, rd(g, h + H::CTRL_PTRS)? + 4 * i)?;
        set_outputs(g, ctrl, block)?; // vfunc 32
        wr(g, ctrl + 4, k)?;
        listener.bind(g, ctrl)?; // [host+108] vfunc 4
        return Ok(1);
    }
    let ctrl = rd(g, rd(g, h + H::CTRL_PTRS)? + 4 * i)?;
    if rdi(g, ctrl + 12)? == 0 {
        set_outputs(g, ctrl, block)?;
    }
    Ok(1)
}

/// The instance object a stage key addresses: `[[host+152][slot]+92] + 96 × group`.
fn keyed_instance(g: &Guest, h: u32, key: u32) -> Result<u32> {
    let slot = ((key as i32 >> 16) as u32) & 0xFF;
    let group = ((key as i32 >> 11) as u32) & 0x1F;
    let base = rd(g, rd(g, h + 152)?.wrapping_add(slot << 2))?;
    Ok(rd(g, base + 92)?.wrapping_add(group * 96))
}

/// A stage entry by `(count field, array field)` of the instance, or 0 when out of range.
fn keyed_entry(g: &Guest, obj: u32, idx: u32, count: u32, array: u32) -> Result<u32> {
    if obj != 0 && (idx as i32) < rdi(g, obj + count)? {
        Ok(rd(g, obj + array)?.wrapping_add(idx << 3))
    } else {
        Ok(0)
    }
}

/// `sub_8294EDF8(host, key, r5, r6)` — resolve a key to the address of the value it names.
///
/// | key top 3 bits | resolves to |
/// |---|---|
/// | `000` A product | `r5`: product result `e2+8`; else the input entry's shaped value `+12` |
/// | `001` C/E node | bit 28 set: sum `+4`; clear: output value `st+8` |
/// | `010`/`011` controller | its input block + 4 × (key & 0xF), creating the controller/block |
/// | `100` B lookup | `r6`: the entry itself; `r5`: `st+12` (mB); else `st+16` (linear) |
/// | `101` F envelope | `r5`: `st+24`; else `st+28` |
/// | other | [`NULL_INPUT`] |
pub fn resolve(
    g: &mut Guest,
    listener: &mut dyn Listener,
    h: u32,
    key: u32,
    r5: u32,
    r6: u32,
) -> Result<u32> {
    let top = key & 0xE000_0000;
    let idx = key & 0xFF;
    match top {
        0x0000_0000 => {
            let obj = keyed_instance(g, h, key)?;
            let e = keyed_entry(g, obj, idx, 72, 8)?;
            let e2 = rd(g, e + 4)?;
            if r5 & 0xFF != 0 {
                Ok(e2.wrapping_add(8))
            } else {
                Ok(rd(g, e2)?.wrapping_add(12))
            }
        }
        0xA000_0000 => {
            let obj = keyed_instance(g, h, key)?;
            let e = keyed_entry(g, obj, idx, 80, 24)?;
            let st = rd(g, e + 4)?;
            Ok(st.wrapping_add(if r5 & 0xFF != 0 { 24 } else { 28 }))
        }
        0x8000_0000 => {
            let obj = keyed_instance(g, h, key)?;
            let e = keyed_entry(g, obj, idx, 76, 20)?;
            if r6 & 0xFF != 0 {
                return Ok(e);
            }
            let st = rd(g, e + 4)?;
            Ok(st.wrapping_add(if r5 & 0xFF != 0 { 12 } else { 16 }))
        }
        0x2000_0000 => {
            let obj = keyed_instance(g, h, key)?;
            if key & 0x1000_0000 != 0 {
                let e = keyed_entry(g, obj, idx, 84, 12)?;
                Ok(rd(g, e + 4)?.wrapping_add(4))
            } else {
                let e = keyed_entry(g, obj, idx, 88, 16)?;
                Ok(rd(g, e + 4)?.wrapping_add(8))
            }
        }
        0x4000_0000 | 0x6000_0000 => {
            let k = key & KEY_MASK;
            let count = rd(g, h + H::CTRL_COUNT)?;
            let ptrs = rd(g, h + H::CTRL_PTRS)?;
            let mut i = 0u32;
            let mut found = false;
            while (i as i32) < count as i32 {
                if rd(g, rd(g, ptrs + 4 * i)? + 4)? == k {
                    found = true;
                    break;
                }
                i += 1;
            }
            if !found {
                wr(g, h + H::CTRL_COUNT, count.wrapping_add(1))?;
                let block = take_block(g, h)?;
                let ctrl = rd(g, rd(g, h + H::CTRL_PTRS)? + 4 * i)?;
                set_inputs(g, ctrl, block)?; // vfunc 28
                wr(g, ctrl + 4, k)?;
                listener.bind(g, ctrl)?; // [host+108] vfunc 4
            }
            let ctrl = rd(g, rd(g, h + H::CTRL_PTRS)? + 4 * i)?;
            if rdi(g, ctrl + 8)? == 0 {
                let block = take_block(g, h)?;
                set_inputs(g, ctrl, block)?;
            }
            Ok(rd(g, ctrl + 8)?.wrapping_add((key & 0xF) << 2))
        }
        _ => Ok(NULL_INPUT),
    }
}

/// The next zeroed 64-byte block of host+380 (host+188 counts them).
fn take_block(g: &mut Guest, h: u32) -> Result<u32> {
    let n = bump(g, h + 188)?;
    let block = rd(g, h + 380)?.wrapping_add(n << 6);
    for w in 0..16 {
        wr(g, block + 4 * w, 0)?;
    }
    Ok(block)
}

/// `sub_8294F228(host)`.
fn resolve_all(g: &mut Guest, listener: &mut dyn Listener, h: u32) -> Result<()> {
    let _fpscr = Fpscr::capture();
    let mut i = 0u32;
    while (i as i32) < rdi(g, h + 476)? {
        let st = rd(g, rd(g, h + 448)? + 8 * i + 4)?;
        let r = find_or_create_output(g, listener, h, rd(g, st)?, rd(g, st + 16)?)?;
        let block = rd(g, st + 16)?;
        for w in 0..15 {
            wr(g, block + 4 * w, 0)?;
        }
        wr(g, block + 60, u32::from(r & 0xFF != 0))?;
        i += 1;
    }
    let mut i = 0u32;
    while (i as i32) < rdi(g, h + H::INPUT_CAP)? {
        let e = rd(g, h + 388)? + 16 * i;
        let p = resolve(g, listener, h, rd(g, e)?, 0, 0)?;
        wr(g, e + 4, if p == 0 { NULL_INPUT } else { p })?;
        i += 1;
    }
    let mut i = 0u32;
    while (i as i32) < rdi(g, h + 212)? {
        let at = rd(g, h + 384)? + 4 * i;
        let p = resolve(g, listener, h, rd(g, at)?, 0, 0)?;
        wr(g, rd(g, h + 384)? + 4 * i, p)?;
        i += 1;
    }
    let mut i = 0u32;
    while (i as i32) < rdi(g, h + 468)? {
        let st = rd(g, rd(g, h + 416)? + 8 * i + 4)?;
        let p = resolve(g, listener, h, rd(g, st + 4)?, 0, 0)?;
        wr(g, st + 4, p)?;
        i += 1;
    }
    let scale = load_single(g, K_FRAMES_TO_MS)?;
    let mut i = 0u32;
    while (i as i32) < rdi(g, h + 480)? {
        let e = rd(g, h + 404)? + 8 * i;
        let st = rd(g, e + 4)?;
        let p = resolve(g, listener, h, rd(g, st + 16)?, 0, 0)?;
        wr(g, st + 16, p)?;
        let rec = rd(g, rd(g, e)?)?;
        let fields = [
            rd(g, rec + 12)? & 0xFFF,
            u32::from(g.u16(rec + 16)?) & 0xFFF,
            rd(g, rec + 16)? & 0xFFF,
            rd(g, rec + 20)? & 0xFFF,
        ];
        for (n, v) in fields.into_iter().enumerate() {
            store_single(
                g,
                st + 32 + 4 * n as u32,
                mul_single(word_to_single(v), scale),
            )?;
        }
        i += 1;
    }
    let mut i = 0u32;
    while (i as i32) < rdi(g, h + 496)? {
        let at = rd(g, h + 520)? + 4 * i;
        let p = resolve(g, listener, h, rd(g, at)?, 1, 0)?;
        wr(g, at, p)?;
        i += 1;
    }
    let mut i = 0u32;
    while (i as i32) < rdi(g, h + 488)? {
        let at = rd(g, h + 516)? + 4 * i;
        let p = resolve(g, listener, h, rd(g, at)?, 1, 1)?;
        wr(g, at, p)?;
        i += 1;
    }
    Ok(())
}

/// Every array's capacity (from the sizing pass) against the count its builder used.
///
/// Not a port: a diagnostic. `sub_8294E970` and `sub_8294E120` walk the input array to its
/// capacity, so a file whose inputs deduplicate across sections would make them read allocator
/// garbage in retail. Returns `(name, capacity, used)` rows.
pub fn check_capacities(g: &Guest, h: u32) -> Result<Vec<(&'static str, u32, u32)>> {
    let mut used_inputs = 0u32;
    for k in 0..10 {
        used_inputs += rd(g, h + 224 + 8 * k)?;
    }
    Ok(vec![
        ("inputs 388", rd(g, h + H::INPUT_CAP)?, used_inputs),
        ("references 384", rd(g, h + 212)?, rd(g, h + 216)?),
        ("product defs 392", rd(g, h + 196)?, rd(g, h + 320)?),
        ("products 400", rd(g, h + 464)?, rd(g, h + 316)?),
        ("product values 396", rd(g, h + 464)?, rd(g, h + 324)?),
        ("lookup variants 420", rd(g, h + 308)?, rd(g, h + 356)?),
        ("lookups 416", rd(g, h + 468)?, rd(g, h + 352)?),
        ("lookup states 424", rd(g, h + 468)?, rd(g, h + 360)?),
        ("envelope defs 408", rd(g, h + 312)?, rd(g, h + 368)?),
        ("envelopes 404", rd(g, h + 480)?, rd(g, h + 364)?),
        ("envelope states 412", rd(g, h + 480)?, rd(g, h + 372)?),
        ("sum defs 428", rd(g, h + 300)?, rd(g, h + 332)?),
        ("sums 436", rd(g, h + 472)?, rd(g, h + 328)?),
        ("sum states 432", rd(g, h + 472)?, rd(g, h + 336)?),
        ("output defs 440", rd(g, h + 304)?, rd(g, h + 344)?),
        ("outputs 448", rd(g, h + 476)?, rd(g, h + 340)?),
        ("output states 444", rd(g, h + 476)?, rd(g, h + 348)?),
        ("sum references 520", rd(g, h + 496)?, rd(g, h + 548)?),
        ("output references 516", rd(g, h + 488)?, rd(g, h + 544)?),
        (
            "output blocks 376 (×16 words)",
            rd(g, h + 504)? * 16,
            rd(g, h + 552)?,
        ),
        (
            "block pool 380",
            POOL_CAPACITY.with(|c| c.get()),
            rd(g, h + 188)?,
        ),
        (
            "controllers 164",
            rd(g, h + H::CTRL_CAP)?,
            rd(g, h + H::CTRL_COUNT)?,
        ),
        (
            "instances 156",
            rd(g, h + H::INSTANCES_TOTAL)? * 96,
            rd(g, h + 524)?,
        ),
    ])
}
