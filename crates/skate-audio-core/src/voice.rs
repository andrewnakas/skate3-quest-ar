//! The evaluator's voice op and the voice-object helpers, run against a host voice device.
//!
//! **Unverified.** Every function here has a C++ body in `recomp/src/audio_ports/`, but all are
//! gate-labelled or partial (each makes indirect calls into the voice device), so none was
//! compared. They are transcribed from those bodies:
//!
//! | function | here |
//! |---|---|
//! | `sub_82B1D240`, evaluator slot 27 | [`voice_op`] |
//! | `sub_82B1F440` | [`deactivate`] |
//! | `sub_82B1F4C8` | [`open_voice`] |
//! | `sub_82B1F5F8` | [`refresh_voice`] |
//! | `sub_82B1BE30` | [`set_property`] |
//!
//! In the game, the device is the singleton at [`DEVICE_SLOT`], which game init sets to the static
//! object at `0x8302F068`. Its open, `sub_824A3140`, builds a mixer graph for the voice. Here the
//! device is [`VoiceDevice`]: every vtable call the originals make goes through it, so a host can
//! log them, or open a voice on the ported graph.
//!
//! The voice object the op runs over (the op's operand block):
//!
//! ```text
//! +0   config: the bank, whose +64 is its sample bank and +72/+76 two open arguments
//! +4   descriptor table {u32 count; 12-byte descriptors from +4}
//! +8   the open voice, or 0
//! +12  s8 latched state, +13 the state before it
//! +14  u8 parameter record count, +15 and +17 u8 copy-back flags
//! +20  descriptor index        +24  requested state: 0 off, 1 on, 2 paused
//! +28  12-byte parameter records {u8 id; s32 applied; s32 wanted}
//! ```

use crate::{Error, Guest, Result};

/// An observer of every voice a device opens: the request, its `{id, s32, value}` records and the
/// voice. The playtest trace installs one so its opens compare with the retail capture's
/// `skate3-audio-open` lines.
pub type OpenObserver = fn(&OpenRequest, &[(u8, i32, i32)], u32);

static OPEN_OBSERVER: std::sync::OnceLock<OpenObserver> = std::sync::OnceLock::new();

/// Install the open observer; only the first call takes effect.
pub fn observe_opens(observer: OpenObserver) {
    let _ = OPEN_OBSERVER.set(observer);
}

/// Report an open to the observer, reading the request's 12-byte records from guest memory.
pub(crate) fn report_open(g: &Guest, request: &OpenRequest, voice: u32) -> Result<()> {
    let Some(observer) = OPEN_OBSERVER.get() else {
        return Ok(());
    };
    let mut records = Vec::with_capacity(request.record_count as usize);
    for i in 0..request.record_count {
        let at = request.records.wrapping_add(12 * i);
        records.push((g.u8(at)?, g.u32(at + 4)? as i32, g.u32(at + 8)? as i32));
    }
    observer(request, &records, voice);
    Ok(())
}

/// An observer of every voice a device releases, alongside [`observe_opens`].
pub type ReleaseObserver = fn(u32);

static RELEASE_OBSERVER: std::sync::OnceLock<ReleaseObserver> = std::sync::OnceLock::new();

/// Install the release observer; only the first call takes effect.
pub fn observe_releases(observer: ReleaseObserver) {
    let _ = RELEASE_OBSERVER.set(observer);
}

pub(crate) fn report_release(voice: u32) {
    if let Some(observer) = RELEASE_OBSERVER.get() {
        observer(voice);
    }
}

/// `lis -32003` + 13816: the device singleton's cell.
pub const DEVICE_SLOT: u32 = 0x82FD_35F8;

/// What the device's open (`vtable+0`) receives, as `sub_82B1F4C8` builds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenRequest {
    /// `r4`: the bank sample's EA Audio Core stream, `S10A offset + S10A base` (a 64-bit add).
    pub sample: u64,
    /// The descriptor's signed sample index, from which `sample` was chosen.
    pub index: i16,
    /// `r5`: descriptor byte 2.
    pub byte2: u8,
    /// The `r6` block: descriptor bytes 3 to 8, each shifted up one byte.
    pub shifted: [u32; 6],
    /// `r7`: the bank's `+72` word.
    pub bank_72: u32,
    /// `r8`: the bank's `+76` word plus the descriptor's `+8` word (a 64-bit add).
    pub arg8: u64,
    /// The `r9` block: how many parameter records, and where the first one is.
    pub record_count: u32,
    pub records: u32,
}

/// The device and the voice it opens, as the originals call them through vtables.
pub trait VoiceDevice {
    /// Device `vtable+0`: open a voice; 0 on failure.
    fn open(&mut self, g: &mut Guest, request: &OpenRequest) -> Result<u32>;
    /// Voice `vtable+0`.
    fn release(&mut self, g: &mut Guest, voice: u32) -> Result<()>;
    /// Voice `vtable+4`.
    fn suspend(&mut self, g: &mut Guest, voice: u32) -> Result<()>;
    /// Voice `vtable+8`.
    fn resume(&mut self, g: &mut Guest, voice: u32) -> Result<()>;
    /// Voice `vtable+12`: `(id, value)`.
    fn set(&mut self, g: &mut Guest, voice: u32, id: u32, value: u32) -> Result<()>;
    /// Voice `vtable+16`: property 3 only, as `(value, 0)`.
    fn set_alternate(&mut self, g: &mut Guest, voice: u32, value: u32) -> Result<()>;
    /// Voice `vtable+20`: fill eleven words; the first is zero when the voice has ended.
    fn query(&mut self, g: &mut Guest, voice: u32, out: &mut [u32; 11]) -> Result<()>;
    /// Voice `vtable+24`, called at the end of every op on a live voice.
    fn poke(&mut self, g: &mut Guest, voice: u32) -> Result<()>;
}

/// A device that opens nothing and refuses every voice call: for callers with no voices.
pub struct NoDevice;

impl VoiceDevice for NoDevice {
    fn open(&mut self, _g: &mut Guest, _request: &OpenRequest) -> Result<u32> {
        Ok(0)
    }
    fn release(&mut self, _g: &mut Guest, voice: u32) -> Result<()> {
        Err(Error::new(voice, "no voice device"))
    }
    fn suspend(&mut self, _g: &mut Guest, voice: u32) -> Result<()> {
        Err(Error::new(voice, "no voice device"))
    }
    fn resume(&mut self, _g: &mut Guest, voice: u32) -> Result<()> {
        Err(Error::new(voice, "no voice device"))
    }
    fn set(&mut self, _g: &mut Guest, voice: u32, _id: u32, _value: u32) -> Result<()> {
        Err(Error::new(voice, "no voice device"))
    }
    fn set_alternate(&mut self, _g: &mut Guest, voice: u32, _value: u32) -> Result<()> {
        Err(Error::new(voice, "no voice device"))
    }
    fn query(&mut self, _g: &mut Guest, voice: u32, _out: &mut [u32; 11]) -> Result<()> {
        Err(Error::new(voice, "no voice device"))
    }
    fn poke(&mut self, _g: &mut Guest, voice: u32) -> Result<()> {
        Err(Error::new(voice, "no voice device"))
    }
}

/// `sub_82B1BE30`: clamp a property to its id's range and hand it to the voice. Ids 0, 6 and 7 clamp
/// to 0..65535, ids 2, 5 and 8 to 0..32767 (both signed), 1 and 4 pass through, 3 goes to the
/// alternate setter, 9 to 136 pass through, and anything else is dropped. A null voice is a no-op.
pub fn set_property(
    g: &mut Guest,
    device: &mut dyn VoiceDevice,
    voice: u32,
    id: u32,
    value: u32,
) -> Result<()> {
    if voice == 0 {
        return Ok(());
    }
    let clamp = |v: u32, limit: i32| -> u32 {
        if (v as i32) < 0 {
            0
        } else if (v as i32) > limit {
            limit as u32
        } else {
            v
        }
    };
    if id > 8 {
        // cmplwi ... bgt, then signed [9, 136]: a negative id drops out here.
        if (id as i32) < 9 || (id as i32) > 136 {
            return Ok(());
        }
        return device.set(g, voice, id, value);
    }
    match id {
        1 | 4 => device.set(g, voice, id, value),
        2 | 5 | 8 => device.set(g, voice, id, clamp(value, 32767)),
        3 => device.set_alternate(g, voice, value),
        6 | 7 => device.set(g, voice, id, clamp(value, 65535)),
        _ => device.set(g, voice, 0, clamp(value, 65535)),
    }
}

/// `sub_82B1F440`: release the open voice, then, when `+15` is set, clear the two words of the
/// parameter record `+14` selects (`+32` before `+28`). Returns 0.
pub fn deactivate(g: &mut Guest, device: &mut dyn VoiceDevice, object: u32) -> Result<u64> {
    let owned = g.u32(object + 8)?;
    if owned != 0 {
        device.release(g, owned)?;
        g.set_u32(object + 8, 0)?;
    }
    if g.u8(object + 15)? != 0 {
        let slot = (g.u8(object + 14)? as u32 * 12).wrapping_add(object);
        g.set_u32(slot + 32, 0)?;
        g.set_u32(slot + 28, 0)?;
    }
    Ok(0)
}

/// `sub_82B1F4C8`: open a voice for `descriptor` and push every parameter record to it. Returns the
/// voice, or 0 after deactivating the object when the open fails.
pub fn open_voice(
    g: &mut Guest,
    device: &mut dyn VoiceDevice,
    object: u32,
    descriptor: u32,
) -> Result<u64> {
    // Loads in the original's order.
    let index_half = g.u16(descriptor)?;
    let config = g.u32(object)?;
    let index = index_half as i16 as i64 + 3; // extsh ; addi r7,r10,3
    let scaled = ((index as u32) << 2) & !3;
    let b4 = g.u8(descriptor + 4)?;
    let b3 = g.u8(descriptor + 3)?;
    let b5 = g.u8(descriptor + 5)?;
    let b8 = g.u8(descriptor + 8)?;
    let table = g.u32(config + 64)?;
    let b6 = g.u8(descriptor + 6)?;
    let b7 = g.u8(descriptor + 7)?;
    let count = g.u8(object + 14)? as u32;
    let entry = g.u32(scaled.wrapping_add(table))?;
    let arg7 = g.u32(config + 72)?;
    let word8 = g.u32(descriptor + 8)?;
    let arg8 = g.u32(config + 76)? as u64 + word8 as u64;
    let byte2 = g.u8(descriptor + 2)?;
    let request = OpenRequest {
        sample: entry as u64 + table as u64,
        index: index_half as i16,
        byte2,
        shifted: [b3, b4, b5, b6, b7, b8].map(|b| (b as u32) << 8),
        bank_72: arg7,
        arg8,
        record_count: count,
        records: object + 28,
    };
    let handle = device.open(g, &request)?;
    if handle == 0 {
        deactivate(g, device, object)?;
        return Ok(0);
    }
    if g.u8(object + 14)? != 0 {
        // Unconditional push; the wanted word is reloaded after each call.
        let mut cursor = object + 28;
        let mut seen = 0i32;
        loop {
            let id = g.u8(cursor)? as u32;
            let wanted = g.u32(cursor + 8)?;
            set_property(g, device, handle, id, wanted)?;
            let applied = g.u32(cursor + 8)?;
            seen += 1;
            g.set_u32(cursor + 4, applied)?;
            cursor += 12;
            if !(seen < g.u8(object + 14)? as i32) {
                break;
            }
        }
    }
    Ok(handle as u64)
}

/// `sub_82B1F5F8`: push the dirty parameter records, then query the voice. A finished voice (first
/// word zero) deactivates the object and returns its 0; otherwise the reported words are copied
/// back behind the records as `+15` and `+17` ask, and 1 is returned.
pub fn refresh_voice(g: &mut Guest, device: &mut dyn VoiceDevice, object: u32) -> Result<u64> {
    let mut cursor = object + 28;
    if g.u8(object + 14)? != 0 {
        let mut index = 0i32;
        loop {
            let wanted = g.u32(cursor + 8)?;
            let applied = g.u32(cursor + 4)?;
            if wanted as i32 != applied as i32 {
                let id = g.u8(cursor)? as u32;
                let voice = g.u32(object + 8)?;
                set_property(g, device, voice, id, wanted)?;
                let reloaded = g.u32(cursor + 8)?;
                g.set_u32(cursor + 4, reloaded)?;
            }
            let count = g.u8(object + 14)? as i32;
            index += 1;
            cursor += 12;
            if !(index < count) {
                break;
            }
        }
    }
    let voice = g.u32(object + 8)?;
    let mut out = [0u32; 11];
    device.query(g, voice, &mut out)?;
    if out[0] as i32 == 0 {
        return deactivate(g, device, object);
    }
    let mut block = cursor;
    if g.u8(object + 15)? != 0 {
        block = cursor + 8;
        g.set_u32(cursor + 4, out[2])?; // high word first, as lifted
        g.set_u32(cursor, out[1])?;
    }
    if g.u8(object + 17)? != 0 {
        for i in 0..8u32 {
            g.set_u32(block + 4 * i, out[3 + i as usize])?;
        }
    }
    Ok(1)
}

/// Evaluator slot 27, `sub_82B1D240`: move the voice object toward its requested state. Returns 0
/// when no voice remains, otherwise the state (or `refresh_voice`'s result on the "on" path).
pub fn voice_op(g: &mut Guest, device: &mut dyn VoiceDevice, object: u32) -> Result<u64> {
    let request = g.u32(object + 24)?;
    let mut state: u64 = if (request as i32) < 0 {
        0
    } else if (request as i32) > 2 {
        2
    } else {
        request as u64
    };
    let latched = g.u8(object + 12)? as i8 as i32;
    if state as u32 as i32 != latched {
        let instance = g.u32(object + 8)?;
        match state {
            0 => {
                if instance != 0 {
                    device.release(g, instance)?;
                    g.set_u32(object + 8, 0)?;
                    deactivate(g, device, object)?;
                }
            }
            1 => {
                if instance != 0 {
                    device.resume(g, instance)?;
                } else if !(latched == 2 && g.u8(object + 13)? == 1) {
                    let table = g.u32(object + 4)?;
                    let index = g.u32(object + 20)?;
                    let count = g.u32(table)?;
                    let chosen: u64 = if (index as i32) < (count as i32) {
                        let sign = (index as u64 >> 31) & 1;
                        sign.wrapping_sub(1) & index as u64 // a negative index becomes 0
                    } else {
                        (count as i32 as i64 - 1) as u64
                    };
                    let twice = ((chosen as u32 as u64) << 1) & 0xFFFF_FFFE;
                    let thrice = chosen.wrapping_add(twice);
                    let scaled = ((thrice as u32 as u64) << 2) & 0xFFFF_FFFC;
                    let entry = scaled.wrapping_add(table as u64) as u32;
                    let descriptor = entry.wrapping_add(4);
                    if g.u16(entry + 4)? == 0xFFFF {
                        g.set_u32(object + 8, 0)?;
                        deactivate(g, device, object)?;
                    } else {
                        let handle = open_voice(g, device, object, descriptor)?;
                        g.set_u32(object + 8, handle as u32)?;
                    }
                }
            }
            _ => {
                if instance != 0 {
                    device.suspend(g, instance)?;
                }
            }
        }
        let displaced = g.u8(object + 12)?; // reloaded
        g.set_u8(object + 12, state as u8)?;
        g.set_u8(object + 13, displaced)?;
    }
    if state == 1 && g.u32(object + 8)? != 0 {
        state = refresh_voice(g, device, object)?;
    }
    let live = g.u32(object + 8)?;
    if live == 0 {
        return Ok(0);
    }
    device.poke(g, live)?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEM: u32 = 0x5000_0000;
    const OBJ: u32 = MEM;
    const CONFIG: u32 = MEM + 0x100;
    const SAMPLES: u32 = MEM + 0x200;
    const TABLE: u32 = MEM + 0x300;
    const VOICE: u32 = 0x7000_0100;

    #[derive(Default)]
    struct Mock {
        calls: Vec<String>,
        alive: bool,
    }

    impl VoiceDevice for Mock {
        fn open(&mut self, _g: &mut Guest, r: &OpenRequest) -> Result<u32> {
            self.calls.push(format!(
                "open {:x} {} {} {:x?} {:x} {:x} {} {:x}",
                r.sample, r.index, r.byte2, r.shifted, r.bank_72, r.arg8, r.record_count, r.records
            ));
            Ok(VOICE)
        }
        fn release(&mut self, _g: &mut Guest, v: u32) -> Result<()> {
            self.calls.push(format!("release {v:x}"));
            Ok(())
        }
        fn suspend(&mut self, _g: &mut Guest, v: u32) -> Result<()> {
            self.calls.push(format!("suspend {v:x}"));
            Ok(())
        }
        fn resume(&mut self, _g: &mut Guest, v: u32) -> Result<()> {
            self.calls.push(format!("resume {v:x}"));
            Ok(())
        }
        fn set(&mut self, _g: &mut Guest, v: u32, id: u32, value: u32) -> Result<()> {
            self.calls.push(format!("set {v:x} {id} {value}"));
            Ok(())
        }
        fn set_alternate(&mut self, _g: &mut Guest, v: u32, value: u32) -> Result<()> {
            self.calls.push(format!("set3 {v:x} {value}"));
            Ok(())
        }
        fn query(&mut self, _g: &mut Guest, v: u32, out: &mut [u32; 11]) -> Result<()> {
            self.calls.push(format!("query {v:x}"));
            out[0] = self.alive as u32;
            Ok(())
        }
        fn poke(&mut self, _g: &mut Guest, v: u32) -> Result<()> {
            self.calls.push(format!("poke {v:x}"));
            Ok(())
        }
    }

    fn guest() -> Guest {
        let mut g = Guest::single(MEM, 0x400);
        let w = |g: &mut Guest, at: u32, v: &[u32]| {
            for (i, x) in v.iter().enumerate() {
                g.set_u32(at + 4 * i as u32, *x).unwrap();
            }
        };
        w(&mut g, OBJ, &[CONFIG, TABLE]);
        g.set_u8(OBJ + 14, 2).unwrap();
        w(&mut g, OBJ + 20, &[1, 1]); // descriptor 1, request "on"
        w(
            &mut g,
            OBJ + 28,
            &[0x0200_0000, 0, 40000, 0x0300_0000, 0, 7],
        );
        g.set_u32(CONFIG + 64, SAMPLES).unwrap();
        w(&mut g, CONFIG + 72, &[0x11, 0x20]);
        w(&mut g, SAMPLES, &[0x5331_3041, 0, 2, 0x20, 0x40]);
        g.set_u32(TABLE, 2).unwrap();
        // Descriptor 1 at TABLE+16: sample 1, byte 5, bytes 1..6 at +3..+8.
        g.set_span(TABLE + 16, &[0, 1, 5, 1, 2, 3, 4, 5, 6, 0, 0, 0])
            .unwrap();
        g
    }

    #[test]
    fn switching_on_opens_the_described_sample_and_pushes_the_records() {
        let mut g = guest();
        let mut dev = Mock {
            alive: true,
            ..Default::default()
        };
        assert_eq!(voice_op(&mut g, &mut dev, OBJ).unwrap(), 1);
        assert_eq!(
            dev.calls,
            vec![
                format!(
                    "open {:x} 1 5 [100, 200, 300, 400, 500, 600] 11 6000020 2 {:x}",
                    SAMPLES + 0x40,
                    OBJ + 28
                ),
                format!("set {VOICE:x} 2 32767"),
                format!("set3 {VOICE:x} 7"),
                format!("query {VOICE:x}"),
                format!("poke {VOICE:x}"),
            ]
        );
        assert_eq!(g.u32(OBJ + 8).unwrap(), VOICE);
        assert_eq!((g.u8(OBJ + 12).unwrap(), g.u8(OBJ + 13).unwrap()), (1, 0));
        assert_eq!(
            g.u32(OBJ + 28 + 4).unwrap(),
            40000,
            "applied copies wanted, unclamped"
        );
    }

    #[test]
    fn a_dirty_record_is_pushed_on_the_next_op_and_a_finished_voice_is_dropped() {
        let mut g = guest();
        let mut dev = Mock {
            alive: true,
            ..Default::default()
        };
        voice_op(&mut g, &mut dev, OBJ).unwrap();
        dev.calls.clear();
        g.set_u32(OBJ + 28 + 8, 100).unwrap();
        voice_op(&mut g, &mut dev, OBJ).unwrap();
        assert_eq!(
            dev.calls,
            vec![
                format!("set {VOICE:x} 2 100"),
                format!("query {VOICE:x}"),
                format!("poke {VOICE:x}")
            ]
        );
        dev.calls.clear();
        dev.alive = false;
        assert_eq!(voice_op(&mut g, &mut dev, OBJ).unwrap(), 0);
        assert_eq!(
            dev.calls,
            vec![format!("query {VOICE:x}"), format!("release {VOICE:x}")]
        );
        assert_eq!(g.u32(OBJ + 8).unwrap(), 0);
    }

    #[test]
    fn off_releases_and_paused_suspends() {
        let mut g = guest();
        let mut dev = Mock {
            alive: true,
            ..Default::default()
        };
        voice_op(&mut g, &mut dev, OBJ).unwrap();
        g.set_u32(OBJ + 24, 2).unwrap();
        dev.calls.clear();
        assert_eq!(voice_op(&mut g, &mut dev, OBJ).unwrap(), 2);
        assert_eq!(
            dev.calls,
            vec![format!("suspend {VOICE:x}"), format!("poke {VOICE:x}")]
        );
        g.set_u32(OBJ + 24, -5i32 as u32).unwrap();
        dev.calls.clear();
        assert_eq!(voice_op(&mut g, &mut dev, OBJ).unwrap(), 0);
        assert_eq!(dev.calls, vec![format!("release {VOICE:x}")]);
        assert_eq!((g.u8(OBJ + 12).unwrap(), g.u8(OBJ + 13).unwrap()), (0, 2));
    }

    #[test]
    fn an_absent_descriptor_opens_nothing() {
        let mut g = guest();
        g.set_u16(TABLE + 16, 0xFFFF).unwrap();
        let mut dev = Mock::default();
        assert_eq!(voice_op(&mut g, &mut dev, OBJ).unwrap(), 0);
        assert!(dev.calls.is_empty());
        assert_eq!(g.u8(OBJ + 12).unwrap(), 1, "the state still latches");
    }

    #[test]
    fn property_ids_outside_the_table_are_dropped() {
        let mut g = guest();
        let mut dev = Mock::default();
        set_property(&mut g, &mut dev, VOICE, 137, 1).unwrap();
        set_property(&mut g, &mut dev, VOICE, -1i32 as u32, 1).unwrap();
        set_property(&mut g, &mut dev, VOICE, 6, -3i32 as u32).unwrap();
        set_property(&mut g, &mut dev, 0, 1, 1).unwrap();
        assert_eq!(dev.calls, vec![format!("set {VOICE:x} 6 0")]);
    }
}
