//! The MixMap inside [`AuthoredRuntime`]'s guest: load, per-frame tick, controller lookup, input
//! writes and the owner readers (vfunc 52/56/60/64).
//!
//! The MixMap lives in its own guest span at [`MIXMAP_SPACE`], whose first 256 bytes are a header
//! (manager, host, file data, heap cursor) so no field is added to the runtime itself. Everything
//! inside the span is built by [`crate::mixmap::load`] exactly as `sub_82484FE8` builds it; the
//! image tables it reads come from the runtime's guest image.
//!
//! **Per-frame order** (the retail call `sub_82485190`, confirmed by the capture's line order):
//!
//! 1. First half (`sub_82485190`'s `r29` branch, the slot objects' vfunc 16): the audio-state
//!    bridge `sub_824B0DA8` (the capture's `ST`), which ends in the state controller's inputs
//!    ([`crate::mixmap::inputs::state_inputs`], plus ids 3 and 12 it writes on the way); then every
//!    component's process slot (+36), which writes the owners' inputs (SkateBoard ids 0/4/6 in
//!    `sub_824C5CA8`/`sub_824C6198`, Rail, OffBoard, the position controllers' `sub_824AEC70`, …).
//! 2. Second half (`r27` branch): [`AuthoredRuntime::mixmap_tick`] (`sub_8294BAE8` →
//!    `sub_8294F5E8`, the capture's `MX`), then the slot objects' vfunc 20: every component's update
//!    slot (+40, the capture's `UP`/`BD`), reading **this** evaluation's outputs with
//!    [`AuthoredRuntime::mixmap_raw`]/[`pitch`](AuthoredRuntime::mixmap_pitch)/[`level`](AuthoredRuntime::mixmap_level).
//! 3. Inputs written during an update (+40) are consumed by the next frame's evaluation.
//!
//! **The dt alternation.** `sub_82485190` accumulates: when its dt is ≤ 0.02 s (`0x822F8DE8`) and
//! its `r5` is 0, one call runs only the first half and the next only the second, each with the sum
//! of the two most recent calls' dt (`sys+492`/`+496`); above 0.02 s, or with `r5` set, both halves
//! run every call with the call's own dt. The capture shows the game calling it every ≈7.4 ms, so
//! each half ran every ≈14.7 ms with dt ≈ 0.0147. A Rust worker at a fixed 60 Hz should run **both
//! halves every frame with dt = 1/60** — that is what retail does at 120 calls/s, and exactly what
//! it does itself for any call whose dt exceeds 0.02. Calling a single-call-per-frame alternation at
//! 60 Hz would instead run each half at 30 Hz with dt = 1/30.

use super::AuthoredRuntime;
use crate::mixmap::{self, KeyedListener, controller};
use crate::patch::BumpHeap;
use crate::{Error, Result};

/// Guest base of the MixMap span (after the graph heap, `0x6400_0000 + 64 MB`).
pub const MIXMAP_SPACE: u32 = 0x6800_0000;
/// The span's size: the build uses about 230 KB of it.
pub const MIXMAP_BYTES: u32 = 0x0040_0000;
const MAGIC: u32 = 0x4D58_4D50; // "MXMP"
const HEADER: u32 = 0x100;

impl AuthoredRuntime {
    /// `sub_82484FE8`'s MixMap half: build the host from `MixMapSK8.mxb` (see
    /// `PlayerAudioCatalog::mixmap`). The instance counts per SFX slot are the retail session's
    /// ([`mixmap::RETAIL_INSTANCE_COUNTS`]). Replaces any MixMap loaded before.
    pub fn load_mixmap(&mut self, mxb: &[u8]) -> Result<()> {
        let g = &mut self.guest;
        g.put(MIXMAP_SPACE, vec![0; MIXMAP_BYTES as usize]);
        let mut heap = BumpHeap {
            next: MIXMAP_SPACE + HEADER,
            end: MIXMAP_SPACE + MIXMAP_BYTES,
        };
        let mut listener = KeyedListener::retail();
        let mm = mixmap::load(g, &mut heap, &mut listener, mxb)?;
        g.set_u32(MIXMAP_SPACE + 4, mm.manager)?;
        g.set_u32(MIXMAP_SPACE + 8, mm.host)?;
        g.set_u32(MIXMAP_SPACE + 12, mm.data)?;
        g.set_u32(MIXMAP_SPACE + 16, heap.next)?;
        g.set_u32(MIXMAP_SPACE, MAGIC)?;
        Ok(())
    }

    /// Whether [`Self::load_mixmap`] has run.
    pub fn has_mixmap(&self) -> bool {
        self.guest.u32(MIXMAP_SPACE).is_ok_and(|m| m == MAGIC)
    }

    fn mixmap_header(&self) -> Result<(u32, u32)> {
        if !self.has_mixmap() {
            return Err(Error::new(MIXMAP_SPACE, "MixMap not loaded"));
        }
        Ok((
            self.guest.u32(MIXMAP_SPACE + 4)?,
            self.guest.u32(MIXMAP_SPACE + 8)?,
        ))
    }

    /// The manager's per-frame tick (`sub_8294BAE8` → host slot 2, `sub_8294F5E8`).
    pub fn mixmap_tick(&mut self, dt: f32) -> Result<()> {
        let (manager, _) = self.mixmap_header()?;
        mixmap::tick(&mut self.guest, manager, f64::from(dt))
    }

    /// The controller for a key (`mixmap::key(kind, slot, group, object)`), as `sub_8294CB48` finds
    /// it; `None` before [`Self::load_mixmap`] or for a key the file never names.
    pub fn mixmap_controller(&self, key: u32) -> Option<u32> {
        let (_, host) = self.mixmap_header().ok()?;
        mixmap::find_controller(&self.guest, host, key)
            .ok()
            .flatten()
    }

    /// Controller slot 8 (`sub_8294BC50`): `[ctrl+8][id] = value`.
    pub fn mixmap_set(&mut self, ctrl: u32, id: u32, value: u32) -> Result<()> {
        controller::set(&mut self.guest, ctrl, id, value)
    }

    /// Controller slot 4 (`sub_8294BC68`): `fctiwz` then slot 8.
    pub fn mixmap_set_float(&mut self, ctrl: u32, id: u32, value: f32) -> Result<()> {
        controller::set_float(&mut self.guest, ctrl, id, f64::from(value))
    }

    /// Store the float's bits unconverted, as the position controllers' writers do for ids 0, 1,
    /// 13 and 14 (`stfs` then `lwz` into slot 8, e.g. `sub_824AEE60`'s `lwz r5,96(r31)`).
    pub fn mixmap_set_bits(&mut self, ctrl: u32, id: u32, value: f32) -> Result<()> {
        controller::set(&mut self.guest, ctrl, id, value.to_bits())
    }

    /// Apply a writer's `(id, word)` list (see [`crate::mixmap::inputs`]) to the controller with
    /// `key`, in order, through slot 8. A key the MixMap does not name is ignored, as retail's
    /// `[owner+12] == 0` check ignores an unbound owner.
    pub fn mixmap_apply(&mut self, key: u32, writes: &[(u32, u32)]) -> Result<()> {
        let Some(ctrl) = self.mixmap_controller(key) else {
            return Ok(());
        };
        for &(id, word) in writes {
            controller::set(&mut self.guest, ctrl, id, word)?;
        }
        Ok(())
    }

    /// SFXObj_Jitter's process (`sub_824EF378`) against this guest's shared random generator
    /// (`sub_82A8AF10`, the one the grain player draws from), writing controller `0x400000E0`.
    pub fn mixmap_jitter_tick(&mut self, jitter: &mut crate::mixmap::inputs::Jitter) -> Result<()> {
        let writes = jitter.process(&mut self.guest)?;
        self.mixmap_apply(0x4000_00E0, &writes)
    }

    /// Controller slot 12 (`sub_8294BCA0`).
    pub fn mixmap_get(&self, ctrl: u32, id: u32) -> Result<u32> {
        controller::get(&self.guest, ctrl, id)
    }

    /// Owner vfunc 52 (`sub_824C2870`): the output half, 16 bits.
    pub fn mixmap_raw(&self, ctrl: u32, id: u32) -> u32 {
        controller::read_u16(&self.guest, ctrl, id).unwrap_or(0)
    }

    /// Owner vfunc 56 (`sub_824C5910`): cents → `2^(c/1200) × 4096`.
    pub fn mixmap_pitch(&self, ctrl: u32, id: u32) -> u32 {
        controller::read_pitch(&self.guest, ctrl, id).unwrap_or(0)
    }

    /// Owner vfunc 60/64 (`sub_824AF240`): the output half `& 0x7FFF`.
    pub fn mixmap_level(&self, ctrl: u32, id: u32) -> u32 {
        controller::read_gain(&self.guest, ctrl, id).unwrap_or(0)
    }

    /// Controller slot 24 (`sub_8294BDB0`): output word 15 = 1.
    pub fn mixmap_enable(&mut self, ctrl: u32) -> Result<()> {
        controller::enable(&mut self.guest, ctrl)
    }

    /// Controller slot 20 (`sub_8294BD98`): output word 15 = 0 (the evaluation then writes each
    /// block's first output as its "off" value).
    pub fn mixmap_disable(&mut self, ctrl: u32) -> Result<()> {
        controller::disable(&mut self.guest, ctrl)
    }
}
