//! Skate 3 audio graph, ported from the shadow-verified native C++.
//!
//! Source: `skate3recomp-dev/src/skate3_audio_native.cpp`, each function of which was
//! checked against the original recompiled body under the shadow harness before being
//! translated here. Offsets come from `docs/rw_audio_structs.h` and are asserted below the
//! way `docs/rw_audio_structs_check.c` asserts them in C.
//!
//! Everything here operates on **guest memory**: big-endian, byte-addressed, at guest
//! addresses. That is deliberate rather than idiomatic. These are recovered layouts with
//! asserted offsets, and Phase 4's per-function criterion compares *bytes* against the
//! verified C++ — so a byte-addressed view makes that comparison direct, instead of routing
//! it through a serialisation step that could hide a discrepancy of its own.
//!
//! Unsafe is not forbidden crate-wide the way it is in `skate-audio-formats`, because the
//! DSP modules will scope `unsafe` to intrinsic calls. Nothing in this file needs it.

pub mod bitstream;
pub mod buffers;
pub mod counter;
pub mod cursors;
pub mod delay;
pub mod eval;
pub mod fp;
pub mod leaves;
pub mod mem;
pub mod player;
pub mod scheduler;
pub mod system;

/// The mixer's buffering and clearing layer.
///
/// Gated on x86_64 for one reason, and it is not SIMD: every body in these two modules emits
/// `ctx.fpscr.disableFlushModeUnconditional()` where the original does, through [`vmx::Fpscr`],
/// which models MXCSR. That is load-bearing rather than decorative — `rex/ppc/context.h` carries
/// flush-to-zero and denormals-are-zero on the **scalar** side too (see [`fp`]'s module note), so a
/// port that ran under Rust's default MXCSR would differ from the recomp on any denormal that
/// reached it. `dsp::biquad` adds a constant to every feed-forward sum for exactly that reason,
/// which is direct evidence denormals occur in this data.
///
/// [`ring::copy_from_ring`] and [`ring::fill_segments`] contain no float work at all and would run
/// anywhere; they are here because they share a structure with [`ring::fill_tail`], which does.
#[cfg(target_arch = "x86_64")]
pub mod mix;
#[cfg(target_arch = "x86_64")]
pub mod ring;

/// The VMX128 layer and the DSP kernels that stand on it.
///
/// Gated on x86_64 because they are a translation of RexGlue's **x86 lowering**, not of Xenon:
/// `core::arch::x86_64` has each of RexGlue's SSE4.1/FMA intrinsics one-for-one, which is the
/// entire reason `docs/vmx128-exactness.md` could measure the translation bit-identical. ARM64 is
/// out of scope there for the same reason and by the same decision (`docs/PLAN.md` non-goals): its
/// rule 4 behaviour would have to be re-measured before anything could be claimed about it.
#[cfg(target_arch = "x86_64")]
pub mod dsp;
#[cfg(target_arch = "x86_64")]
pub mod vmx;

/// The two-source crossfade and the mix dispatcher that runs it through a stage descriptor.
///
/// Gated with the rest: vector work under the guest's flush mode.
#[cfg(target_arch = "x86_64")]
pub mod crossfade;
/// The per-channel filter stages, which stand on [`dsp::biquad`] and [`mathlib::Trig`].
///
/// Gated with the rest for the same reason: both bodies normalise a cutoff with `fdivs`/`fmuls` and
/// clear their history with a rodata single, all under the guest's flush mode held through
/// [`vmx::Fpscr`], and the kernel they call adds a denormal-avoidance bias for that exact reason.
#[cfg(target_arch = "x86_64")]
pub mod filters;
/// The spatial layer, the gain plumbing under it, and the guest math leaves they call.
///
/// Gated on x86_64 for the same reason [`mix`] and [`ring`] are, and it is again not SIMD: every
/// body in these three modules is scalar float work that runs under the guest's flush mode, held
/// through [`vmx::Fpscr`]. `sub_82B453D8` and `sub_82B45788` both take a `fsqrts` of a sum that can
/// be denormal, and `sub_82F4DE80` subtracts two doubles whose difference can be; under Rust's
/// default MXCSR those would keep a denormal the recomp flushes to zero.
#[cfg(target_arch = "x86_64")]
pub mod gains;
#[cfg(target_arch = "x86_64")]
pub mod mathlib;
#[cfg(target_arch = "x86_64")]
pub mod spatial;
/// The one-pole filter stage and the dispatcher that runs it over a descriptor.
///
/// Gated with the rest: the stage is VMX128 work under the guest's flush mode.
#[cfg(target_arch = "x86_64")]
pub mod stage;

/// The scatter-mixer, which composes [`dsp::scale`]'s two kernels and [`mem::memset`].
///
/// Gated with them, because it calls them.
#[cfg(target_arch = "x86_64")]
pub mod routing;

/// One contribution's republish into its owner's running total, which reaches the image's log10.
///
/// Gated with the rest: every value it touches is single-rounded float work under the guest's flush
/// mode, held through [`vmx::Fpscr`].
#[cfg(target_arch = "x86_64")]
pub mod contributions;

/// The output stage — route, interleave, ramp once, clamp — which composes the modules around it.
///
/// Gated with them, because it calls them.
#[cfg(target_arch = "x86_64")]
pub mod output;

/// Voice and handle lifecycle, which reaches the gather and the fold's single-rounded adds.
///
/// Gated with them: it calls the x86 modules.
#[cfg(target_arch = "x86_64")]
pub mod voices;

/// The bus mixers: a source's channels into bus blocks, and a descriptor's into 1 KB runs.
#[cfg(target_arch = "x86_64")]
pub mod bus;

/// A speaker layout's parameter blocks expanded into per-channel slots.
#[cfg(target_arch = "x86_64")]
pub mod layout;

/// Per-channel mean-square and peak meters over ring histories, and the tick that runs them.
#[cfg(target_arch = "x86_64")]
pub mod meters;

/// A voice's pitch ratio and fractional position, single-rounded float work under the flush mode.
#[cfg(target_arch = "x86_64")]
pub mod pitch;

/// Planar channels into interleaved frames, the last shuffle before the driver.
///
/// Gated with the rest because every sample passes through an `lfs`/`stfs` pair under the guest's
/// flush mode, held through [`vmx::Fpscr`] by the kernels around it.
#[cfg(target_arch = "x86_64")]
pub mod interleave;

/// One contiguous span of guest memory.
#[derive(Clone, Debug)]
pub struct Segment {
    pub base: u32,
    pub bytes: Vec<u8>,
}

/// Guest memory as a set of spans.
///
/// Segmented rather than one flat slice, and that correction came from real data: the first
/// recorded vector put a buffer-pair object at `0x7018E110` on the guest stack with its buffers
/// at `0x401736D0` on the heap — about 768 MB apart, which no single slice can span. The flat
/// model was an assumption, and eleven synthetic tests passed against it happily because they
/// used one tidy contiguous window.
#[derive(Clone, Debug, Default)]
pub struct Guest {
    segments: Vec<Segment>,
    /// `(base, end, segment)` sorted by base; rebuilt whenever the layout changes.
    index: Vec<(u32, u64, usize)>,
    /// No two segments overlap, so at most one can contain any address and the indexed lookup
    /// returns exactly what the linear scan would.
    disjoint: bool,
    /// The segment of the previous hit; consecutive accesses are almost always to the same one.
    last: std::cell::Cell<usize>,
}

/// Out-of-window access, reported rather than panicking silently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub address: u32,
    pub message: String,
}

impl Error {
    pub fn new(address: u32, message: impl Into<String>) -> Self {
        Self {
            address,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "at {:#010x}: {}", self.address, self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

impl Guest {
    /// A single span, which is what the unit tests want.
    pub fn single(base: u32, len: usize) -> Self {
        Self::from_segments(vec![Segment {
            base,
            bytes: vec![0u8; len],
        }])
    }

    pub fn from_segments(segments: Vec<Segment>) -> Self {
        let mut guest = Self {
            segments,
            ..Self::default()
        };
        guest.reindex();
        guest
    }

    /// Add a span, or overwrite an existing one with the same base.
    pub fn put(&mut self, base: u32, bytes: Vec<u8>) {
        match self.segments.iter_mut().find(|s| s.base == base) {
            Some(s) => s.bytes = bytes,
            None => self.segments.push(Segment { base, bytes }),
        }
        self.reindex();
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    fn reindex(&mut self) {
        self.index = self
            .segments
            .iter()
            .enumerate()
            .map(|(i, s)| (s.base, u64::from(s.base) + s.bytes.len() as u64, i))
            .collect();
        self.index.sort_unstable_by_key(|&(base, _, _)| base);
        self.disjoint = self
            .index
            .windows(2)
            .all(|pair| pair[0].1 <= u64::from(pair[1].0));
        self.last.set(0);
    }

    fn locate(&self, ea: u32, len: usize) -> Result<(usize, usize)> {
        if self.disjoint {
            let fits = |i: usize| {
                let s = &self.segments[i];
                ea >= s.base && (ea - s.base) as usize + len <= s.bytes.len()
            };
            let last = self.last.get();
            if last < self.segments.len() && fits(last) {
                return Ok((last, (ea - self.segments[last].base) as usize));
            }
            // The only candidate is the segment with the greatest base not above `ea`.
            let at = self.index.partition_point(|&(base, _, _)| base <= ea);
            if at > 0 {
                let i = self.index[at - 1].2;
                if fits(i) {
                    self.last.set(i);
                    return Ok((i, (ea - self.segments[i].base) as usize));
                }
            }
            return Err(Error::new(ea, "no segment covers this address"));
        }
        for (i, s) in self.segments.iter().enumerate() {
            if ea >= s.base {
                let off = (ea - s.base) as usize;
                if off + len <= s.bytes.len() {
                    return Ok((i, off));
                }
            }
        }
        Err(Error::new(ea, "no segment covers this address"))
    }

    pub fn u32(&self, ea: u32) -> Result<u32> {
        let (i, o) = self.locate(ea, 4)?;
        let b = &self.segments[i].bytes;
        Ok(u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]))
    }

    pub fn set_u32(&mut self, ea: u32, value: u32) -> Result<()> {
        let (i, o) = self.locate(ea, 4)?;
        self.segments[i].bytes[o..o + 4].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }

    /// `lfd`/`ld`: eight big-endian bytes.
    ///
    /// Added for [`fp::load_double`], which `mathlib::floor` needs: `sub_82F4DE80` reaches its two
    /// pool constants with `lfd`, and splitting that into two `u32` reads would invent a byte order
    /// for the halves that the guest does not have.
    pub fn u64(&self, ea: u32) -> Result<u64> {
        let (i, o) = self.locate(ea, 8)?;
        let b = &self.segments[i].bytes;
        Ok(u64::from_be_bytes([
            b[o],
            b[o + 1],
            b[o + 2],
            b[o + 3],
            b[o + 4],
            b[o + 5],
            b[o + 6],
            b[o + 7],
        ]))
    }

    pub fn set_u64(&mut self, ea: u32, value: u64) -> Result<()> {
        let (i, o) = self.locate(ea, 8)?;
        self.segments[i].bytes[o..o + 8].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }

    pub fn u16(&self, ea: u32) -> Result<u16> {
        let (i, o) = self.locate(ea, 2)?;
        let b = &self.segments[i].bytes;
        Ok(u16::from_be_bytes([b[o], b[o + 1]]))
    }

    pub fn set_u16(&mut self, ea: u32, value: u16) -> Result<()> {
        let (i, o) = self.locate(ea, 2)?;
        self.segments[i].bytes[o..o + 2].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }

    pub fn u8(&self, ea: u32) -> Result<u8> {
        let (i, o) = self.locate(ea, 1)?;
        Ok(self.segments[i].bytes[o])
    }

    pub fn set_u8(&mut self, ea: u32, value: u8) -> Result<()> {
        let (i, o) = self.locate(ea, 1)?;
        self.segments[i].bytes[o] = value;
        Ok(())
    }

    /// `f32` from the guest's big-endian bits, without touching the value.
    pub fn f32(&self, ea: u32) -> Result<f32> {
        Ok(f32::from_bits(self.u32(ea)?))
    }

    pub fn fill(&mut self, ea: u32, value: u8, len: u32) -> Result<()> {
        let (i, o) = self.locate(ea, len as usize)?;
        self.segments[i].bytes[o..o + len as usize].fill(value);
        Ok(())
    }

    /// The bytes of the span starting exactly at `base`, for comparing against a recorded
    /// expectation.
    pub fn span(&self, base: u32, len: usize) -> Result<&[u8]> {
        let (i, o) = self.locate(base, len)?;
        Ok(&self.segments[i].bytes[o..o + len])
    }

    /// Write a block of bytes at `ea`, all or nothing.
    ///
    /// The byte-granular counterpart of [`Guest::span`], added for [`crate::vmx`]: a `stvx128` is
    /// sixteen bytes at one address and a `stvlx128` is a run of one to sixteen, neither of which
    /// decomposes into word stores without inventing an order the guest does not have.
    pub fn set_span(&mut self, ea: u32, bytes: &[u8]) -> Result<()> {
        let (i, o) = self.locate(ea, bytes.len())?;
        self.segments[i].bytes[o..o + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::Guest;

    /// Guest addresses used by the tests. The window is flat, so everything lives inside it.
    pub const BASE: u32 = 0x4000_0000;
    pub const SYSTEM: u32 = 0x4000_0000;
    pub const RING: u32 = 0x4000_0200;
    pub const PLAYER: u32 = 0x4000_0400;
    pub const PACKET_A: u32 = 0x4000_0600;
    pub const PACKET_B: u32 = 0x4000_0620;
    pub const PARAMS: u32 = 0x4000_0700;
    pub const SOURCE: u32 = 0x4000_0740;

    pub fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x800);
        wire(&mut g);
        g
    }

    /// A player wired to a system with an empty ring, and a source object.
    pub fn wire(g: &mut Guest) {
        g.set_u32(PLAYER + crate::system::PLAYER_SYSTEM, SYSTEM)
            .unwrap();
        g.set_u32(SYSTEM + crate::system::SYSTEM_CMD_BUFFER, RING)
            .unwrap();
        g.set_u32(SYSTEM + crate::system::SYSTEM_CMD_WRITE_OFF, 0)
            .unwrap();
        g.set_u32(PLAYER + crate::player::PLAYER_SOURCE, SOURCE)
            .unwrap();
    }
}

/// The `.csi` symbol tables at run time: install a project, resolve a symbol into a slot.
/// Unverified new work; see the module note.
pub mod symbols;

/// Patch banks at run time: load an `.abk`, post to an object, spawn evaluator instances.
/// Unverified new work; see the module note.
pub mod patch;

/// The evaluator voice op and the voice-object helpers, against a host voice device.
/// Unverified; see the module note.
pub mod voice;

/// A stream object's frame delivery with a host fill function. Unverified; see the module note.
pub mod stream;

/// SndPlayer1's block render and fade. Unverified; see the module note.
pub mod sndplayer;

/// A voice graph's per-block pass and node mixer fill. Unverified; see the module note.
pub mod graph;

/// The voice classes' kernels by address, a GraphHost for a voice graph. Routing unverified.
pub mod kernels;

/// Voice graph construction: the module classes' sizes and constructors, the builder and its install
/// command. Unverified; see the module note.
pub mod modules;

/// The voice module classes: registration, cooked parameter defaults, and a parameter's default block.
/// Unverified; see the module note.
pub mod classes;

/// The voice device's open: a voice object over a built module graph, its sends and angles. Unverified;
/// see the module note.
pub mod device;

/// Audio-thread command dispatch and snapshot drain (`sub_82B48530`, phase four).
pub mod commands;
/// Dac snapshot publication and completed-pass close-out (`sub_82B219E8`).
pub mod dac;
/// Decoded resident PCM cursors used by the recovered SndPlayer1 stream fill.
pub mod pcm;
/// Resident SndPlayer1 play-command branch (`sub_82B32DC8`).
pub mod play;
/// A continuously pumpable worker fixture for exercising the recovered final output graph on a
/// host device. It is deliberately a probe signal, not a replacement for authored game voices.
#[cfg(target_arch = "x86_64")]
pub mod probe;
/// SndPlayer1 segment-ring stream pump (`sub_82B31EE0`).
pub mod pump;
/// The recovered five-phase `rw_system` audio-thread block driver.
pub mod runtime;
/// The Dac worker's double-buffer PCM handoff.
pub mod worker;

/// Single-owner authored bank evaluator, voice graphs, decoded streams and Dac output.
#[cfg(target_arch = "x86_64")]
pub mod authored;

/// Retail board rolling: the grain player over `grains.big`, its seek reader and the board math.
#[cfg(target_arch = "x86_64")]
pub mod grain;

/// The retail MixMap mixer: controller inputs → every gain/pitch/filter output word.
///
/// Gated with the float modules: its evaluation holds the guest flush mode through [`vmx::Fpscr`].
#[cfg(target_arch = "x86_64")]
pub mod mixmap;
