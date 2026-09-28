//! The retail MixMap mixer (EA PathFinder 5.03, `packages\engine\audio\path\5.03.00-sk8`), which
//! turns the game's per-object controller inputs into every gain, pitch and filter word the player
//! sounds post. Transliterated from the lifted recomp C++ (`skate3_recomp.50.cpp`/`.51.cpp`) to run
//! on guest memory, byte for byte; image constants and tables are read from the guest image.
//!
//! **Unverified against a shadow harness** (these bodies run on the game thread; no C++ reference
//! exists in this crate). Verified instead against the retail capture — see
//! `examples/mixmap_replay.rs` and the tests.
//!
//! # How the game uses it
//!
//! * **Load** (`sub_82484FE8`, once at audio-system init, after the 14 SFX slot objects exist):
//!   [`load`] = `sub_8294B918` (manager init) + the file load (`sub_8298ED88`, 16-byte aligned) +
//!   `sub_8294BA18` (build).
//! * **Binding.** Every controller key names one SFX object: bits 29–31 kind (`010` SFXObj,
//!   `011` SFXCTL), bits 16–23 the SFX slot, bits 11–15 the instance (group), bits 4–10 the object's
//!   id within its slot, bits 0–3 the input/output id. When the build creates a controller it hands
//!   it to the manager's slot 4 — the game's `sub_82485850`, which walks
//!   `sys[164 + slot]` → groups (`+16` list, id at `+16`) → objects (`+36` list for `010`, `+32` for
//!   `011`, id at `(obj+24 >> 4) & 0x7F`) and stores the controller at **`object+12`**. That is the
//!   `[owner+12]` every owner vfunc reads. See [`Listener`] and [`find_controller`].
//! * **Per game frame**, in `sub_82485190`: the audio-state bridge `sub_824B0DA8` ends with
//!   `sub_824B19C8`, which writes the state controller's inputs; then the manager tick
//!   `sub_8294BAE8` → host slot 2 [`evaluate`]; then the components' process (+36) and update
//!   (+40) slots read the outputs of *this* evaluation. Inputs a component writes during its own
//!   update (e.g. the SkateBoard's id 4 in `sub_824C6198`) are consumed by the *next* evaluation.
//!   The capture's line order (`ST` … `MX`/`MC` … `UP`/`VF` … `BD` … next `ST`) agrees.
//!
//! # Layout
//!
//! The host (564 bytes, vtable `0x82316B78`) owns flat arrays, one per stage, sized by a first pass
//! over the file ([`offsets`]). Five record kinds per section: **A** products of an input curve
//! and constants, **B** 2-D position/range lookups on an SFXCTL controller's float inputs, **F**
//! envelopes, **C** clamped sums, **E** output sums written as packed int16 into the controller's
//! 64-byte output block.

pub mod build;
pub mod controller;
pub mod eval;
pub mod inputs;
pub mod tables;

use crate::patch::Heap;
use crate::{Error, Guest, Result};

pub use build::{KEY_MASK, check_capacities, resolve};
pub use eval::evaluate;

/// Host field offsets used across the builders and the evaluator.
pub mod offsets {
    /// `[host+8 + 4·slot]`: instances of each SFX slot (from the manager's slot 8).
    pub const SLOT_INSTANCES: u32 = 8;
    /// The manager (`sys+4`), whose slots 4/8 bind controllers and count instances.
    pub const MANAGER: u32 = 108;
    /// The descriptor X.
    pub const DESCRIPTOR: u32 = 112;
    /// The file data.
    pub const DATA: u32 = 116;
    /// Previous and current mode word (`r5` of [`super::evaluate`]).
    pub const PREV_MODE: u32 = 124;
    pub const MODE: u32 = 128;
    /// dt (s) and dt × 1000 (ms).
    pub const DT: u32 = 132;
    pub const DT_MS: u32 = 136;
    /// The controller pointer array and its storage.
    pub const CTRL_PTRS: u32 = 160;
    /// Σ instances over the slots with a section.
    pub const INSTANCES_TOTAL: u32 = 168;
    /// Controllers allocated (`sub_8294E120`) and used.
    pub const CTRL_CAP: u32 = 172;
    pub const CTRL_COUNT: u32 = 180;
    /// Input entries allocated.
    pub const INPUT_CAP: u32 = 208;
}

/// The manager's slots 4 and 8, which retail implements in the game (`sub_82485850`,
/// `sub_82485980`, vtable `0x822FBEC8` at `sys+4`).
pub trait Listener {
    /// Slot 8: the number of instances (groups) of SFX slot `slot` — `[sys[164 + slot] + 20]`.
    fn instance_count(&mut self, g: &mut Guest, slot: u32) -> Result<u32>;
    /// Slot 4: bind a newly created controller (its key is at `ctrl+4`). Retail stores it at the
    /// owning object's `+12` and returns 1, or calls the controller's slot 20 ([`controller::disable`])
    /// and returns 0. The MixMap ignores the result; a disable here is overwritten by
    /// `sub_8294F228`, which zeroes every output block and sets its enable word to 1.
    fn bind(&mut self, g: &mut Guest, ctrl: u32) -> Result<u32>;
}

/// Instances per SFX slot in the retail session, **derived from the capture**, not from code: the
/// groups seen in the 247 controller keys (slot 1 = players: 2; slot 5 = pedestrians: 15; …).
/// Retail reads `[sys[164+slot]+20]` at load time; the slot objects' constructors were not traced.
/// Building with these reproduces the capture's 247 controllers and its exact key set.
pub const RETAIL_INSTANCE_COUNTS: [u32; 14] = [1, 2, 1, 10, 4, 15, 5, 5, 5, 3, 3, 0, 5, 7];

/// A listener with fixed instance counts that records every bound controller by key and reports
/// success. The Rust engine's owners find their controller by key ([`find_controller`]) rather than
/// being written into, so nothing here touches an owner.
#[derive(Clone, Debug, Default)]
pub struct KeyedListener {
    pub counts: Vec<u32>,
    pub bound: Vec<(u32, u32)>,
}

impl KeyedListener {
    pub fn retail() -> Self {
        Self {
            counts: RETAIL_INSTANCE_COUNTS.to_vec(),
            bound: Vec::new(),
        }
    }
}

impl Listener for KeyedListener {
    fn instance_count(&mut self, _g: &mut Guest, slot: u32) -> Result<u32> {
        Ok(self.counts.get(slot as usize).copied().unwrap_or(0))
    }
    fn bind(&mut self, g: &mut Guest, ctrl: u32) -> Result<u32> {
        self.bound.push((g.u32(ctrl + 4)?, ctrl));
        Ok(1)
    }
}

/// A loaded MixMap: the manager object, its host and where the file sits.
#[derive(Clone, Copy, Debug)]
pub struct MixMap {
    pub manager: u32,
    pub host: u32,
    pub data: u32,
}

/// The game system's manager vtable (the derived class at `sys+4`).
pub const MANAGER_VTABLE: u32 = 0x822F_BEC8;

/// `sub_82484FE8`'s MixMap half: allocate the manager object (the 32 bytes of `sys+4` the MixMap
/// uses: vtable, `+4` mode = −1 as `sub_824845B8` leaves it, `+8` X, `+28` Y), init it with 14 slots,
/// copy the file into a 16-byte aligned guest buffer, build.
pub fn load(
    g: &mut Guest,
    heap: &mut dyn Heap,
    listener: &mut dyn Listener,
    file: &[u8],
) -> Result<MixMap> {
    let manager = heap.alloc(g, 32, 16)?;
    if manager == 0 {
        return Err(Error::new(0, "guest heap exhausted (manager)"));
    }
    for w in 0..8 {
        g.set_u32(manager + 4 * w, 0)?;
    }
    g.set_u32(manager, MANAGER_VTABLE)?;
    g.set_u32(manager + 4, 0xFFFF_FFFF)?;
    build::init_manager(g, heap, manager, 14)?;
    let data = heap.alloc(g, file.len() as u32, 16)?;
    if data == 0 {
        return Err(Error::new(0, "guest heap exhausted (MixMap file)"));
    }
    g.set_span(data, file)?;
    let host = build::build_host(g, heap, listener, manager, data)?;
    Ok(MixMap {
        manager,
        host,
        data,
    })
}

/// `sub_8294BAE8(mgr, dt)` — the manager's per-frame tick: the Y object (`sub_8294BF40`, inert
/// unless `Y+4 == 2`), then, once the descriptor says built (`X+112`), host slot 2 with the
/// manager's `+4` as the mode.
pub fn tick(g: &mut Guest, manager: u32, dt: f64) -> Result<()> {
    let y = g.u32(manager + 28)?;
    if y != 0 && g.u32(y + 4)? == 2 {
        return Err(Error::new(
            0x8294_BF40,
            "MixMap Y object in state 2: not ported",
        ));
    }
    let x = g.u32(manager + 8)?;
    if x == 0 || g.u8(x + 112)? == 0 {
        return Ok(());
    }
    let host = g.u32(x)?;
    let mode = g.u32(manager + 4)?;
    evaluate(g, host, mode, dt)
}

/// The controller whose (masked) key is `key`, by the same search `sub_8294CB48` does.
pub fn find_controller(g: &Guest, host: u32, key: u32) -> Result<Option<u32>> {
    let k = key & KEY_MASK;
    let count = g.u32(host + offsets::CTRL_COUNT)?;
    let ptrs = g.u32(host + offsets::CTRL_PTRS)?;
    for i in 0..count {
        let c = g.u32(ptrs + 4 * i)?;
        if g.u32(c + 4)? == k {
            return Ok(Some(c));
        }
    }
    Ok(None)
}

/// Every controller, `(ctrl, key)`, in creation order.
pub fn controllers(g: &Guest, host: u32) -> Result<Vec<(u32, u32)>> {
    let count = g.u32(host + offsets::CTRL_COUNT)?;
    let ptrs = g.u32(host + offsets::CTRL_PTRS)?;
    (0..count)
        .map(|i| {
            let c = g.u32(ptrs + 4 * i)?;
            Ok((c, g.u32(c + 4)?))
        })
        .collect()
}

/// A controller key: `kind` 2 = SFXObj (`0x4…`), 3 = SFXCTL (`0x6…`).
pub const fn key(kind: u32, slot: u32, group: u32, object: u32) -> u32 {
    (kind << 29) | (slot << 16) | (group << 11) | (object << 4)
}

#[cfg(test)]
mod tests;
