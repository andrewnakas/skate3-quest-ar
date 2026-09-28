//! The packet stream's decoders: a bit reader, two variable-length integer codes, the run-length
//! layer over them, and the paged bit cursor.
//!
//! Seven verified leaves and near-leaves, ported from their `recomp/src/audio_ports/sub_*.inc`. None
//! of them touches a float; they are integer work over big-endian bytes, which is why this module is
//! not gated on x86 the way the mixer's are.
//!
//! | function | guest | lifted lines | calls/boot | calls/play |
//! |---|---|---|---|---|
//! | [`read_bits`] | `sub_82B26B70` | 64 | 36,190 | 42,524 |
//! | [`decode_unsigned`] | `sub_82B4F5F8` | 127 | 32,974 | 22,336 |
//! | [`decode_signed`] | `sub_82B46EA0` | 150 | 32,604 | 26,780 |
//! | [`decode_signed_into`] | `sub_82B46FB0` | 53 | 15,048 | 12,360 |
//! | [`run_length_step`] | `sub_82B47010` | 107 | 12,540 | 10,300 |
//! | [`decode_four`] | `sub_82B47658` | 60 | 2,508 | 2,060 |
//! | [`advance_bit_cursor`] | `sub_82B50270` | 162 | 6,354 | 5,518 |
//! | [`unpack_stream_header`] | `sub_82B31D90` | 93 | 745 | 981 |
//!
//! Replayed against **20,000 recorded calls, 0 disagreements**, and each compared live against the
//! original between 481 and 17,668 times in the same session.
//!
//! ## The two variable-length codes
//!
//! Both read a prefix byte whose *value* picks the width — below 192 one byte, below 240 two, below
//! 252 three, below 255 four, and 255 means four raw bytes follow. The payload is what the prefix
//! leaves over, plus a bias that makes each width start exactly where the previous one ended, so no
//! value has two short encodings.
//!
//! The signed code is **zigzag**: the last byte's low bit is the sign, and a set bit means the value
//! is `!magnitude` (that is, `-1 - magnitude`). One magnitude step costs two encoded steps, so every
//! signed bias is exactly half the unsigned one — 96, 6240 and 399,456 against 192, 12,480 and
//! 798,912. The five-byte form of either code is the raw 32-bit word, with no zigzag and no bias.
//!
//! The tests encode with an encoder written from that description and check the decoders round-trip
//! it across every width boundary, which is what catches a mask one bit off.
//!
//! ## The run-length layer
//!
//! A sub-object `{cursor* +0, accumulated +4, remaining +8, flag +12}` walks a stream of signed codes.
//! When its run is exhausted it reads a **count** code: a non-negative count `c` starts a *repeat*
//! run — one delta is read now, and the running sum is returned `c + 1` times — and a negative count
//! starts a *literal* run of `1 - c` values, each with its own delta. [`run_length_step`] returns the
//! running sum on every call. [`decode_four`] runs four of these side by side.
//!
//! ## The paged bit cursor
//!
//! [`advance_bit_cursor`] reads a 15-bit field at a bit cursor and advances the cursor by it. The
//! stream is paged: 2,048-byte pages whose first 32 bits are a header, so each page carries 16,352
//! payload bits. A field that straddles a page end takes its tail from the next page past that
//! page's header, and a cursor that runs off the end steps the buffer base on by one page.
//!
//! ## What is not reproduced
//!
//! Two of these open a 112-byte frame and hand the callee a scratch word at `sp + 80` to decode into.
//! Here the decoder returns the value instead ([`signed_code`], [`unsigned_code`]), so there is no
//! scratch word and no frame: nothing but this function ever reads that word, and the guest-facing
//! [`decode_signed`] and [`decode_unsigned`] still store exactly where the original does.

use crate::{Guest, Result};

// ------------------------------------------------------------------------ the MSB-first bit reader

/// `lwz r6,0(r3)` — the reader's byte stream.
pub const READER_DATA: u32 = 0;
/// `lwz r10,4(r8)` — the absolute bit position, **reloaded every iteration**.
pub const READER_CURSOR: u32 = 4;

/// Read `bits` bits MSB-first from the reader at `reader`, advancing its cursor (`sub_82B26B70`).
///
/// A do-while over bytes: each pass takes the rest of the current byte or what is left, whichever is
/// smaller. So a request for **zero** bits still loads one byte and stores the cursor back unchanged,
/// which is observable and reproduced. The count is 64-bit in the guest and the loop ends when its
/// low word reaches zero; returns the accumulated value, zero-extended.
pub fn read_bits(g: &mut Guest, reader: u32, bits: u64) -> Result<u64> {
    let data = g.u32(reader + READER_DATA)?; // lwz r6,0(r3)
    let mut remaining = bits; // subf. r4,r11,r4 works in 64 bits
    let mut accumulator = 0u32; // li r3,0
    loop {
        let cursor = g.u32(reader + READER_CURSOR)?; // lwz r10,4(r8)
        let bit_in_byte = cursor & 7; // clrlwi r9,r10,29
        let byte_index = cursor >> 3; // rlwinm r7,r10,29,3,31
        let mut take = 8 - bit_in_byte; // subfic r11,r9,8
        if take > remaining as u32 {
            take = remaining as u32; // cmplw ; mr r11,r4 -- a 32-bit unsigned compare
        }
        let shift = (8 - take - bit_in_byte) & 0xFF;
        let byte = u32::from(g.u8(data.wrapping_add(byte_index))?); // lbzx r7,r6,r7
        accumulator <<= take; // slw r3,r3,r11 -- take is at most 8
        let mask = (1u32 << take) - 1; // slw r9,r5,r11 ; addi r9,r9,-1
        accumulator |= (byte >> shift) & mask;
        g.set_u32(reader + READER_CURSOR, cursor.wrapping_add(take))?; // stw r7,4(r8)
        remaining = remaining.wrapping_sub(u64::from(take));
        if remaining as u32 as i32 == 0 {
            break; // bne on cr0 from the 32-bit signed compare
        }
    }
    Ok(u64::from(accumulator))
}

// ------------------------------------------------------------------- the variable-length codes

/// The prefix thresholds shared by both codes: below each, the code is 1, 2, 3 or 4 bytes long.
pub const PREFIX_LIMITS: [i32; 4] = [192, 240, 252, 255];
/// The unsigned code's bias for its 2-, 3- and 4-byte forms. Each is where the previous width ends.
pub const UNSIGNED_BIASES: [u32; 3] = [192, 12_480, 786_432 + 12_480];
/// The signed code's biases: exactly half the unsigned ones, because the payload is zigzagged.
pub const SIGNED_BIASES: [u32; 3] = [96, 6_240, 393_216 + 6_240];

const _: () = assert!(
    SIGNED_BIASES[0] * 2 == UNSIGNED_BIASES[0]
        && SIGNED_BIASES[1] * 2 == UNSIGNED_BIASES[1]
        && SIGNED_BIASES[2] * 2 == UNSIGNED_BIASES[2],
    "one magnitude step is two encoded steps"
);

/// `rlwinm rX,rY,8,0,23`: rotate left eight and keep bits 0..23. The byte that wraps round is zero
/// on every path that uses it, but the rotate is what the original does.
fn rotate_in_byte(value: u32) -> u32 {
    value.rotate_left(8) & 0xFFFF_FF00
}

/// The bytes of a code, read on demand: the prefix decides how many are touched.
fn code_byte(g: &Guest, stream: u32, index: u32) -> Result<u32> {
    Ok(u32::from(g.u8(stream.wrapping_add(index))?))
}

/// One unsigned variable-length code at `stream`: `(value, bytes consumed)`.
pub fn unsigned_code(g: &Guest, stream: u32) -> Result<(u32, u32)> {
    let b0 = code_byte(g, stream, 0)?; // lbz r10,0(r3)
    // Every compare is cmpwi on the zero-extended byte, so the ladder is on 0..255.
    Ok(if (b0 as i32) < PREFIX_LIMITS[0] {
        (b0, 1)
    } else if (b0 as i32) < PREFIX_LIMITS[1] {
        let b1 = code_byte(g, stream, 1)?;
        // rlwinm r11,r9,0,18,15 clears bits 14 and 15 of the pair: the 0xC0 prefix.
        (
            (((b0 << 8) | b1) & 0xFFFF_3FFF).wrapping_add(UNSIGNED_BIASES[0]),
            2,
        )
    } else if (b0 as i32) < PREFIX_LIMITS[2] {
        let b1 = code_byte(g, stream, 1)?;
        let b2 = code_byte(g, stream, 2)?;
        // rlwinm r11,r11,0,12,7 clears bits 20..23: the 0xF0 prefix.
        (
            ((((b0 << 8) | b1) << 8 | b2) & 0xFF0F_FFFF).wrapping_add(UNSIGNED_BIASES[1]),
            3,
        )
    } else if (b0 as i32) < PREFIX_LIMITS[3] {
        let b1 = code_byte(g, stream, 1)?;
        let b2 = code_byte(g, stream, 2)?;
        let b3 = code_byte(g, stream, 3)?;
        // rlwinm r8,r10,24,6,7 keeps the prefix's low two bits as bits 24..25.
        let high = (b0 & 3) << 24;
        let low = ((((b1 << 8) | b2) << 8) | b3) & 0x3FF_FFFF;
        ((low | high).wrapping_add(UNSIGNED_BIASES[2]), 4)
    } else {
        let b1 = code_byte(g, stream, 1)?;
        let b2 = code_byte(g, stream, 2)?;
        let b3 = code_byte(g, stream, 3)?;
        let b4 = code_byte(g, stream, 4)?;
        ((b1 << 24) | (b2 << 16) | (b3 << 8) | b4, 5) // no bias on the widest form
    })
}

/// Decode one unsigned code at `stream`, store it at `out`, return its length (`sub_82B4F5F8`).
pub fn decode_unsigned(g: &mut Guest, stream: u32, out: u32) -> Result<u64> {
    let (value, length) = unsigned_code(g, stream)?;
    g.set_u32(out, value)?; // stw r10,0(r4)
    Ok(u64::from(length)) // li r3,1..5
}

/// One signed zigzag code at `stream`: `(value as a u32, bytes consumed)`.
pub fn signed_code(g: &Guest, stream: u32) -> Result<(u32, u32)> {
    let b0 = code_byte(g, stream, 0)?; // lbz r10,0(r3)
    // (magnitude, the zigzag sign bit, length). The sign is bit 0 of the code's LAST byte.
    let (magnitude, low_bit, length): (u64, u32, u32) = if (b0 as i32) < PREFIX_LIMITS[0] {
        (u64::from(b0 >> 1), b0 & 1, 1) // rlwinm r11,r10,31,1,31 ; clrlwi r10,r10,31
    } else if (b0 as i32) < PREFIX_LIMITS[1] {
        let b1 = code_byte(g, stream, 1)?;
        let pair = ((b0 << 8) & 0xFFFF_FF00) | b1; // rlwinm r10,r10,8,0,23 ; or
        // srawi r8,r9,1 ; rlwinm r11,r8,0,19,16 -- halve, then clear bits 13 and 14, which is where
        // the 0xC0 prefix lands once the pair has been shifted down one.
        let v = (((pair as i32) >> 1) as u32 & 0xFFFF_9FFF).wrapping_add(SIGNED_BIASES[0]);
        (u64::from(v), b1 & 1, 2)
    } else if (b0 as i32) < PREFIX_LIMITS[2] {
        let b1 = code_byte(g, stream, 1)?;
        let b2 = code_byte(g, stream, 2)?;
        let pair = ((b0 << 8) & 0xFFFF_FF00) | b1;
        // rlwinm r11,r6,0,20,15 strips the 0xF0 prefix BEFORE the third byte is appended.
        let trio = rotate_in_byte(pair & 0xFFFF_0FFF) | b2;
        (
            u64::from(((trio as i32) >> 1) as u32) + u64::from(SIGNED_BIASES[1]),
            b2 & 1,
            3,
        )
    } else if (b0 as i32) < PREFIX_LIMITS[3] {
        let b1 = code_byte(g, stream, 1)?;
        let b2 = code_byte(g, stream, 2)?;
        let b3 = code_byte(g, stream, 3)?;
        let high = b0.rotate_left(16) & 0x3_0000; // rlwinm r8,r10,16,14,15
        let mid = (b1.rotate_left(8) | b2) & 0x3_FFFF; // clrlwi r7,r11,14
        let quad = rotate_in_byte(mid | high) | (b3 & 0xFFFF_FFFE); // rlwinm r9,r5,0,0,30
        (
            u64::from(((quad as i32) >> 1) as u32) + u64::from(SIGNED_BIASES[2]),
            b3 & 1,
            4,
        )
    } else {
        // loc_82B46F7C: the raw 32-bit two's-complement word, never reaching the sign step.
        let b1 = code_byte(g, stream, 1)?;
        let b2 = code_byte(g, stream, 2)?;
        let b3 = code_byte(g, stream, 3)?;
        let b4 = code_byte(g, stream, 4)?;
        let wide = rotate_in_byte(rotate_in_byte(b1.rotate_left(8) | b2) | b3) | b4;
        return Ok((wide, 5));
    };
    // loc_82B46F20: an odd encoding is a negative. `subfic r11,r11,-1` is a 64-bit subtract; only
    // its low word is stored.
    let value = if low_bit != 0 {
        u64::MAX.wrapping_sub(magnitude)
    } else {
        magnitude
    };
    Ok((value as u32, length))
}

/// Decode one signed code at `stream`, store it at `out`, return its length (`sub_82B46EA0`).
pub fn decode_signed(g: &mut Guest, stream: u32, out: u32) -> Result<u64> {
    let (value, length) = signed_code(g, stream)?;
    g.set_u32(out, value)?; // stw r11,0(r4)
    Ok(u64::from(length))
}

// ---------------------------------------------------------------------- the run-length layer

/// A run's sub-object: `+0` its cursor object, `+4` the running sum, `+8` what is left of the run
/// (signed), `+12` the flag byte — set for a repeat run, clear for a literal run.
pub const RUN_CURSOR: u32 = 0;
/// `+4` — the running sum, returned by every [`run_length_step`].
pub const RUN_ACCUMULATED: u32 = 4;
/// `+8` — values left in the current run. Compared signed: zero or below means read a new count.
pub const RUN_REMAINING: u32 = 8;
/// `+12` — 1 for a repeat run, 0 for a literal run. Re-read from memory at every decision.
pub const RUN_FLAG: u32 = 12;
/// The cursor object's `+0`: the live byte pointer into the stream.
pub const CURSOR_STREAM: u32 = 0;

/// Decode one signed code into the running sum and advance the stream (`sub_82B46FB0`).
///
/// Returns the code's byte length, which is what the original leaves in `r3` — it is the callee's
/// return value and nothing overwrites it. The stream pointer is **re-read after the decode** even
/// though the decode never writes it; kept, since a cursor object that aliased the stream would see
/// the difference.
pub fn decode_signed_into(g: &mut Guest, object: u32) -> Result<u64> {
    let cursor = g.u32(object + RUN_CURSOR)?; // lwz r30,0(r3)
    let stream = g.u32(cursor + CURSOR_STREAM)?; // lwz r3,0(r30)
    let (decoded, length) = signed_code(g, stream)?; // bl 0x82b46ea0
    let reloaded = g.u32(cursor + CURSOR_STREAM)?; // lwz r10,0(r30)
    g.set_u32(cursor + CURSOR_STREAM, reloaded.wrapping_add(length))?; // stw r9,0(r30)
    let accumulated = g.u32(object + RUN_ACCUMULATED)?; // lwz r10,4(r31)
    g.set_u32(object + RUN_ACCUMULATED, accumulated.wrapping_add(decoded))?; // stw r8,4(r31)
    Ok(u64::from(length))
}

/// One step of the run-length decoder: return the current running sum (`sub_82B47010`).
///
/// When the run is exhausted a count code is read. The two stores that follow it — flag 1, remaining
/// `count + 1` — are overwritten straight away for a negative count, and those dead stores stay,
/// because a concurrent reader could see them. The flag is then **re-read from memory** twice, and
/// exactly one of the two [`decode_signed_into`] call sites runs after a fresh count.
pub fn run_length_step(g: &mut Guest, object: u32) -> Result<u64> {
    // lwz r11,8(r3) ; cmpwi cr6,r11,0 ; bgt -- signed
    if g.u32(object + RUN_REMAINING)? as i32 <= 0 {
        let cursor = g.u32(object + RUN_CURSOR)?;
        let stream = g.u32(cursor + CURSOR_STREAM)?;
        let (count, length) = signed_code(g, stream)?; // bl 0x82b46ea0
        let reloaded = g.u32(cursor + CURSOR_STREAM)?; // lwz r10,0(r30) -- reloaded
        g.set_u32(cursor + CURSOR_STREAM, reloaded.wrapping_add(length))?;
        g.set_u32(object + RUN_REMAINING, count.wrapping_add(1))?; // addi r7,r11,1 ; stw
        g.set_u8(object + RUN_FLAG, 1)?; // stb r9,12(r31)
        if (count as i32) < 0 {
            // A literal run: the two stores above are overwritten, and they stay as dead stores.
            g.set_u8(object + RUN_FLAG, 0)?; // stb r10,12(r31)
            g.set_u32(object + RUN_REMAINING, 1u32.wrapping_sub(count))?; // subfic r9,r11,1
        }
        if g.u8(object + RUN_FLAG)? != 0 {
            decode_signed_into(g, object)?; // the repeat run's one delta
        }
    }
    // loc_82B47090 -- the flag again, from memory.
    if g.u8(object + RUN_FLAG)? == 0 {
        decode_signed_into(g, object)?; // a literal run decodes on every call
    }
    let remaining = g.u32(object + RUN_REMAINING)?; // lwz r11,8(r31)
    let result = g.u32(object + RUN_ACCUMULATED)?; // lwz r3,4(r31)
    g.set_u32(object + RUN_REMAINING, remaining.wrapping_sub(1))?; // addi -1 ; stw
    Ok(u64::from(result))
}

/// The four sub-objects of [`decode_four`], sixteen bytes apart from `+4`.
pub const FIELD_OFFSETS: [u32; 4] = [4, 20, 36, 52];

/// Step four run-length decoders and store their four values at `out` (`sub_82B47658`).
///
/// `out` is `r3` and `input` is `r4`. The four may share one cursor object, in which case they
/// consume one stream in turn — the order is the original's, sub-object 0 first.
pub fn decode_four(g: &mut Guest, out: u32, input: u32) -> Result<()> {
    for (i, offset) in FIELD_OFFSETS.iter().enumerate() {
        let value = run_length_step(g, input.wrapping_add(*offset))?; // bl 0x82b47010
        g.set_u32(out + 4 * i as u32, value as u32)?; // stw r3,4i(r30)
    }
    Ok(())
}

// ------------------------------------------------------------------------ the paged bit cursor

/// Past this cursor a 15-bit field straddles the page end.
pub const SPLIT_POINT: i32 = 16_337;
/// Payload bits per page: 2,048 bytes less a 32-bit header. The cursor wraps here.
pub const PAGE_BITS: u32 = 16_352;
/// A page, in bytes. The buffer base steps by this on a wrap.
pub const PAGE_BYTES: u32 = 2_048;

const _: () = assert!(PAGE_BITS == PAGE_BYTES * 8 - 32, "a 32-bit header per page");
const _: () = assert!(
    SPLIT_POINT == PAGE_BITS as i32 - 15,
    "a 15-bit field fits below it"
);

/// `srawi r,x,3 ; addze`: divide by eight, truncating toward zero.
fn divide_by_eight_toward_zero(value: i32) -> i32 {
    let carry = value < 0 && (value as u32 & 7) != 0;
    (value >> 3) + i32::from(carry)
}

/// Three bytes at a byte offset, left-aligned in a word: `b0<<24 | b1<<16 | b2<<8`.
fn three_bytes_aligned(g: &Guest, buffer: u32, byte_offset: i32) -> Result<u32> {
    let at = buffer.wrapping_add(byte_offset as u32);
    let b0 = u32::from(g.u8(at)?);
    let b1 = u32::from(g.u8(at.wrapping_add(1))?);
    let b2 = u32::from(g.u8(at.wrapping_add(2))?);
    Ok(((((b0 << 8) | b1) << 8 | b2) << 8) & 0xFFFF_FF00)
}

/// Read the 15-bit field at the cursor and advance the cursor by it (`sub_82B50270`).
///
/// `base_field` is `r3`, holding the page buffer's address, and `cursor_field` is `r4`, holding the
/// bit cursor. The field is read at physical bit `cursor + 32`, past the page header; any part past
/// the split point comes from physical bit `cursor + width + 64`, past the *next* page's header. A
/// result at or past [`PAGE_BITS`] steps the base on one page and keeps the remainder — **once**, so a
/// field long enough to cross two pages leaves the cursor still past the end, as the original does.
pub fn advance_bit_cursor(g: &mut Guest, base_field: u32, cursor_field: u32) -> Result<()> {
    let cursor = g.u32(cursor_field)? as i32; // lwz r5,0(r4)
    let spill = if cursor > SPLIT_POINT {
        cursor.wrapping_sub(SPLIT_POINT)
    } else {
        0
    };
    let first_width = 15i32.wrapping_sub(spill); // subfic r11,r7,15
    let buffer = g.u32(base_field)?; // lwz r10,0(r3)

    let mut value = 0u32;
    if first_width > 0 {
        let position = cursor.wrapping_add(32); // addi r9,r5,32
        let byte_offset = divide_by_eight_toward_zero(position);
        let bit_in_group = position.wrapping_sub(byte_offset.wrapping_mul(8));
        let word = three_bytes_aligned(g, buffer, byte_offset)?;
        let shifted = word << (bit_in_group as u32 & 0x1F); // slw
        value = shifted >> ((32 - first_width) as u32 & 0x1F); // srw
    }

    let second_position = cursor.wrapping_add(first_width).wrapping_add(64);
    if spill > 0 {
        let already = value << (spill as u32 & 0x1F); // slw r6,r8,r7
        let byte_offset = divide_by_eight_toward_zero(second_position);
        let bit_in_group = second_position.wrapping_sub(byte_offset.wrapping_mul(8));
        let word = three_bytes_aligned(g, buffer, byte_offset)?;
        let shifted = word << (bit_in_group as u32 & 0x1F);
        value = (shifted >> (32i32.wrapping_sub(spill) as u32 & 0x1F)) | already;
    }

    let advanced = (cursor as u32).wrapping_add(value); // add r11,r5,r8
    if advanced < PAGE_BITS {
        return g.set_u32(cursor_field, advanced); // stw r11,0(r4)
    }
    g.set_u32(base_field, buffer.wrapping_add(PAGE_BYTES))?; // stw r10,0(r3)
    g.set_u32(cursor_field, advanced - PAGE_BITS) // stw r9,0(r4)
}

// ------------------------------------------------------------------- the packed stream header

/// `cmplwi cr6,r11,72` — an `'H'` prefix, which makes the header start four bytes later.
pub const HEADER_TAG: u8 = 72;
/// `stwu r1,-112(r1)` — the frame the bit reader lives in.
pub const HEADER_FRAME_BYTES: u32 = 112;
/// `addi r3,r1,80` — the reader's two words, handed to [`read_bits`] six times.
pub const HEADER_READER: u32 = 80;

/// Unpack a 64-bit packed stream header into a four-field description (`sub_82B31D90`).
///
/// Six fields are read MSB-first with [`read_bits`] — 4, 4, 6, 18, 3 and 29 bits — and four are
/// kept: the second nibble at `out + 12`, the 6-bit field **plus one** as a byte at `out + 0`, the
/// 18-bit field at `out + 4`, and the 29-bit field at `out + 8`. The first nibble and the 3-bit field
/// are read and discarded. An `'H'` tag byte in front skips four bytes.
///
/// The frame is reproduced, because the reader is guest memory that [`read_bits`] reads and writes.
pub fn unpack_stream_header(g: &mut Guest, stream: u32, out: u32, sp: u32) -> Result<()> {
    // lbz r11,0(r3) ; cmplwi cr6,r11,72 ; bne ; addi r3,r3,4 -- before the frame moves
    let body = if g.u8(stream)? == HEADER_TAG {
        stream.wrapping_add(4)
    } else {
        stream
    };
    let frame = sp.wrapping_sub(HEADER_FRAME_BYTES); // stwu r1,-112(r1)
    g.set_u32(frame, sp)?;
    let reader = frame + HEADER_READER;
    g.set_u32(reader + READER_DATA, body)?; // stw r3,80(r1)
    g.set_u32(reader + READER_CURSOR, 0)?; // li r11,0 ; stw r11,84(r1)

    read_bits(g, reader, 4)?; // the first nibble, discarded
    let nibble = read_bits(g, reader, 4)?;
    g.set_u32(out + 12, nibble as u32)?; // stw r3,12(r31)
    let count = read_bits(g, reader, 6)?;
    g.set_u8(out, count.wrapping_add(1) as u8)?; // addi r10,r3,1 ; stb r10,0(r31)
    let field18 = read_bits(g, reader, 18)?;
    g.set_u32(out + 4, field18 as u32)?; // stw r3,4(r31)
    read_bits(g, reader, 3)?; // discarded
    let field29 = read_bits(g, reader, 29)?;
    g.set_u32(out + 8, field29 as u32) // stw r3,8(r31)
}

// ============================================================ sub_82B335A8: a voice's bit header

/// `lwz r10,96(r3)` — the base of the 80-byte voice records.
pub const HEADER_VOICE_ARRAY: u32 = 96;
/// `lhz r9,464(r3)` — the byte offset, inside the object, of the 48-byte control slots.
pub const HEADER_SLOT_TABLE: u32 = 464;
/// Voice `+8`: the first byte after the header.
pub const VOICE_DATA_BASE: u32 = 8;
/// Voice `+12`: the tenth field, or 0.
pub const VOICE_DELTA: u32 = 12;
/// Voice `+16`: the ninth field, mode 2 only.
pub const VOICE_FIELD_16: u32 = 16;
/// Voice `+72`: the second field, four bits.
pub const VOICE_FORMAT_INDEX: u32 = 72;
/// Voice `+73`: the fifth field, two bits, then **reloaded**.
pub const VOICE_MODE: u32 = 73;
/// Voice `+76`: the first field, four bits.
pub const VOICE_STATE: u32 = 76;
/// Slot `+16`: the fourth field, eighteen bits, as a single.
pub const SLOT_FIELD_16: u32 = 16;
/// Slot `+20`: the seventh field, 29 bits.
pub const SLOT_FIELD_20: u32 = 20;
/// Slot `+24`: the eighth field, or -1.
pub const SLOT_CURSOR: u32 = 24;
/// Slot `+47`: the third field, six bits, plus one.
pub const SLOT_TYPE: u32 = 47;
/// `lbz r11,0(r29) ; cmplwi cr6,r11,72` — an `'H'` tag word, skipped.
pub const VOICE_HEADER_TAG: u8 = 72;
/// `stwu r1,-160(r1)`. Real: the bit reader's `{data, cursor}` pair lives at `r1 + 80`.
pub const HEADER_FRAME: u32 = 160;
const VOICE_HEADER_READER: u32 = 80;

/// Parse a stream's packed bit header into voice `index`'s record and control slot, or write the
/// defaults when there is no stream (`sub_82B335A8`).
///
/// `object` is `r3`, `index` `r4`, `stream` `r5` and `sp` `r1`. Both records are addressed once at
/// entry: the voice at `*(object + 96) + 80 * index`, the slot at `object + u16(object + 464) +
/// 48 * index`. The fields are read MSB-first through [`read_bits`] over a reader built in the frame:
/// 4, 4, 6, 18, 2, 1 and 29 bits, then a 32-bit cursor if the one-bit flag is set, a 32-bit field if
/// the mode (**re-read from the record**) is 2, and a 32-bit delta if the flag is set and the mode is
/// 1, or 2 with the cursor at or past that field — a signed compare of two words re-read after this
/// call stored them. The data base is the stream plus the header's whole bytes.
///
/// The original converts the eighteen-bit field through a doubleword at `r1 + 88`; the conversion is
/// exact, so it is done in registers here and that frame word is not written.
pub fn parse_voice_header(
    g: &mut Guest,
    object: u32,
    index: u32,
    stream: u64,
    sp: u32,
) -> Result<()> {
    let slot_off = u32::from(g.u16(object.wrapping_add(HEADER_SLOT_TABLE))?);
    let slot = slot_off
        .wrapping_add(index.wrapping_mul(3) << 4)
        .wrapping_add(object); // r30
    let array = g.u32(object.wrapping_add(HEADER_VOICE_ARRAY))?;
    let voice = (index.wrapping_mul(5) << 4).wrapping_add(array); // r31
    let frame = sp.wrapping_sub(HEADER_FRAME);
    g.set_u32(frame, sp)?; // stwu r1,-160(r1)
    let mut fpscr = crate::vmx::Fpscr::capture();
    if stream as u32 == 0 {
        g.set_u8(slot.wrapping_add(SLOT_TYPE), 0)?; // stb r27,47(r30)
        fpscr.disable_flush_mode_unconditional();
        let default = crate::fp::load_single(g, crate::routing::UNITY_GAIN)?; // lfs f0,-22460(r10)
        crate::fp::store_single(g, slot.wrapping_add(SLOT_FIELD_16), default)?;
        g.set_u32(slot.wrapping_add(SLOT_FIELD_20), 0x7FFF_FFFF)?; // lis ; ori ; stw r8,20(r30)
        g.set_u32(slot.wrapping_add(SLOT_CURSOR), u32::MAX)?; // li r7,-1 ; stw r7,24(r30)
        g.set_u8(voice.wrapping_add(VOICE_FORMAT_INDEX), 255)?;
        g.set_u8(voice.wrapping_add(VOICE_MODE), 1)?;
        g.set_u32(voice.wrapping_add(VOICE_FIELD_16), 0)?;
        g.set_u32(voice.wrapping_add(VOICE_DELTA), 0)?;
        g.set_u32(voice.wrapping_add(VOICE_DATA_BASE), 0)?;
        g.set_u8(voice.wrapping_add(VOICE_STATE), 1)?;
        return Ok(());
    }
    let mut stream = stream;
    if g.u8(stream as u32)? == VOICE_HEADER_TAG {
        stream = stream.wrapping_add(4); // addi r29,r29,4
    }
    let reader = frame.wrapping_add(VOICE_HEADER_READER);
    g.set_u32(reader + READER_DATA, stream as u32)?; // stw r29,80(r1)
    g.set_u32(reader + READER_CURSOR, 0)?; // stw r27,84(r1)
    let state = read_bits(g, reader, 4)?;
    g.set_u8(voice.wrapping_add(VOICE_STATE), state as u8)?;
    let format = read_bits(g, reader, 4)?;
    g.set_u8(voice.wrapping_add(VOICE_FORMAT_INDEX), format as u8)?;
    let kind = read_bits(g, reader, 6)?;
    g.set_u8(slot.wrapping_add(SLOT_TYPE), kind.wrapping_add(1) as u8)?; // addi r9,r3,1 ; stb
    let wide = read_bits(g, reader, 18)?;
    fpscr.disable_flush_mode_unconditional();
    let converted = f64::from((wide & 0xFFFF_FFFF) as i64 as f64 as f32); // clrldi ; std ; lfd ; fcfid ; frsp
    crate::fp::store_single(g, slot.wrapping_add(SLOT_FIELD_16), converted)?; // stfs f12,16(r30)
    let mode = read_bits(g, reader, 2)?;
    g.set_u8(voice.wrapping_add(VOICE_MODE), mode as u8)?;
    let has_cursor = read_bits(g, reader, 1)? & 0xFF; // clrlwi r28,r3,24
    let field20 = read_bits(g, reader, 29)?;
    g.set_u32(slot.wrapping_add(SLOT_FIELD_20), field20 as u32)?;
    if has_cursor != 0 {
        let cursor = read_bits(g, reader, 32)?;
        g.set_u32(slot.wrapping_add(SLOT_CURSOR), cursor as u32)?;
    } else {
        g.set_u32(slot.wrapping_add(SLOT_CURSOR), u32::MAX)?; // loc_82B33700
    }
    let mode = g.u8(voice.wrapping_add(VOICE_MODE))?; // lbz r28,73(r31) -- reloaded
    if mode == 2 {
        let field16 = read_bits(g, reader, 32)?;
        g.set_u32(voice.wrapping_add(VOICE_FIELD_16), field16 as u32)?;
    }
    if has_cursor != 0 {
        let read_delta = match mode {
            1 => true,
            2 => {
                let cursor = g.u32(slot.wrapping_add(SLOT_CURSOR))? as i32; // lwz r11,24(r30)
                let limit = g.u32(voice.wrapping_add(VOICE_FIELD_16))? as i32; // lwz r10,16(r31)
                cursor >= limit // cmpw ; bge
            }
            _ => false,
        };
        let delta = if read_delta {
            read_bits(g, reader, 32)? as u32
        } else {
            0
        };
        g.set_u32(voice.wrapping_add(VOICE_DELTA), delta)?;
    }
    let header_bytes = u64::from(g.u32(reader + READER_CURSOR)? >> 3); // lwz r11,84(r1) ; rlwinm 29,3,31
    g.set_u32(
        voice.wrapping_add(VOICE_DATA_BASE),
        header_bytes.wrapping_add(stream) as u32,
    )?;
    Ok(())
}

// ============================================================ sub_82B474B8: seeking a record

/// `stwu r1,-272(r1)`. Real: the four run-length decoders and their shared cursor live in it.
pub const SEEK_FRAME: u32 = 272;
const SEEK_DECODED: u32 = 80;
const SEEK_CURSOR: u32 = 96;
/// Published: 0, or the entry value of this word plus stream 1's running sum.
pub const SEEK_PUBLISH_BASE: u32 = 4;
/// Published: the record's start position.
pub const SEEK_PUBLISH_POSITION: u32 = 8;
/// Published: `(target - length) - position`.
pub const SEEK_PUBLISH_SKIP: u32 = 12;
/// Published: `min(target - position, [+28])`, signed.
pub const SEEK_PUBLISH_LENGTH: u32 = 16;
/// Published: stream 0's running sum.
pub const SEEK_PUBLISH_OFFSET: u32 = 20;
/// Read only, twice: caps the published length.
pub const SEEK_MIN_LENGTH: u32 = 28;
/// Published byte: 1 when the mode word is exactly 1.
pub const SEEK_PUBLISH_EXACT: u32 = 32;

/// Walk four interleaved run-length streams until the record containing `target` is found, and
/// publish it (`sub_82B474B8`). Returns 1 when a negative record length ends the walk, 0 when the
/// walk passes the target.
///
/// `object` is `r3`, `stream` `r4`, `target` `r5`, `sp` `r1`. The four decoders are built in the
/// frame over one shared cursor, primed with [`decode_four`], then stepped with [`run_length_step`]
/// in stream order: 0 is an offset, 1 a base, 2 the record length and 3 a mode. A record is published
/// when it contains `max(target - [+28], 0)` — or on every record when the mode is 1 — and the walk
/// keeps going until it passes the target, so a later match overwrites an earlier one. Every compare
/// is signed on the low words; the sums are 64-bit and truncated at the stores.
pub fn seek_record(g: &mut Guest, object: u32, stream: u64, target: u64, sp: u32) -> Result<u64> {
    let frame = sp.wrapping_sub(SEEK_FRAME);
    g.set_u32(frame, sp)?; // stwu r1,-272(r1)
    let shared = frame.wrapping_add(SEEK_CURSOR);
    g.set_u32(shared, stream as u32)?; // stw r4,96(r1)
    let min_entry = u64::from(g.u32(object.wrapping_add(SEEK_MIN_LENGTH))?); // lwz r10,28(r3)
    for offset in FIELD_OFFSETS {
        let sub = shared.wrapping_add(offset);
        g.set_u32(sub + RUN_CURSOR, 0)?;
        g.set_u32(sub + RUN_ACCUMULATED, 0)?;
        g.set_u32(sub + RUN_REMAINING, 0)?;
        g.set_u8(sub + RUN_FLAG, 0)?;
    }
    for offset in FIELD_OFFSETS {
        g.set_u32(shared.wrapping_add(offset) + RUN_CURSOR, shared)?; // all four at r1+96
    }
    // subf ; subfic ; rlwinm ; addme ; and -- the span when it is a positive int32, else zero.
    let span = target.wrapping_sub(min_entry);
    let carry = u64::from(span as u32 == 0);
    let sign = u64::from((span as u32) >> 31);
    let limit = (sign + carry).wrapping_sub(1) & span;
    let decoded = frame.wrapping_add(SEEK_DECODED);
    decode_four(g, decoded, shared)?; // bl 0x82b47658
    let mut step = u64::from(g.u32(decoded + 8)?); // lwz r28,88(r1)
    let base_entry = u64::from(g.u32(object.wrapping_add(SEEK_PUBLISH_BASE))?); // before any store
    let mut result = 1;
    if step as u32 as i32 >= 0 {
        let mut mode = u64::from(g.u32(decoded + 12)?);
        let mut value_b = u64::from(g.u32(decoded + 4)?);
        let mut value_a = u64::from(g.u32(decoded)?);
        let (mut position, mut sum_a, mut sum_b) = (0u64, 0u64, 0u64);
        loop {
            let in_range = (position as u32 as i32) <= (limit as u32 as i32)
                && (limit as u32 as i32) < (step.wrapping_add(position) as u32 as i32);
            if in_range || mode as u32 as i32 == 1 {
                let published = if value_b as u32 as i32 == 0 {
                    0
                } else {
                    sum_b.wrapping_add(base_entry)
                };
                g.set_u32(object.wrapping_add(SEEK_PUBLISH_BASE), published as u32)?;
                let min_length = u64::from(g.u32(object.wrapping_add(SEEK_MIN_LENGTH))?); // reloaded
                let remaining = target.wrapping_sub(position);
                g.set_u32(object.wrapping_add(SEEK_PUBLISH_POSITION), position as u32)?;
                let length = if (remaining as u32 as i32) < (min_length as u32 as i32) {
                    remaining
                } else {
                    min_length
                };
                let leading = (mode.wrapping_sub(1) as u32).leading_zeros(); // cntlzw
                g.set_u32(object.wrapping_add(SEEK_PUBLISH_LENGTH), length as u32)?;
                let tail = target.wrapping_sub(length);
                g.set_u32(object.wrapping_add(SEEK_PUBLISH_OFFSET), sum_a as u32)?;
                g.set_u32(
                    object.wrapping_add(SEEK_PUBLISH_SKIP),
                    tail.wrapping_sub(position) as u32,
                )?;
                g.set_u8(
                    object.wrapping_add(SEEK_PUBLISH_EXACT),
                    ((leading >> 5) & 1) as u8,
                )?;
            }
            position = step.wrapping_add(position); // add r29,r28,r29
            if (target as u32 as i32) < (position as u32 as i32) {
                result = 0; // walked past the target
                break;
            }
            sum_a = value_a.wrapping_add(sum_a);
            sum_b = value_b.wrapping_add(sum_b);
            value_a = run_length_step(g, shared.wrapping_add(FIELD_OFFSETS[0]))?;
            value_b = run_length_step(g, shared.wrapping_add(FIELD_OFFSETS[1]))?;
            step = run_length_step(g, shared.wrapping_add(FIELD_OFFSETS[2]))?;
            mode = run_length_step(g, shared.wrapping_add(FIELD_OFFSETS[3]))?;
            if (step as u32 as i32) < 0 {
                break; // a negative record length ends the walk, with 1
            }
        }
    }
    Ok(result)
}

// ============================================================ sub_82B50100: seeking a packet

/// `stwu r1,-160(r1)`. Real: the bit cell at `r1 + 80` and the packet pointer at `r1 + 88` are the
/// two callees' arguments.
pub const PACKET_FRAME: u32 = 160;
const PACKET_BITS_CELL: u32 = 80;
const PACKET_POINTER_CELL: u32 = 88;
/// Cursor `+4`: the packet the target frame starts in.
pub const PACKET_CURSOR_PACKET: u32 = 4;
/// Cursor `+8`: bytes from that packet to the end of the chunk, less four.
pub const PACKET_CURSOR_REMAINING: u32 = 8;
/// Cursor `+20`: the bit offset inside that packet, or -1.
pub const PACKET_CURSOR_BITS: u32 = 20;
/// Descriptor `+4`: the seek table, or 0.
pub const PACKET_DESC_TABLE: u32 = 4;
/// Descriptor `+8`: the sample position.
pub const PACKET_DESC_SAMPLES: u32 = 8;
/// Descriptor `+20`: clear adds the 384-sample bias before the divide.
pub const PACKET_DESC_FLAG: u32 = 20;
/// The packet size.
pub const PACKET_BYTES: u64 = 2048;
/// At or past this the bit position is poisoned to -1.
pub const PACKET_PAGE_BITS: i32 = 16_352;

/// `srawi ; addze` — a signed divide of the low word by `2^shift`, truncating toward zero.
fn divide_toward_zero(value: u32, shift: u32) -> i64 {
    let signed = value as i32;
    let carry = signed < 0 && value & ((1 << shift) - 1) != 0;
    i64::from(signed >> shift) + i64::from(carry)
}

/// `lwz ; srawi r6,r7,11 ; clrlwi r10,r6,17 ; cmpwi 16352 ; blt ; li -1` — the four header bytes,
/// just stored into the cell, read back as one big-endian word.
fn bit_position_from_cell(g: &Guest, cell: u32) -> Result<i64> {
    let word = g.u32(cell)?;
    let value = (((word as i32) >> 11) as u32 & 0x7FFF) as i32;
    Ok(if value < PACKET_PAGE_BITS {
        i64::from(value)
    } else {
        -1
    })
}

/// Store a packet's four header bytes into the cell, in the original's order, and turn them into a
/// bit position stored back over them.
fn load_packet_header(g: &mut Guest, cell: u32, packet: u32, order: [u32; 4]) -> Result<i64> {
    let mut bytes = [0u8; 4];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = g.u8(packet.wrapping_add(i as u32))?;
    }
    for i in order {
        g.set_u8(cell + i, bytes[i as usize])?;
    }
    let position = bit_position_from_cell(g, cell)?;
    g.set_u32(cell, position as u32)?;
    Ok(position)
}

/// Seek a packetised bit stream to a frame, recording the packet, the bytes left and the bit offset
/// in the cursor record (`sub_82B50100`). Returns the chunk's byte length, the length field with its
/// two flag bits shifted out.
///
/// `cursor` is `r3`, `substream` `r4`, `desc` `r5`, `chunk` `r6` (full width: the packet arithmetic is
/// 64-bit and truncated at the stores), `sp` `r1`. The sample position becomes 512-sample frames,
/// biased by 384 samples when the descriptor's flag is clear. With a seek table, the substream's
/// entry is walked with [`decode_unsigned`] one packet at a time while the frames counted so far do
/// not pass the target — signed compares on the low words — and the chosen packet's header gives a
/// fresh bit position. Each frame still ahead is then skipped with [`advance_bit_cursor`], which may
/// step the packet pointer across a page wrap.
pub fn seek_packet(
    g: &mut Guest,
    cursor: u32,
    substream: u32,
    desc: u32,
    chunk: u64,
    sp: u32,
) -> Result<u64> {
    let frame = sp.wrapping_sub(PACKET_FRAME);
    g.set_u32(frame, sp)?; // stwu r1,-160(r1)
    let bits_cell = frame.wrapping_add(PACKET_BITS_CELL);
    let pointer_cell = frame.wrapping_add(PACKET_POINTER_CELL);
    let chunk32 = chunk as u32;
    let samples = g.u32(desc.wrapping_add(PACKET_DESC_SAMPLES))?; // lwz r11,8(r5)
    let length_field = g.u32(chunk32)?; // lwz r10,0(r6)
    let flag = g.u8(desc.wrapping_add(PACKET_DESC_FLAG))?; // lbz r9,20(r5)
    let chunk_bytes = u64::from(length_field >> 2); // rlwinm r25,r10,30,2,31
    let mut frames = if flag == 0 {
        divide_toward_zero(samples.wrapping_add(384), 9) // addi r11,r11,384 ; srawi ; addze
    } else {
        divide_toward_zero(samples, 9)
    };
    let payload = chunk.wrapping_add(4); // addi r27,r6,4
    let mut packet = payload;
    g.set_u32(pointer_cell, packet as u32)?; // stw r27,88(r1)
    // lbz 5,4,6,7 ; stb 81,80,82,83 -- the header of the chunk's first packet.
    load_packet_header(g, bits_cell, payload as u32, [1, 0, 2, 3])?;
    let table = g.u32(desc.wrapping_add(PACKET_DESC_TABLE))?; // lwz r11,4(r5)
    let mut consumed = 0u64;
    if table != 0 {
        let entry = g.u32((substream << 2).wrapping_add(table))?; // lwzx
        let mut reader = u64::from(entry).wrapping_add(u64::from(table)); // add r30,r10,r11
        let length = decode_unsigned(g, reader as u32, bits_cell)?; // bl 0x82b4f5f8
        let mut counted = u64::from(g.u32(bits_cell)?);
        reader = length.wrapping_add(reader);
        if (frames as u32 as i32) >= (counted as u32 as i32) {
            loop {
                packet = packet.wrapping_add(PACKET_BYTES); // addi r29,r29,2048
                consumed = consumed.wrapping_add(counted);
                let length = decode_unsigned(g, reader as u32, bits_cell)?;
                counted = u64::from(g.u32(bits_cell)?);
                reader = length.wrapping_add(reader);
                if (frames as u32 as i32) < (counted.wrapping_add(consumed) as u32 as i32) {
                    break; // add r10,r11,r31 ; cmpw ; bge
                }
            }
            g.set_u32(pointer_cell, packet as u32)?; // stw r29,88(r1)
        }
        frames = (frames as u64).wrapping_sub(consumed) as i64; // subf r28,r31,r28
        // lbz 0,2,1,3 ; stb 80,82,81,83 -- the chosen packet's header.
        load_packet_header(g, bits_cell, packet as u32, [0, 2, 1, 3])?;
    }
    if frames as u32 as i32 > 0 {
        loop {
            advance_bit_cursor(g, pointer_cell, bits_cell)?; // bl 0x82b50270
            frames = frames.wrapping_sub(1);
            if frames as u32 as i32 == 0 {
                break; // addic. r28,r28,-1 -- the low word
            }
        }
        packet = u64::from(g.u32(pointer_cell)?); // lwz r29,88(r1)
    }
    let behind = payload.wrapping_sub(packet); // subf r11,r29,r27
    let bits = g.u32(bits_cell)?; // lwz r10,80(r1)
    g.set_u32(cursor.wrapping_add(PACKET_CURSOR_PACKET), packet as u32)?; // stw r29,4(r26)
    let remaining = behind.wrapping_add(chunk_bytes); // add r11,r11,r25
    g.set_u32(cursor.wrapping_add(PACKET_CURSOR_BITS), bits)?; // stw r10,20(r26)
    g.set_u32(
        cursor.wrapping_add(PACKET_CURSOR_REMAINING),
        remaining.wrapping_sub(4) as u32,
    )?; // addi -4 ; stw
    Ok(chunk_bytes) // mr r3,r25
}

// ======================================================= sub_82B472C0: seeking fixed records

/// One record: `{offset, base, length, mode}`, big-endian words read byte by byte.
pub const FIXED_RECORD_BYTES: u32 = 16;

fn record_word(g: &Guest, at: u32) -> Result<u64> {
    let mut word = 0u32;
    for i in 0..4 {
        word = (word << 8) | u32::from(g.u8(at.wrapping_add(i))?);
    }
    Ok(u64::from(word))
}

fn fixed_record(g: &Guest, at: u32) -> Result<[u64; 4]> {
    Ok([
        record_word(g, at)?,
        record_word(g, at.wrapping_add(4))?,
        record_word(g, at.wrapping_add(8))?,
        record_word(g, at.wrapping_add(12))?,
    ])
}

/// Walk a table of fixed 16-byte seek records until the one containing a target is found, and
/// publish it (`sub_82B472C0`) — [`seek_record`]'s twin over a table instead of four run-length
/// streams. Returns 1 when a negative length ends the walk, 0 when it passes the target.
///
/// `object` is `r3`, `records` `r4`, `target` `r5`. The publish set, the containment test and the
/// mode-1 rule are [`seek_record`]'s; the one difference in the arithmetic is that the minimum
/// length at `+28` is read **once**, where [`seek_record`] reloads it. Each record's words are read
/// after the previous record's publish, byte by byte, so an unaligned table reads the same.
pub fn seek_fixed_records(g: &mut Guest, object: u32, records: u32, target: u64) -> Result<u64> {
    let min_length = u64::from(g.u32(object.wrapping_add(SEEK_MIN_LENGTH))?); // lwz r30,28(r9)
    let span = target.wrapping_sub(min_length);
    let carry = u64::from(span as u32 == 0);
    let sign = u64::from((span as u32) >> 31);
    let limit = (sign + carry).wrapping_sub(1) & span;
    let base_entry = u64::from(g.u32(object.wrapping_add(SEEK_PUBLISH_BASE))?); // lwz r27,4(r9)
    let mut record = fixed_record(g, records)?;
    let mut result = 1;
    if record[2] as u32 as i32 >= 0 {
        let (mut position, mut sum_a, mut sum_b) = (0u64, 0u64, 0u64);
        let mut cursor = records;
        loop {
            let [offset, base_word, length, mode] = record;
            let in_range = (position as u32 as i32) <= (limit as u32 as i32)
                && (limit as u32 as i32) < (length.wrapping_add(position) as u32 as i32);
            if in_range || mode as u32 as i32 == 1 {
                let published = if base_word as u32 as i32 == 0 {
                    0
                } else {
                    sum_b.wrapping_add(base_entry)
                };
                g.set_u32(object.wrapping_add(SEEK_PUBLISH_BASE), published as u32)?;
                let remaining = target.wrapping_sub(position);
                g.set_u32(object.wrapping_add(SEEK_PUBLISH_POSITION), position as u32)?;
                let published_length = if (remaining as u32 as i32) < (min_length as u32 as i32) {
                    remaining
                } else {
                    min_length
                };
                let leading = (mode.wrapping_sub(1) as u32).leading_zeros();
                g.set_u32(
                    object.wrapping_add(SEEK_PUBLISH_LENGTH),
                    published_length as u32,
                )?;
                let tail = target.wrapping_sub(published_length);
                g.set_u32(object.wrapping_add(SEEK_PUBLISH_OFFSET), sum_a as u32)?;
                g.set_u32(
                    object.wrapping_add(SEEK_PUBLISH_SKIP),
                    tail.wrapping_sub(position) as u32,
                )?;
                g.set_u8(
                    object.wrapping_add(SEEK_PUBLISH_EXACT),
                    ((leading >> 5) & 1) as u8,
                )?;
            }
            position = length.wrapping_add(position); // add r10,r31,r10
            if (target as u32 as i32) < (position as u32 as i32) {
                result = 0;
                break;
            }
            sum_b = base_word.wrapping_add(sum_b); // add r6,r29,r6
            sum_a = offset.wrapping_add(sum_a); // add r7,r8,r7
            cursor = cursor.wrapping_add(FIXED_RECORD_BYTES);
            record = fixed_record(g, cursor)?;
            if (record[2] as u32 as i32) < 0 {
                break;
            }
        }
    }
    Ok(result)
}

// ====================================================== sub_82B471D8: the seek-header dispatch

/// `stwu r1,-128(r1)`. Real: [`seek_record`]'s frame sits below it.
pub const SEEK_HEADER_FRAME: u32 = 128;
/// `stw r7,0(r3)` — the header's address plus 12.
pub const SEEK_HEADER_END: u32 = 0;
/// `stw r7,24(r3)` — the low nibble of byte 1.
pub const SEEK_HEADER_KIND: u32 = 24;
/// The deepest stack the dispatch reaches.
pub const SEEK_HEADER_STACK_DEPTH: u32 = SEEK_HEADER_FRAME + SEEK_FRAME;

fn header_word(g: &Guest, at: u32) -> Result<u32> {
    Ok(record_word(g, at)? as u32)
}

/// Parse a seek header and hand the walk on (`sub_82B471D8`). Returns the walker's result, or 0.
///
/// `object` is `r3`, `header` `r4` at full width (the pointer sums are 64-bit), `target` `r5`, `sp`
/// `r1`. Byte 1's low nibble goes to `+24` **before** bytes 2 to 11 are read; bytes 2..3 are the
/// minimum length at `+28`, bytes 4..7 the records' offset and bytes 8..11 the data's (zero stays
/// zero) at `+4`, and `+0` gets the header's end. Byte 1's high nibble then picks
/// [`seek_fixed_records`] (0) or [`seek_record`] (1); anything else returns 0.
pub fn parse_seek_header(
    g: &mut Guest,
    object: u32,
    header: u64,
    target: u64,
    sp: u32,
) -> Result<u64> {
    let at = header as u32;
    let frame = sp.wrapping_sub(SEEK_HEADER_FRAME);
    g.set_u32(frame, sp)?; // stwu r1,-128(r1)
    let byte1 = u32::from(g.u8(at.wrapping_add(1))?); // lbz r8,1(r4)
    g.set_u32(object.wrapping_add(SEEK_HEADER_KIND), byte1 & 0xF)?; // stw r7,24(r3)
    let min_length =
        (u32::from(g.u8(at.wrapping_add(2))?) << 8) | u32::from(g.u8(at.wrapping_add(3))?);
    let stream_offset = header_word(g, at.wrapping_add(4))?; // lwz r11,88(r1)
    let data_offset = header_word(g, at.wrapping_add(8))?; // lwz r9,96(r1)
    let stream = u64::from(stream_offset).wrapping_add(header); // add r4,r11,r10
    g.set_u32(object.wrapping_add(SEEK_MIN_LENGTH), min_length)?; // stw r6,28(r3)
    let data = if data_offset == 0 {
        0
    } else {
        u64::from(data_offset).wrapping_add(header)
    };
    g.set_u32(object.wrapping_add(SEEK_PUBLISH_BASE), data as u32)?; // stw r11,4(r3)
    g.set_u32(
        object.wrapping_add(SEEK_HEADER_END),
        header.wrapping_add(12) as u32,
    )?; // stw r7,0(r3)
    match byte1 >> 4 {
        0 => seek_fixed_records(g, object, stream as u32, target), // bl 0x82b472c0
        1 => seek_record(g, object, stream, target, frame),        // bl 0x82b474b8
        _ => Ok(0),                                                // li r3,0
    }
}

// =================================================== sub_82B470D0: the stream-header dispatch

/// `stwu r1,-128(r1)`.
pub const STREAM_HEADER_FRAME: u32 = 128;
/// The deepest stack the dispatch reaches.
pub const STREAM_HEADER_STACK_DEPTH: u32 = STREAM_HEADER_FRAME + SEEK_HEADER_STACK_DEPTH;

/// Parse a stream header by its version byte (`sub_82B470D0`). Returns the status; any non-zero
/// **low byte** wipes six of the object's words.
///
/// `object` is `r3`, `stream` `r4`, `target` `r5` (it flows through untouched to whichever walker
/// runs), `sp` `r1`. The version byte is sign-extended and compared unsigned, so only 0 and 1 have
/// paths of their own: 1 is [`parse_seek_header`], and 0 is its inline twin — `+0` and `+24` written
/// **before** bytes 2..7 are read, the payload at `stream + 8`, the high nibble picking the walker.
/// The wipe clears `+0, +4, +8, +12, +20, +24` and leaves `+16`, `+28` and `+32`.
pub fn parse_stream_header(
    g: &mut Guest,
    object: u32,
    stream: u32,
    target: u64,
    sp: u32,
) -> Result<u64> {
    let frame = sp.wrapping_sub(STREAM_HEADER_FRAME);
    g.set_u32(frame, sp)?; // stwu r1,-128(r1)
    let version = g.u8(stream)? as i8 as i32 as u32; // lbz ; extsb ; cmplwi
    let status = if version > 1 {
        1 // li r3,1
    } else if version == 1 {
        parse_seek_header(g, object, u64::from(stream), target, frame)? // bl 0x82b471d8
    } else {
        let nibbles = u32::from(g.u8(stream.wrapping_add(1))?); // lbz r9,1(r10)
        g.set_u32(object, 0)?; // stw r30,0(r31)
        g.set_u32(object.wrapping_add(SEEK_HEADER_KIND), nibbles & 0xF)?; // stw r8,24(r31)
        let field28 = (u32::from(g.u8(stream.wrapping_add(2))?) << 8)
            | u32::from(g.u8(stream.wrapping_add(3))?);
        let offset = header_word(g, stream.wrapping_add(4))?;
        g.set_u32(object.wrapping_add(SEEK_MIN_LENGTH), field28)?; // stw r29,28(r31)
        let data = if offset != 0 {
            u64::from(offset).wrapping_add(u64::from(stream))
        } else {
            0
        };
        g.set_u32(object.wrapping_add(SEEK_PUBLISH_BASE), data as u32)?; // stw r11,4(r31)
        let payload = stream.wrapping_add(8);
        match nibbles >> 4 {
            0 => seek_fixed_records(g, object, payload, target)?, // bl 0x82b472c0
            1 => seek_record(g, object, u64::from(payload), target, frame)?, // bl 0x82b474b8
            _ => 0,                                               // mr r3,r30 -- not a failure
        }
    };
    if status & 0xFF != 0 {
        for offset in [0, 4, 8, 12, 20, 24] {
            g.set_u32(object.wrapping_add(offset), 0)?; // the failure wipe
        }
    }
    Ok(status)
}

// ===================================================== sub_82B33780: one packet header decoded

/// `stwu r1,-144(r1)`. Real: the 33-byte output buffer at `r1 + 80` is handed to
/// [`parse_stream_header`].
pub const PACKET_HEADER_FRAME: u32 = 144;
const PACKET_HEADER_OUT: u32 = 80;
/// The deepest stack the decode reaches.
pub const PACKET_HEADER_STACK_DEPTH: u32 = PACKET_HEADER_FRAME + STREAM_HEADER_STACK_DEPTH;

/// Decode one packet header for stream `index` and scatter its fields into the object's 48-byte
/// slot and the array's 80-byte entry (`sub_82B33780`).
///
/// `object` is `r3`, `index` `r4`, `bytes` `r5`, `length` `r6` — the length is also the target the
/// walkers seek, since it reaches them as `r5`. The two record addresses are computed once at entry,
/// with 64-bit adds truncated only at the stores. A positive length and a non-null pointer decode
/// into the frame and copy seven fields out, the slot's `+36` **re-read** before it is copied to the
/// entry's `+20`; otherwise both records are cleared and the entry is marked 1.
pub fn decode_packet_header(
    g: &mut Guest,
    object: u64,
    index: u64,
    bytes: u64,
    length: u64,
    sp: u32,
) -> Result<()> {
    let obj = object as u32;
    let slot_base = u64::from(g.u16(obj.wrapping_add(HEADER_SLOT_TABLE))?); // lhz r9,464(r3)
    let array = u64::from(g.u32(obj.wrapping_add(HEADER_VOICE_ARRAY))?); // lwz r10,96(r3)
    let triple = index.wrapping_add(u64::from((index as u32).wrapping_mul(2))); // rlwinm ; add
    let quint = index.wrapping_add(u64::from((index as u32).wrapping_mul(4)));
    let slot = u64::from((triple as u32).wrapping_mul(16))
        .wrapping_add(slot_base)
        .wrapping_add(object) as u32;
    let entry = u64::from((quint as u32).wrapping_mul(16)).wrapping_add(array) as u32;
    let frame = sp.wrapping_sub(PACKET_HEADER_FRAME);
    g.set_u32(frame, sp)?; // stwu r1,-144(r1)
    let out = frame.wrapping_add(PACKET_HEADER_OUT);
    let at = |base: u32, offset: u32| base.wrapping_add(offset);
    if (length as u32 as i32) > 0 && bytes as u32 != 0 {
        parse_stream_header(g, out, bytes as u32, length, frame)?; // bl 0x82b470d0
        let at88 = g.u32(at(out, 8))?; // lwz r10,88(r1)
        let at92 = g.u32(at(out, 12))?; // lwz r9,92(r1)
        let at96 = g.u32(at(out, 16))?; // lwz r8,96(r1)
        let at100 = g.u32(at(out, 20))?; // lwz r7,100(r1)
        let at84 = g.u32(at(out, 4))?; // lwz r6,84(r1)
        let at104 = g.u32(at(out, 24))?; // lwz r5,104(r1)
        let at112 = g.u8(at(out, 32))?;
        g.set_u32(at(slot, 36), at88)?; // stw r10,36(r30)
        g.set_u32(at(slot, 32), at92)?; // stw r9,32(r30)
        g.set_u32(at(entry, 60), at96)?; // stw r8,60(r31)
        g.set_u32(at(entry, 64), at100)?; // stw r7,64(r31)
        g.set_u32(at(entry, 56), at84)?; // stw r6,56(r31)
        g.set_u32(at(entry, 68), at104)?; // stw r5,68(r31)
        g.set_u8(at(entry, 77), at112)?; // stb r4,77(r31)
        let published = g.u32(at(slot, 36))?; // lwz r3,36(r30) -- re-read
        g.set_u32(at(slot, 28), 0)?; // stw r11,28(r30)
        g.set_u32(at(entry, 20), published)?; // stw r3,20(r31)
    } else {
        g.set_u32(at(slot, 32), 0)?;
        g.set_u32(at(entry, 60), 0)?;
        g.set_u32(at(entry, 64), 0)?;
        g.set_u32(at(entry, 56), 0)?;
        g.set_u8(at(entry, 77), 1)?;
        g.set_u32(at(slot, 28), 0)?;
        g.set_u32(at(slot, 36), 0)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;

    // ---------------------------------------------------------------- independent encoders

    /// The unsigned code, written from the module description rather than from the decoder.
    fn encode_unsigned(v: u32) -> Vec<u8> {
        if v < 192 {
            vec![v as u8]
        } else if v < 12_480 {
            let x = v - 192;
            vec![0xC0 | (x >> 8) as u8, x as u8]
        } else if v < 798_912 {
            let x = v - 12_480;
            vec![0xF0 | (x >> 16) as u8, (x >> 8) as u8, x as u8]
        } else if v - 798_912 < 0x300_0000 {
            let x = v - 798_912;
            vec![
                0xFC | (x >> 24) as u8,
                (x >> 16) as u8,
                (x >> 8) as u8,
                x as u8,
            ]
        } else {
            let mut out = vec![0xFF];
            out.extend_from_slice(&v.to_be_bytes());
            out
        }
    }

    /// The signed zigzag code, likewise independent: sign in the last byte's low bit.
    fn encode_signed(n: i32) -> Vec<u8> {
        let (m, s) = if n >= 0 {
            (n as u32, 0u32)
        } else {
            (!n as u32, 1u32)
        };
        if m < 96 {
            vec![(2 * m + s) as u8]
        } else if m < 6_240 {
            let x = 2 * (m - 96) + s;
            vec![0xC0 | (x >> 8) as u8, x as u8]
        } else if m < 399_456 {
            let x = 2 * (m - 6_240) + s;
            vec![0xF0 | (x >> 16) as u8, (x >> 8) as u8, x as u8]
        } else if m - 399_456 < 0x180_0000 {
            let x = 2 * (m - 399_456) + s;
            vec![
                0xFC | (x >> 24) as u8,
                (x >> 16) as u8,
                (x >> 8) as u8,
                x as u8,
            ]
        } else {
            let mut out = vec![0xFF];
            out.extend_from_slice(&n.to_be_bytes());
            out
        }
    }

    fn with_bytes(bytes: &[u8]) -> Guest {
        let mut g = Guest::single(BASE, 0x4000);
        g.set_span(BASE + 0x100, bytes).unwrap();
        g
    }

    #[test]
    fn the_unsigned_code_round_trips_across_every_width_boundary() {
        let values = [
            0u32,
            1,
            191,
            192,
            193,
            12_479,
            12_480,
            12_481,
            798_911,
            798_912,
            798_913,
            798_912 + 0x2FF_FFFF,
            798_912 + 0x300_0000,
            0x7FFF_FFFF,
            0xFFFF_FFFF,
            1_000,
            500_000,
        ];
        for v in values {
            let code = encode_unsigned(v);
            let g = with_bytes(&code);
            assert_eq!(
                unsigned_code(&g, BASE + 0x100).unwrap(),
                (v, code.len() as u32),
                "{v}"
            );
        }
    }

    #[test]
    fn the_signed_code_round_trips_across_every_width_boundary() {
        let values = [
            0i32,
            1,
            -1,
            95,
            -96,
            96,
            -97,
            6_239,
            -6_240,
            6_240,
            -6_241,
            399_455,
            -399_456,
            399_456,
            -399_457,
            399_456 + 0x17F_FFFF,
            -(399_456 + 0x17F_FFFF) - 1,
            i32::MAX,
            i32::MIN,
            12_345,
            -54_321,
        ];
        for n in values {
            let code = encode_signed(n);
            let g = with_bytes(&code);
            let (value, length) = signed_code(&g, BASE + 0x100).unwrap();
            assert_eq!((value as i32, length), (n, code.len() as u32), "{n}");
        }
    }

    #[test]
    fn every_width_starts_where_the_previous_one_ends() {
        // The biases make the widths contiguous: the smallest two-byte code is 192, the largest
        // one-byte code 191, and so on. A bias off by one leaves a gap or an overlap here.
        let first = |bytes: &[u8]| unsigned_code(&with_bytes(bytes), BASE + 0x100).unwrap().0;
        assert_eq!(first(&[0xBF]), 191);
        assert_eq!(first(&[0xC0, 0x00]), 192);
        assert_eq!(first(&[0xEF, 0xFF]), 12_479);
        assert_eq!(first(&[0xF0, 0x00, 0x00]), 12_480);
        assert_eq!(first(&[0xFB, 0xFF, 0xFF]), 798_911);
        assert_eq!(first(&[0xFC, 0x00, 0x00, 0x00]), 798_912);
    }

    #[test]
    fn the_five_byte_forms_are_the_raw_word_with_no_zigzag() {
        let g = with_bytes(&[0xFF, 0x80, 0x00, 0x00, 0x01]);
        assert_eq!(
            signed_code(&g, BASE + 0x100).unwrap(),
            (0x8000_0001, 5),
            "not zigzagged"
        );
        assert_eq!(
            unsigned_code(&g, BASE + 0x100).unwrap(),
            (0x8000_0001, 5),
            "not biased"
        );
    }

    #[test]
    fn the_guest_facing_decoders_store_the_value_and_return_the_length() {
        let mut g = with_bytes(&encode_signed(-7_000));
        assert_eq!(decode_signed(&mut g, BASE + 0x100, BASE + 0x10).unwrap(), 3);
        assert_eq!(g.u32(BASE + 0x10).unwrap() as i32, -7_000);

        let mut g = with_bytes(&encode_unsigned(70_000));
        assert_eq!(
            decode_unsigned(&mut g, BASE + 0x100, BASE + 0x10).unwrap(),
            3
        );
        assert_eq!(g.u32(BASE + 0x10).unwrap(), 70_000);
    }

    // ------------------------------------------------------------------------ the bit reader

    const READER: u32 = BASE;
    const DATA: u32 = BASE + 0x100;

    /// An MSB-first reader that works one bit at a time — nothing like the byte-chunked original.
    fn model_bits(data: &[u8], cursor: u32, bits: u32) -> u32 {
        let mut v = 0u64;
        for i in 0..bits {
            let p = (cursor + i) as usize;
            v = (v << 1) | u64::from((data[p / 8] >> (7 - p % 8)) & 1);
        }
        v as u32
    }

    fn pattern() -> Vec<u8> {
        let mut x = 0x9E37_79B9u32;
        (0..64)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    #[test]
    fn read_bits_matches_a_bit_at_a_time_model() {
        let data = pattern();
        for cursor in [0u32, 1, 3, 7, 8, 13, 100] {
            for bits in [1u32, 2, 5, 7, 8, 9, 15, 16, 17, 24, 31, 32] {
                let mut g = Guest::single(BASE, 0x400);
                g.set_span(DATA, &data).unwrap();
                g.set_u32(READER + READER_DATA, DATA).unwrap();
                g.set_u32(READER + READER_CURSOR, cursor).unwrap();

                let got = read_bits(&mut g, READER, u64::from(bits)).unwrap();

                assert_eq!(
                    got as u32,
                    model_bits(&data, cursor, bits),
                    "cursor {cursor}, {bits} bits"
                );
                assert_eq!(
                    g.u32(READER + READER_CURSOR).unwrap(),
                    cursor + bits,
                    "cursor advanced"
                );
            }
        }
    }

    #[test]
    fn a_zero_bit_read_still_loads_a_byte_and_leaves_the_cursor() {
        // The do-while runs once: take = 0, the byte is loaded, and the cursor is stored unchanged.
        // With the data unmapped the load is an error, which proves it happens.
        let mut g = Guest::single(BASE, 0x40);
        g.set_u32(READER + READER_DATA, BASE + 0x1000).unwrap(); // not mapped
        g.set_u32(READER + READER_CURSOR, 5).unwrap();
        assert!(
            read_bits(&mut g, READER, 0).is_err(),
            "the byte load happens"
        );

        let mut g = Guest::single(BASE, 0x400);
        g.set_u32(READER + READER_DATA, DATA).unwrap();
        g.set_u32(READER + READER_CURSOR, 5).unwrap();
        assert_eq!(read_bits(&mut g, READER, 0).unwrap(), 0);
        assert_eq!(g.u32(READER + READER_CURSOR).unwrap(), 5);
    }

    // --------------------------------------------------------------------- the run-length layer

    const CURSOR_OBJ: u32 = BASE + 0x40;
    const RUN: u32 = BASE + 0x60;
    const STREAM: u32 = BASE + 0x200;

    fn run_guest(codes: &[i32]) -> Guest {
        let mut g = Guest::single(BASE, 0x1000);
        let bytes: Vec<u8> = codes.iter().flat_map(|c| encode_signed(*c)).collect();
        g.set_span(STREAM, &bytes).unwrap();
        g.set_u32(CURSOR_OBJ + CURSOR_STREAM, STREAM).unwrap();
        g.set_u32(RUN + RUN_CURSOR, CURSOR_OBJ).unwrap();
        g
    }

    /// Expand a stream of run codes the way the module note describes, independently of the port:
    /// a count `c >= 0` is one delta and `c + 1` outputs; a count `c < 0` is `1 - c` deltas, each
    /// with an output.
    fn model_runs(codes: &[i32], outputs: usize) -> Vec<i32> {
        let mut out = Vec::new();
        let mut sum = 0i32;
        let mut i = 0;
        while out.len() < outputs {
            let count = codes[i];
            i += 1;
            if count >= 0 {
                sum = sum.wrapping_add(codes[i]);
                i += 1;
                for _ in 0..=count {
                    out.push(sum);
                }
            } else {
                for _ in 0..(1 - count) {
                    sum = sum.wrapping_add(codes[i]);
                    i += 1;
                    out.push(sum);
                }
            }
        }
        out.truncate(outputs);
        out
    }

    #[test]
    fn a_repeat_run_then_a_literal_run_matches_the_model() {
        let codes = [2, 3, -1, 10, -4, 0, 100];
        let mut g = run_guest(&codes);
        let got: Vec<i32> = (0..6)
            .map(|_| run_length_step(&mut g, RUN).unwrap() as u32 as i32)
            .collect();
        assert_eq!(got, model_runs(&codes, 6));
        assert_eq!(
            got,
            vec![3, 3, 3, 13, 9, 109],
            "spelled out, so the model is checked too"
        );
    }

    #[test]
    fn a_repeat_run_consumes_no_stream_after_its_first_value() {
        let codes = [3, 5];
        let mut g = run_guest(&codes);
        run_length_step(&mut g, RUN).unwrap();
        let after_first = g.u32(CURSOR_OBJ + CURSOR_STREAM).unwrap();
        assert_eq!(
            after_first,
            STREAM + 2,
            "the count and the delta, one byte each"
        );
        for _ in 0..3 {
            assert_eq!(run_length_step(&mut g, RUN).unwrap(), 5);
        }
        assert_eq!(
            g.u32(CURSOR_OBJ + CURSOR_STREAM).unwrap(),
            after_first,
            "nothing read"
        );
        assert_eq!(
            g.u8(RUN + RUN_FLAG).unwrap(),
            1,
            "a repeat run keeps its flag set"
        );
    }

    #[test]
    fn a_negative_count_leaves_the_flag_clear_and_the_run_long() {
        // count -3: remaining = 1 - (-3) = 4, and the first delta is already consumed on this call.
        let mut g = run_guest(&[-3, 1, 1, 1, 1]);
        run_length_step(&mut g, RUN).unwrap();
        assert_eq!(g.u8(RUN + RUN_FLAG).unwrap(), 0);
        assert_eq!(
            g.u32(RUN + RUN_REMAINING).unwrap(),
            3,
            "four values, one already returned"
        );
    }

    #[test]
    fn decode_signed_into_advances_the_stream_and_the_sum() {
        let mut g = run_guest(&[-5_000]);
        g.set_u32(RUN + RUN_ACCUMULATED, 7_000).unwrap();
        assert_eq!(
            decode_signed_into(&mut g, RUN).unwrap(),
            2,
            "a two-byte code"
        );
        assert_eq!(g.u32(RUN + RUN_ACCUMULATED).unwrap(), 2_000);
        assert_eq!(g.u32(CURSOR_OBJ + CURSOR_STREAM).unwrap(), STREAM + 2);
    }

    #[test]
    fn decode_four_steps_each_sub_object_in_turn() {
        // Four sub-objects, each with its own cursor and stream, and one output word each.
        let mut g = Guest::single(BASE, 0x2000);
        let input = BASE + 0x80;
        let out = BASE + 0x10;
        for (i, offset) in FIELD_OFFSETS.iter().enumerate() {
            let cursor = BASE + 0x400 + 0x10 * i as u32;
            let stream = BASE + 0x800 + 0x100 * i as u32;
            let codes = [0, (i as i32 + 1) * 11]; // a repeat run of one, delta 11, 22, 33, 44
            let bytes: Vec<u8> = codes.iter().flat_map(|c| encode_signed(*c)).collect();
            g.set_span(stream, &bytes).unwrap();
            g.set_u32(cursor + CURSOR_STREAM, stream).unwrap();
            g.set_u32(input + offset + RUN_CURSOR, cursor).unwrap();
        }
        decode_four(&mut g, out, input).unwrap();
        for i in 0..4u32 {
            assert_eq!(g.u32(out + 4 * i).unwrap(), (i + 1) * 11, "field {i}");
        }
    }

    // ------------------------------------------------------------------- the paged bit cursor

    const BASE_FIELD: u32 = BASE;
    const CURSOR_FIELD: u32 = BASE + 4;
    const PAGES: u32 = BASE + 0x1000;

    /// The same field read from the *logical* payload stream: logical bit L lives at physical bit
    /// `L + 32` on the first page and `L - 16352 + 16384 + 32` on the next. Independent of the
    /// port's two-read split.
    fn model_field(pages: &[u8], cursor: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..15u32 {
            let l = cursor + i;
            let p = if l < PAGE_BITS {
                l + 32
            } else {
                l - PAGE_BITS + 8 * PAGE_BYTES + 32
            } as usize;
            v = (v << 1) | u32::from((pages[p / 8] >> (7 - p % 8)) & 1);
        }
        v
    }

    fn page_guest(pages: &[u8], cursor: u32) -> Guest {
        let mut g = Guest::single(BASE, 0x4000);
        g.set_span(PAGES, pages).unwrap();
        g.set_u32(BASE_FIELD, PAGES).unwrap();
        g.set_u32(CURSOR_FIELD, cursor).unwrap();
        g
    }

    #[test]
    fn the_cursor_advances_by_the_field_the_logical_model_reads() {
        let mut x = 0x1234_5678u32;
        let pages: Vec<u8> = (0..2 * PAGE_BYTES)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        // Across the page, including every cursor where the field straddles the split point.
        let cursors = (0..40).map(|k| k * 409).chain(16_320..16_352);
        for cursor in cursors {
            let mut g = page_guest(&pages, cursor);
            advance_bit_cursor(&mut g, BASE_FIELD, CURSOR_FIELD).unwrap();

            let advanced = cursor + model_field(&pages, cursor);
            let (want_base, want_cursor) = if advanced < PAGE_BITS {
                (PAGES, advanced)
            } else {
                (PAGES + PAGE_BYTES, advanced - PAGE_BITS)
            };
            assert_eq!(g.u32(CURSOR_FIELD).unwrap(), want_cursor, "cursor {cursor}");
            assert_eq!(
                g.u32(BASE_FIELD).unwrap(),
                want_base,
                "base, cursor {cursor}"
            );
        }
    }

    #[test]
    fn a_wrap_steps_the_base_once_and_keeps_the_remainder() {
        // A field of 0x7FFF at a cursor of 16,000 runs 32,767 bits on: one page step, and the
        // remainder still past the page end — the original subtracts only once.
        let mut pages = vec![0u8; 2 * PAGE_BYTES as usize];
        for i in 0..15u32 {
            let p = (16_000 + 32 + i) as usize;
            pages[p / 8] |= 0x80 >> (p % 8);
        }
        let mut g = page_guest(&pages, 16_000);
        advance_bit_cursor(&mut g, BASE_FIELD, CURSOR_FIELD).unwrap();
        assert_eq!(g.u32(BASE_FIELD).unwrap(), PAGES + PAGE_BYTES);
        assert_eq!(g.u32(CURSOR_FIELD).unwrap(), 16_000 + 0x7FFF - PAGE_BITS);
        assert!(
            g.u32(CURSOR_FIELD).unwrap() >= PAGE_BITS,
            "still past the end: subtracted once"
        );
    }

    #[test]
    fn the_page_constants_are_what_the_header_implies() {
        assert_eq!(PAGE_BITS, 2048 * 8 - 32);
        assert_eq!(SPLIT_POINT, 16_352 - 15);
        assert_eq!(divide_by_eight_toward_zero(-1), 0, "toward zero, not floor");
        assert_eq!(divide_by_eight_toward_zero(-9), -1);
        assert_eq!(divide_by_eight_toward_zero(17), 2);
    }

    // ------------------------------------------------------------------ the packed stream header

    /// Pack the six fields the way the module note describes, MSB first — independent of the reader.
    fn pack_header(a: u64, b: u64, c: u64, d: u64, e: u64, f: u64) -> [u8; 8] {
        ((a << 60) | (b << 56) | (c << 50) | (d << 32) | (e << 29) | f).to_be_bytes()
    }

    #[test]
    fn the_header_unpacks_its_four_kept_fields_with_and_without_the_tag() {
        let fields = (0xA, 0x5, 0x2A, 0x2_ABCD, 5, 0x123_4567);
        let header = pack_header(fields.0, fields.1, fields.2, fields.3, fields.4, fields.5);
        for tagged in [false, true] {
            let mut g = Guest::single(BASE, 0x1000);
            let stream = BASE + 0x100;
            let mut bytes = if tagged {
                vec![b'H', 0xEE, 0xEE, 0xEE]
            } else {
                Vec::new()
            };
            bytes.extend_from_slice(&header);
            g.set_span(stream, &bytes).unwrap();
            let (out, sp) = (BASE + 0x40, BASE + 0x800);

            unpack_stream_header(&mut g, stream, out, sp).unwrap();

            assert_eq!(
                g.u8(out).unwrap(),
                0x2B,
                "tagged {tagged}: the 6-bit field plus one"
            );
            assert_eq!(
                g.u32(out + 4).unwrap(),
                0x2_ABCD,
                "tagged {tagged}: 18 bits"
            );
            assert_eq!(
                g.u32(out + 8).unwrap(),
                0x123_4567,
                "tagged {tagged}: 29 bits"
            );
            assert_eq!(
                g.u32(out + 12).unwrap(),
                0x5,
                "tagged {tagged}: the SECOND nibble"
            );
            let reader = sp - HEADER_FRAME_BYTES + HEADER_READER;
            assert_eq!(
                g.u32(reader + READER_CURSOR).unwrap(),
                64,
                "all 64 bits consumed"
            );
            assert_eq!(
                g.u32(sp - HEADER_FRAME_BYTES).unwrap(),
                sp,
                "the back chain"
            );
        }
    }
}

#[cfg(test)]
mod seek_tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const SP: u32 = BASE + 0xF000;

    fn guest() -> Guest {
        Guest::single(BASE, 0x10000)
    }

    /// Bytes into memory the guest already maps: a `put` there would be shadowed by the segment.
    fn fill(g: &mut Guest, at: u32, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            g.set_u8(at + i as u32, *b).unwrap();
        }
    }

    /// Packs fields MSB-first, as [`read_bits`] reads them.
    struct Bits(Vec<u8>, u32);
    impl Bits {
        fn push(&mut self, value: u64, width: u32) {
            for i in (0..width).rev() {
                if self.1.is_multiple_of(8) {
                    self.0.push(0);
                }
                let bit = ((value >> i) & 1) as u8;
                let last = self.0.len() - 1;
                self.0[last] |= bit << (7 - self.1 % 8);
                self.1 += 1;
            }
        }
    }

    // ------------------------------------------------------------------ sub_82B335A8

    const OBJECT: u32 = BASE;
    const VOICES: u32 = BASE + 0x1000;
    const STREAM: u32 = BASE + 0x2000;
    const SLOT: u32 = OBJECT + 0x300 + 48; // index 1
    const VOICE: u32 = VOICES + 80;

    fn header_guest() -> Guest {
        let mut g = guest();
        g.put(
            crate::routing::UNITY_GAIN,
            1.0f32.to_bits().to_be_bytes().to_vec(),
        );
        g.set_u32(OBJECT + HEADER_VOICE_ARRAY, VOICES).unwrap();
        g.set_u16(OBJECT + HEADER_SLOT_TABLE, 0x300).unwrap();
        g
    }

    fn header(
        tag: bool,
        mode: u64,
        has_cursor: bool,
        cursor: u64,
        field16: u64,
        delta: u64,
    ) -> Vec<u8> {
        let mut b = Bits(if tag { vec![72, 0, 0, 0] } else { vec![] }, 0);
        b.1 = 8 * b.0.len() as u32;
        for (v, w) in [
            (3, 4),
            (5, 4),
            (9, 6),
            (1000, 18),
            (mode, 2),
            (u64::from(has_cursor), 1),
            (12_345, 29),
        ] {
            b.push(v, w);
        }
        if has_cursor {
            b.push(cursor, 32);
        }
        if mode == 2 {
            b.push(field16, 32);
        }
        if has_cursor && (mode == 1 || (mode == 2 && cursor >= field16)) {
            b.push(delta, 32);
        }
        b.0
    }

    #[test]
    fn no_stream_writes_the_defaults() {
        let mut g = header_guest();
        parse_voice_header(&mut g, OBJECT, 1, 0, SP).unwrap();
        assert_eq!(g.u8(SLOT + SLOT_TYPE).unwrap(), 0);
        assert_eq!(g.f32(SLOT + SLOT_FIELD_16).unwrap(), 1.0);
        assert_eq!(g.u32(SLOT + SLOT_FIELD_20).unwrap(), 0x7FFF_FFFF);
        assert_eq!(g.u32(SLOT + SLOT_CURSOR).unwrap(), u32::MAX);
        assert_eq!(g.u8(VOICE + VOICE_FORMAT_INDEX).unwrap(), 255);
        assert_eq!(g.u8(VOICE + VOICE_STATE).unwrap(), 1);
    }

    #[test]
    fn a_tagged_header_with_a_cursor_reads_every_field_and_the_delta() {
        let mut g = header_guest();
        let bytes = header(true, 1, true, 77, 0, 5);
        let len = bytes.len() as u32;
        fill(&mut g, STREAM, &bytes);
        parse_voice_header(&mut g, OBJECT, 1, u64::from(STREAM), SP).unwrap();
        assert_eq!(g.u8(VOICE + VOICE_STATE).unwrap(), 3);
        assert_eq!(g.u8(VOICE + VOICE_FORMAT_INDEX).unwrap(), 5);
        assert_eq!(g.u8(SLOT + SLOT_TYPE).unwrap(), 10, "six bits plus one");
        assert_eq!(g.f32(SLOT + SLOT_FIELD_16).unwrap(), 1000.0);
        assert_eq!(g.u8(VOICE + VOICE_MODE).unwrap(), 1);
        assert_eq!(g.u32(SLOT + SLOT_FIELD_20).unwrap(), 12_345);
        assert_eq!(g.u32(SLOT + SLOT_CURSOR).unwrap(), 77);
        assert_eq!(g.u32(VOICE + VOICE_DELTA).unwrap(), 5);
        assert_eq!(
            g.u32(VOICE + VOICE_DATA_BASE).unwrap(),
            STREAM + len,
            "past the tag and the 128 bits"
        );
    }

    #[test]
    fn mode_two_reads_its_field_and_a_delta_only_when_the_cursor_has_reached_it() {
        let mut g = header_guest();
        fill(&mut g, STREAM, &header(false, 2, true, 10, 20, 0));
        parse_voice_header(&mut g, OBJECT, 1, u64::from(STREAM), SP).unwrap();
        assert_eq!(g.u32(VOICE + VOICE_FIELD_16).unwrap(), 20);
        assert_eq!(
            g.u32(VOICE + VOICE_DELTA).unwrap(),
            0,
            "cursor 10 is below 20: no delta read"
        );
        let mut h = header_guest();
        fill(&mut h, STREAM, &header(false, 2, true, 30, 20, 9));
        parse_voice_header(&mut h, OBJECT, 1, u64::from(STREAM), SP).unwrap();
        assert_eq!(h.u32(VOICE + VOICE_DELTA).unwrap(), 9);
    }

    // ------------------------------------------------------------------ sub_82B474B8

    /// The one-byte signed code for `v`, found by asking the decoder rather than restating it.
    fn signed_byte(v: i32) -> u8 {
        let mut probe = Guest::single(0x5000_0000, 16);
        (0..192u8)
            .find(|b| {
                probe.set_u8(0x5000_0000, *b).unwrap();
                signed_code(&probe, 0x5000_0000).unwrap() == (v as u32, 1)
            })
            .expect("a one-byte code")
    }

    /// Each step of each stream is a repeat run of one: a count of 0, then the delta.
    fn seek_stream(sets: &[[i32; 4]]) -> Vec<u8> {
        sets.iter()
            .flat_map(|set| set.iter().flat_map(|d| [signed_byte(0), signed_byte(*d)]))
            .collect()
    }

    const RECORD: u32 = BASE + 0x100;

    #[test]
    fn the_record_containing_the_target_less_its_minimum_is_published() {
        // Records of length 10 at 0, 10, 20, 30; target 25 less a minimum of 5 is 20, inside the third.
        let mut g = guest();
        fill(
            &mut g,
            STREAM,
            &seek_stream(&[[3, 2, 10, 0], [3, 2, 0, 0], [3, 2, 0, 0], [3, 2, 0, 0]]),
        );
        g.set_u32(RECORD + SEEK_MIN_LENGTH, 5).unwrap();
        g.set_u32(RECORD + SEEK_PUBLISH_BASE, 100).unwrap();
        assert_eq!(
            seek_record(&mut g, RECORD, u64::from(STREAM), 25, SP).unwrap(),
            0,
            "walked past the target"
        );
        assert_eq!(g.u32(RECORD + SEEK_PUBLISH_POSITION).unwrap(), 20);
        assert_eq!(
            g.u32(RECORD + SEEK_PUBLISH_LENGTH).unwrap(),
            5,
            "min(25 - 20, 5)"
        );
        assert_eq!(
            g.u32(RECORD + SEEK_PUBLISH_SKIP).unwrap(),
            0,
            "(25 - 5) - 20"
        );
        assert_eq!(
            g.u32(RECORD + SEEK_PUBLISH_OFFSET).unwrap(),
            9,
            "stream 0 summed over two records: 3 + 6"
        );
        assert_eq!(
            g.u32(RECORD + SEEK_PUBLISH_BASE).unwrap(),
            106,
            "the entry word plus 2 + 4"
        );
        assert_eq!(g.u8(RECORD + SEEK_PUBLISH_EXACT).unwrap(), 0);
    }

    #[test]
    fn a_negative_first_length_returns_one_and_publishes_nothing() {
        let mut g = guest();
        fill(&mut g, STREAM, &seek_stream(&[[3, 2, -1, 0]]));
        g.set_u32(RECORD + SEEK_PUBLISH_POSITION, 0x7777).unwrap();
        assert_eq!(
            seek_record(&mut g, RECORD, u64::from(STREAM), 25, SP).unwrap(),
            1
        );
        assert_eq!(g.u32(RECORD + SEEK_PUBLISH_POSITION).unwrap(), 0x7777);
    }

    #[test]
    fn mode_one_publishes_every_record_and_marks_it_exact() {
        let mut g = guest();
        fill(
            &mut g,
            STREAM,
            &seek_stream(&[[3, 2, 10, 1], [3, 2, 0, 0], [3, 2, 0, 0]]),
        );
        g.set_u32(RECORD + SEEK_MIN_LENGTH, 5).unwrap();
        seek_record(&mut g, RECORD, u64::from(STREAM), 25, SP).unwrap();
        // Record 0 is published because its mode is 1; the later ones decode a mode of 1 too (a
        // repeat run's sum stays 1), so the last published is the one at 20.
        assert_eq!(g.u8(RECORD + SEEK_PUBLISH_EXACT).unwrap(), 1);
        assert_eq!(g.u32(RECORD + SEEK_PUBLISH_POSITION).unwrap(), 20);
    }

    // ------------------------------------------------------------------ sub_82B50100

    const CURSOR: u32 = BASE + 0x200;
    const DESC: u32 = BASE + 0x300;
    const TABLE: u32 = BASE + 0x400;
    const CHUNK: u32 = BASE + 0x4000;

    fn packet_guest(samples: u32, header: u32) -> Guest {
        let mut g = guest();
        g.set_u32(DESC + PACKET_DESC_SAMPLES, samples).unwrap();
        g.set_u8(DESC + PACKET_DESC_FLAG, 1).unwrap();
        g.set_u32(CHUNK, (0x3000 << 2) | 1).unwrap(); // 0x3000 bytes, one flag bit
        g.set_u32(CHUNK + 4, header).unwrap();
        g
    }

    #[test]
    fn a_target_in_the_first_packet_publishes_its_header_position() {
        // 100 samples is frame 0; the header word 0x00012800 carries bit position 37.
        let mut g = packet_guest(100, 0x0001_2800);
        assert_eq!(
            seek_packet(&mut g, CURSOR, 0, DESC, u64::from(CHUNK), SP).unwrap(),
            0x3000
        );
        assert_eq!(g.u32(CURSOR + PACKET_CURSOR_PACKET).unwrap(), CHUNK + 4);
        assert_eq!(g.u32(CURSOR + PACKET_CURSOR_BITS).unwrap(), 37);
        assert_eq!(g.u32(CURSOR + PACKET_CURSOR_REMAINING).unwrap(), 0x3000 - 4);
    }

    #[test]
    fn a_position_past_the_page_is_poisoned() {
        let mut g = packet_guest(100, 16_352 << 11);
        seek_packet(&mut g, CURSOR, 0, DESC, u64::from(CHUNK), SP).unwrap();
        assert_eq!(g.u32(CURSOR + PACKET_CURSOR_BITS).unwrap(), u32::MAX);
    }

    #[test]
    fn the_seek_table_walks_whole_packets_until_the_target_frame() {
        // Frame 5 (2,560 samples); substream 1's packets hold 2, then 3, then 4 frames, so the walk
        // passes two packets and lands on the third with no frames left to skip.
        let mut g = packet_guest(5 * 512, 0);
        g.set_u32(DESC + PACKET_DESC_TABLE, TABLE).unwrap();
        g.set_u32(TABLE + 4, 0x40).unwrap();
        fill(&mut g, TABLE + 0x40, &[2, 3, 4]);
        g.set_u32(CHUNK + 4 + 4096, 0x0000_5000).unwrap(); // the third packet's header: position 10
        seek_packet(&mut g, CURSOR, 1, DESC, u64::from(CHUNK), SP).unwrap();
        assert_eq!(
            g.u32(CURSOR + PACKET_CURSOR_PACKET).unwrap(),
            CHUNK + 4 + 4096
        );
        assert_eq!(g.u32(CURSOR + PACKET_CURSOR_BITS).unwrap(), 10);
        assert_eq!(
            g.u32(CURSOR + PACKET_CURSOR_REMAINING).unwrap(),
            0x3000 - 4096 - 4
        );
    }
}

#[cfg(test)]
mod header_seek_tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const OBJECT: u32 = BASE + 0x100;
    const RECORDS: u32 = BASE + 0x1000;
    const HEADER: u32 = BASE + 0x2000;
    const STREAM: u32 = BASE + 0x3000;
    const SP: u32 = BASE + 0xF000;

    fn guest() -> Guest {
        Guest::single(BASE, 0x10000)
    }

    fn fill(g: &mut Guest, at: u32, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            g.set_u8(at + i as u32, *b).unwrap();
        }
    }

    /// Records of `{offset, base, length, mode}`.
    fn records(g: &mut Guest, at: u32, list: &[[i32; 4]]) {
        let bytes: Vec<u8> = list
            .iter()
            .flat_map(|r| r.iter().flat_map(|w| w.to_be_bytes()))
            .collect();
        fill(g, at, &bytes);
    }

    const FOUR: [[i32; 4]; 4] = [[3, 2, 10, 0], [3, 2, 10, 0], [3, 2, 10, 0], [3, 2, 10, 0]];

    #[test]
    fn the_record_containing_the_target_less_its_minimum_is_published() {
        let mut g = guest();
        records(&mut g, RECORDS, &FOUR);
        g.set_u32(OBJECT + SEEK_MIN_LENGTH, 5).unwrap();
        g.set_u32(OBJECT + SEEK_PUBLISH_BASE, 100).unwrap();
        assert_eq!(seek_fixed_records(&mut g, OBJECT, RECORDS, 25).unwrap(), 0);
        assert_eq!(g.u32(OBJECT + SEEK_PUBLISH_POSITION).unwrap(), 20);
        assert_eq!(g.u32(OBJECT + SEEK_PUBLISH_LENGTH).unwrap(), 5);
        assert_eq!(g.u32(OBJECT + SEEK_PUBLISH_SKIP).unwrap(), 0);
        assert_eq!(
            g.u32(OBJECT + SEEK_PUBLISH_OFFSET).unwrap(),
            6,
            "two records' offsets"
        );
        assert_eq!(g.u32(OBJECT + SEEK_PUBLISH_BASE).unwrap(), 104);
    }

    #[test]
    fn a_negative_first_length_returns_one_untouched() {
        let mut g = guest();
        records(&mut g, RECORDS, &[[3, 2, -1, 0]]);
        g.set_u32(OBJECT + SEEK_PUBLISH_POSITION, 0x7777).unwrap();
        assert_eq!(seek_fixed_records(&mut g, OBJECT, RECORDS, 25).unwrap(), 1);
        assert_eq!(g.u32(OBJECT + SEEK_PUBLISH_POSITION).unwrap(), 0x7777);
    }

    #[test]
    fn the_seek_header_dispatches_by_its_high_nibble() {
        let mut g = guest();
        // Mode 0, kind 3, minimum 5, records 0x100 on, data 0x40 on.
        fill(&mut g, HEADER, &[0, 0x03, 0, 5, 0, 0, 1, 0, 0, 0, 0, 0x40]);
        records(&mut g, HEADER + 0x100, &FOUR);
        assert_eq!(
            parse_seek_header(&mut g, OBJECT, u64::from(HEADER), 25, SP).unwrap(),
            0
        );
        assert_eq!(g.u32(OBJECT + SEEK_HEADER_KIND).unwrap(), 3);
        assert_eq!(g.u32(OBJECT + SEEK_MIN_LENGTH).unwrap(), 5);
        assert_eq!(g.u32(OBJECT + SEEK_HEADER_END).unwrap(), HEADER + 12);
        assert_eq!(g.u32(OBJECT + SEEK_PUBLISH_POSITION).unwrap(), 20);
        assert_eq!(
            g.u32(OBJECT + SEEK_PUBLISH_BASE).unwrap(),
            HEADER + 0x40 + 4,
            "the data pointer is the base"
        );
        let mut h = guest();
        fill(&mut h, HEADER, &[0, 0x23, 0, 5, 0, 0, 1, 0, 0, 0, 0, 0]);
        h.set_u32(OBJECT + SEEK_PUBLISH_POSITION, 0x7777).unwrap();
        assert_eq!(
            parse_seek_header(&mut h, OBJECT, u64::from(HEADER), 25, SP).unwrap(),
            0
        );
        assert_eq!(
            h.u32(OBJECT + SEEK_PUBLISH_BASE).unwrap(),
            0,
            "a zero data offset stays zero"
        );
        assert_eq!(
            h.u32(OBJECT + SEEK_PUBLISH_POSITION).unwrap(),
            0x7777,
            "mode 2 walks nothing"
        );
    }

    #[test]
    fn a_version_zero_header_walks_its_payload_inline() {
        let mut g = guest();
        fill(&mut g, STREAM, &[0, 0x07, 0, 5, 0, 0, 0, 0]);
        records(&mut g, STREAM + 8, &FOUR);
        assert_eq!(
            parse_stream_header(&mut g, OBJECT, STREAM, 25, SP).unwrap(),
            0
        );
        assert_eq!(g.u32(OBJECT + SEEK_HEADER_KIND).unwrap(), 7);
        assert_eq!(g.u32(OBJECT + SEEK_PUBLISH_POSITION).unwrap(), 20);
        assert_eq!(
            g.u32(OBJECT + SEEK_PUBLISH_BASE).unwrap(),
            4,
            "a zero base entry plus two bases"
        );
    }

    #[test]
    fn an_unknown_version_fails_and_wipes_six_words() {
        for version in [2u8, 0x80] {
            let mut g = guest();
            fill(&mut g, STREAM, &[version]);
            for offset in [0u32, 4, 8, 12, 16, 20, 24, 28] {
                g.set_u32(OBJECT + offset, 0x1000 + offset).unwrap();
            }
            assert_eq!(
                parse_stream_header(&mut g, OBJECT, STREAM, 25, SP).unwrap(),
                1,
                "version {version:#x}"
            );
            for offset in [0u32, 4, 8, 12, 20, 24] {
                assert_eq!(g.u32(OBJECT + offset).unwrap(), 0, "+{offset}");
            }
            assert_eq!(g.u32(OBJECT + 16).unwrap(), 0x1010, "+16 is left");
            assert_eq!(g.u32(OBJECT + 28).unwrap(), 0x101C, "+28 is left");
        }
    }

    const DECODER: u32 = BASE + 0x4000;
    const ARRAY: u32 = BASE + 0x5000;
    const SLOT: u32 = DECODER + 0x200 + 48;
    const ENTRY: u32 = ARRAY + 80;

    fn decoder() -> Guest {
        let mut g = guest();
        g.set_u32(DECODER + HEADER_VOICE_ARRAY, ARRAY).unwrap();
        g.set_u16(DECODER + HEADER_SLOT_TABLE, 0x200).unwrap();
        fill(&mut g, STREAM, &[0, 0x07, 0, 5, 0, 0, 0, 0]);
        records(&mut g, STREAM + 8, &FOUR);
        g
    }

    #[test]
    fn a_decoded_header_scatters_seven_fields() {
        let mut g = decoder();
        decode_packet_header(&mut g, u64::from(DECODER), 1, u64::from(STREAM), 25, SP).unwrap();
        assert_eq!(g.u32(SLOT + 36).unwrap(), 20, "the position");
        assert_eq!(g.u32(SLOT + 32).unwrap(), 0, "the skip");
        assert_eq!(g.u32(ENTRY + 60).unwrap(), 5, "the length");
        assert_eq!(g.u32(ENTRY + 64).unwrap(), 6, "the offset");
        assert_eq!(g.u32(ENTRY + 56).unwrap(), 4, "the base");
        assert_eq!(g.u32(ENTRY + 68).unwrap(), 7, "the kind");
        assert_eq!(g.u8(ENTRY + 77).unwrap(), 0);
        assert_eq!(g.u32(ENTRY + 20).unwrap(), 20, "the slot's +36, re-read");
    }

    #[test]
    fn a_zero_length_clears_both_records_and_marks_the_entry() {
        let mut g = decoder();
        g.set_u32(SLOT + 36, 0x55).unwrap();
        g.set_u32(ENTRY + 60, 0x55).unwrap();
        decode_packet_header(&mut g, u64::from(DECODER), 1, u64::from(STREAM), 0, SP).unwrap();
        assert_eq!(g.u32(SLOT + 36).unwrap(), 0);
        assert_eq!(g.u32(ENTRY + 60).unwrap(), 0);
        assert_eq!(g.u8(ENTRY + 77).unwrap(), 1);
    }
}
