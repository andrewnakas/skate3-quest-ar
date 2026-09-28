//! What a concrete runtime owner needs to host grain players: the grain files placed in guest
//! memory with their decoded PCM, the resident-PCM form of the retail seek, the scheduler bucket-0
//! tick that runs the players' plug-in, and the game-side API (create, bind, record, stop).
//!
//! **How the retail scheduler runs the plug-in.** `sub_828EBE68` registers each bound player's
//! instance (`player+148`) in bucket 0 of the audio system's scheduler (`sub_82481BE0`, pool at
//! `system+112`). The per-block driver `sub_82B48530` ticks bucket 0 in its phase 1, before the
//! command drain (phase 4) and the graph pass, calling `[instance+4]` = `sub_828ECE98` (which
//! branches to `sub_828EC6F0`) with `[instance+8]` = the player and `f1 = [system+176]`.
//! [`tick_bucket_zero`] is exactly that phase-1 call over the recovered [`scheduler::tick_bucket`].
//!
//! **The delta.** `[system+176]` (`scheduler+64`) is not written by anything the crate ports. The
//! retail capture pins it: a fresh voice's attack timer runs `0.1 → 0x3DAC0831 → 0x3D962FC9 →
//! 0x3D54FDF2 → 0x3D294D22` one block at a time, which is `fsubs` of exactly `0x3BAEC33E` =
//! `(f32) 256/48000`. [`install`] stores that single when the cell is still zero. The store
//! itself (which runs at audio-system creation) was not located and is reported as such.
//!
//! **The seek, resident.** The retail play command turns the grain's start time into a frame
//! (`fctiwz(rate × start)`, [`seek::start_frame`]), seeks the XMA stream through the seek table
//! ([`seek::seek`]) and re-arms the decoder at the entry with a preroll and skip. Here the stream
//! is already decoded, so the host verifies that the retail seek lands on that frame (entry +
//! preroll + skip = frame, an error otherwise) and hands the SndPlayer1 stream the decoded PCM
//! from that frame on; the slot's queued count and the decoder's end become the frames remaining.
//! The slot/record stores of `sub_82B33780` ([`seek::apply_seek_fields`]) are not applied, because
//! the resident cursor replaces the decoder state they re-arm.

use std::collections::HashMap;
use std::sync::Arc;

use crate::fp::load_single;
use crate::mathlib::Image;
use crate::patch::Heap;
use crate::pcm::{CachedPcm, PcmSource, PcmStreams};
use crate::scheduler::{self, SchedulerHost};
use crate::{Error, Guest, Result, classes, play};

use super::board::{ChainValues, GrainRecord};
use super::{chain, player, seek, stream};

/// `(f32) 256/48000`, the block delta the capture shows the plug-in receiving.
pub const BLOCK_DELTA_BITS: u32 = 0x3BAE_C33E;
/// `scheduler + 64` in `rw_system`.
pub const SCHEDULER_DELTA: u32 = scheduler::SYSTEM_SCHEDULER + 64;

/// One grain file in guest memory, with its decoded stream.
#[derive(Clone, Debug)]
pub struct GrainSource {
    pub name: String,
    /// The member's first byte (`player+48`).
    pub data: u32,
    /// `data + H`, the EAAC stream: the play command's sample key.
    pub stream: u32,
    pub samples: Arc<[i16]>,
    pub channels: u8,
    pub rate: u32,
}

/// A grain voice start the host carried out, for diagnostics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StartEvent {
    /// The SndPlayer1 module that plays it.
    pub stream: u32,
    /// The grain start in seconds (the play command's `+24`).
    pub seconds: f64,
    /// `fctiwz(rate × seconds)`, the first decoded frame.
    pub frame: u32,
    pub seek: Option<seek::SeekResult>,
}

struct Pending {
    stream: u32,
    key: u32,
    span: f64,
}

/// The grain state a runtime owner keeps beside its device.
#[derive(Default)]
pub struct GrainRuntime {
    sources: HashMap<u32, GrainSource>,
    names: HashMap<String, u32>,
    players: Vec<u32>,
    pending: Option<Pending>,
    installed: bool,
    clock: u32,
    /// 128-byte decoder blocks allocated ahead, see [`GrainRuntime::take_decoder`].
    spare: Vec<u32>,
    /// Each player's bus-chain record (`chain`), built once.
    chains: HashMap<u32, u32>,
    /// Voice slots of resident-stream owners ([`Grains::reserve_voices`]), each of which may start
    /// a SndPlayer1 between two decoder top-ups.
    stream_voices: usize,
    /// Every voice start, oldest first. The owner may drain it.
    pub starts: Vec<StartEvent>,
}

impl GrainRuntime {
    pub fn source(&self, name: &str) -> Option<&GrainSource> {
        self.names.get(name).and_then(|key| self.sources.get(key))
    }

    pub fn players(&self) -> &[u32] {
        &self.players
    }

    /// Called by the owner's play handler before `play::append_resident`: when the request plays a
    /// grain stream, remember it for [`GrainRuntime::seek`] and return the SndPlayer1 module so the
    /// owner can give it a decoder (grain graphs are not opened through the voice device).
    pub fn begin_play(&mut self, g: &Guest, request: u32) -> Result<Option<u32>> {
        let key = g.u32(request + play::REQUEST_OFFSET)?;
        if !self.sources.contains_key(&key) {
            self.pending = None;
            return Ok(None);
        }
        let stream = g.u32(request + play::REQUEST_STREAM)?;
        let span = f64::from_bits(g.u64(request + play::REQUEST_SPAN)?);
        self.pending = Some(Pending { stream, key, span });
        Ok(Some(stream))
    }

    /// A decoder block for a grain SndPlayer1, allocated before the command drain: the owner's
    /// heap is not available to the play handler, which runs inside the drain.
    pub fn take_decoder(&mut self) -> Option<u32> {
        self.spare.pop()
    }

    pub fn end_play(&mut self) {
        self.pending = None;
    }

    /// Called from the owner's `PlayHost::decode` for `stream`: apply the retail start offset of
    /// the pending grain request by attaching the decoded PCM from that frame to `decoder`.
    pub fn seek(
        &mut self,
        g: &mut Guest,
        streams: &mut PcmStreams,
        stream: u32,
        index: u8,
        detail: u32,
        decoder: u32,
        sp: u32,
    ) -> Result<()> {
        let Some(pending) = self.pending.as_ref().filter(|p| p.stream == stream) else {
            return Ok(());
        };
        let source = self
            .sources
            .get(&pending.key)
            .ok_or_else(|| Error::new(pending.key, "grain source vanished"))?;
        let slot = play::slot_address(g, stream, index)?;
        let record = play::record_address(g, stream, index)?;
        let rate = g.f32(slot + play::SLOT_RATE)?;
        let frame = seek::start_frame(
            rate,
            pending.span,
            g.u8(record + 73)?,
            g.u32(slot + play::SLOT_CURSOR)?,
        );
        let mut event = StartEvent {
            stream,
            seconds: pending.span,
            frame: 0,
            seek: None,
        };
        if frame > 0 && detail != 0 {
            let decode_sp = sp.wrapping_sub(seek::DECODE_FRAME);
            let out = decode_sp + 80;
            let status = seek::seek(g, out, detail, frame as u32, decode_sp)?;
            let result = seek::SeekResult::read(g, out, status)?;
            if status != 0 || result.output_frame() != frame as u32 {
                return Err(Error::new(
                    detail,
                    format!(
                        "{}: the retail seek to frame {frame} landed on {:?}",
                        source.name, result
                    ),
                ));
            }
            let channels = usize::from(source.channels);
            let total = source.samples.len() / channels;
            let start = frame as usize;
            if start >= total {
                return Err(Error::new(detail, "grain start is past the decoded stream"));
            }
            let tail = PcmSource::new(
                Arc::from(&source.samples[start * channels..]),
                source.channels,
            )?;
            let remaining = (total - start) as u32;
            streams.attach(decoder, tail);
            g.set_u32(slot + play::SLOT_QUEUED, remaining)?;
            g.set_u32(decoder + 64 + 12, remaining)?;
            g.set_u32(stream + 436, remaining)?;
            event.frame = frame as u32;
            event.seek = Some(result);
        }
        self.starts.push(event);
        Ok(())
    }
}

/// One-time setup, retail's boot-side registrations the players rely on: `GaF0` in the class
/// registry (`sub_828DD590`), the chain's `FSS0` (`sub_824C8798`) and `HS20` (`sub_828DD590`), and
/// the scheduler delta (see the module note).
pub fn install<H: Heap + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    runtime: &mut GrainRuntime,
) -> Result<()> {
    if runtime.installed {
        return Ok(());
    }
    let system = g.u32(crate::modules::SYSTEM)?;
    let registry = classes::class_registry(g, heap, system)?;
    classes::register_class(g, registry, player::FADER_DESCRIPTOR)?;
    chain::register_classes(g, heap)?;
    if g.u32(system + SCHEDULER_DELTA)? == 0 {
        g.set_u32(system + SCHEDULER_DELTA, BLOCK_DELTA_BITS)?;
    }
    runtime.installed = true;
    Ok(())
}

struct TickHost<'a, H: Heap + ?Sized> {
    heap: &'a mut H,
    clock: &'a mut u32,
    sp: u32,
}

impl<H: Heap + ?Sized> SchedulerHost for TickHost<'_, H> {
    fn timebase(&mut self) -> u32 {
        *self.clock = self.clock.wrapping_add(1);
        *self.clock
    }
    fn process(&mut self, g: &mut Guest, function: u32, context: u32, delta: f32) -> Result<()> {
        match function {
            player::PLUG_IN => {
                player::tick(g, self.heap, &mut Image, context, f64::from(delta), self.sp)
            }
            other => Err(Error::new(
                other,
                "scheduler bucket 0 plug-in is not a grain player",
            )),
        }
    }
}

/// Phase 1 of the retail block driver for grain players: tick scheduler bucket 0.
pub fn tick_bucket_zero<H: Heap + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    runtime: &mut GrainRuntime,
    sp: u32,
) -> Result<()> {
    if !runtime.installed {
        return Ok(());
    }
    // Keep enough decoder blocks for every voice a tick can start before the next top-up.
    let wanted = 2 * runtime.players.len() + runtime.stream_voices + 2;
    while runtime.spare.len() < wanted {
        let at = heap.alloc(g, 128, 16)?;
        if at == 0 {
            return Err(Error::new(
                0,
                "guest heap exhausted allocating a grain decoder",
            ));
        }
        runtime.spare.push(at);
    }
    let system = g.u32(crate::modules::SYSTEM)?;
    let mut host = TickHost {
        heap,
        clock: &mut runtime.clock,
        sp,
    };
    scheduler::tick_bucket(g, &mut host, system + scheduler::SYSTEM_SCHEDULER, 0)
}

/// The game-side API over a runtime owner's guest, device heap, decoded-source table and grain
/// state. Obtain one from the owner (`AuthoredRuntime::grains`).
pub struct Grains<'a> {
    pub g: &'a mut Guest,
    pub heap: &'a mut dyn Heap,
    pub sources: &'a mut HashMap<u32, CachedPcm>,
    pub runtime: &'a mut GrainRuntime,
    pub sp: u32,
}

impl Grains<'_> {
    /// Place a `.grain` member in guest memory, unmodified, as the loader `sub_828DC158` does, and
    /// register its decoded stream (`samples`, interleaved) under the stream's address.
    pub fn load(
        &mut self,
        name: &str,
        bytes: &[u8],
        samples: Arc<[i16]>,
        channels: u8,
        rate: u32,
    ) -> Result<u32> {
        install(self.g, self.heap, self.runtime)?;
        if let Some(&key) = self.runtime.names.get(name) {
            return Ok(self.runtime.sources[&key].data);
        }
        let grain = skate_grain_header(bytes)?;
        let data = self.heap.alloc(self.g, bytes.len() as u32, 16)?;
        if data == 0 {
            return Err(Error::new(0, "guest heap exhausted placing a grain file"));
        }
        self.g.set_span(data, bytes)?;
        let stream = data + grain;
        let source = PcmSource::new(samples.clone(), channels)?;
        self.sources.insert(
            stream,
            CachedPcm {
                source,
                sample_rate: rate,
            },
        );
        self.runtime.sources.insert(
            stream,
            GrainSource {
                name: name.to_owned(),
                data,
                stream,
                samples,
                channels,
                rate,
            },
        );
        self.runtime.names.insert(name.to_owned(), stream);
        Ok(data)
    }

    /// Place a resident `.snr` (an EAAC header at offset 0) in guest memory, unmodified, as the
    /// resource loader `sub_828DC158` leaves it, and register its decoded stream under its own
    /// address: the address is the SndPlayer1 play request's stream (`SFXObj_Wheels`).
    pub fn load_resident(
        &mut self,
        name: &str,
        bytes: &[u8],
        samples: Arc<[i16]>,
        channels: u8,
        rate: u32,
    ) -> Result<u32> {
        install(self.g, self.heap, self.runtime)?;
        if let Some(&key) = self.runtime.names.get(name) {
            return Ok(key);
        }
        let data = self.place(bytes)?;
        let source = PcmSource::new(samples.clone(), channels)?;
        self.sources.insert(
            data,
            CachedPcm {
                source,
                sample_rate: rate,
            },
        );
        self.runtime.sources.insert(
            data,
            GrainSource {
                name: name.to_owned(),
                data,
                stream: data,
                samples,
                channels,
                rate,
            },
        );
        self.runtime.names.insert(name.to_owned(), data);
        Ok(data)
    }

    /// Copy `bytes` into a fresh 16-aligned guest block (`sub_8298ED88`'s whole-file load, used
    /// for `.sek` seek tables).
    pub fn place(&mut self, bytes: &[u8]) -> Result<u32> {
        let at = self.heap.alloc(self.g, bytes.len() as u32, 16)?;
        if at == 0 {
            return Err(Error::new(
                0,
                "guest heap exhausted placing a resident file",
            ));
        }
        self.g.set_span(at, bytes)?;
        Ok(at)
    }

    /// `[[[BUS_ROOT]+44]]`, the default output target (the rocket grain player's bus).
    pub fn root_bus(&self) -> Result<u32> {
        let root = self.g.u32(classes::BUS_ROOT)?;
        self.g.u32(self.g.u32(root + 44)?)
    }

    /// Keep `count` more decoder blocks in the spare pool for voices a resident-stream owner can
    /// start within one tick.
    pub fn reserve_voices(&mut self, count: usize) {
        self.runtime.stream_voices += count;
    }

    /// `sub_824CE108` ([`stream::build_bus`]).
    pub fn stream_bus(&mut self, eq_chain: u32) -> Result<stream::Bus> {
        install(self.g, self.heap, self.runtime)?;
        stream::build_bus(self.g, self.heap, &mut Image, eq_chain, self.sp)
    }

    /// `sub_824CEAF0` ([`stream::start`]).
    pub fn stream_start(
        &mut self,
        bus: stream::Bus,
        stream_address: u32,
        seek_table: u32,
        start: f32,
    ) -> Result<stream::Voice> {
        stream::start(
            self.g,
            self.heap,
            &mut Image,
            bus,
            stream_address,
            seek_table,
            start,
            self.sp,
        )
    }

    /// `sub_824CEE00` ([`stream::set`]).
    pub fn stream_set(&mut self, voice: stream::Voice, gain: f32, pitch: f32) -> Result<()> {
        stream::set(self.g, voice, f64::from(gain), f64::from(pitch))
    }

    /// `sub_824CEF60` ([`stream::stop`]).
    pub fn stream_stop(&mut self, voice: stream::Voice) -> Result<()> {
        stream::stop(self.g, voice)
    }

    /// `[graph+71] == 2`.
    pub fn stream_finished(&self, voice: stream::Voice) -> Result<bool> {
        stream::finished(self.g, voice)
    }

    /// A property post to one bus module.
    pub fn stream_post(&mut self, bus: stream::Bus, index: u32, id: u32, value: f32) -> Result<()> {
        stream::post_bus(self.g, bus, index, id, value)
    }

    /// Allocate and construct a 372-byte player (`sub_828EBD88`), as the board owner's constructor
    /// does four times.
    pub fn create_player(&mut self) -> Result<u32> {
        install(self.g, self.heap, self.runtime)?;
        let at = self.heap.alloc(self.g, player::PLAYER_BYTES, 16)?;
        if at == 0 {
            return Err(Error::new(
                0,
                "guest heap exhausted allocating a grain player",
            ));
        }
        player::construct(self.g, at)?;
        self.runtime.players.push(at);
        Ok(at)
    }

    /// The surface router's grain branch for one player (`sub_824C5CA8`): bind the named member and
    /// send target (`sub_828EC040`), copy the five `GrainParams` words to `+16..+32`, pick a grain
    /// and start it in slot 0 at once. A player that is still bound must be [`Grains::stop`]ped
    /// first, as the router does on every surface change.
    pub fn bind(&mut self, player: u32, member: &str, params: [f32; 5], bus: u32) -> Result<()> {
        let data = self
            .runtime
            .source(member)
            .ok_or_else(|| Error::new(0, format!("grain {member} is not loaded")))?
            .data;
        player::bind(self.g, self.heap, player, bus, data)?;
        for (i, value) in params.iter().enumerate() {
            self.g
                .set_u32(player + player::ATTACK + 4 * i as u32, value.to_bits())?;
        }
        let start = player::pick(self.g, player, self.sp)?;
        player::start_voice(
            self.g,
            self.heap,
            &mut Image,
            player,
            player + player::SLOTS,
            start,
            self.sp,
        )
    }

    /// `sub_824C6BD8`'s per-frame store: `{gain, pitch, 0, position}` at `player+0`.
    pub fn set_record(&mut self, player: u32, record: GrainRecord) -> Result<()> {
        self.g
            .set_u32(player + player::GAIN, record.gain.to_bits())?;
        self.g
            .set_u32(player + player::PITCH, record.pitch.to_bits())?;
        self.g.set_u32(player + player::RECORD_WORD, 0)?;
        self.g
            .set_u32(player + player::POSITION, record.position.to_bits())
    }

    /// `sub_828EBF90`: stop both voices (deferred graph stops) and detach the plug-in.
    pub fn stop(&mut self, player: u32) -> Result<()> {
        player::stop(self.g, player)
    }

    pub fn view(&self, player: u32) -> Result<player::PlayerView> {
        player::view(self.g, player)
    }

    /// The default output bus module (`[0x830775EC]`), what bank voices send to.
    pub fn default_bus(&self) -> Result<u32> {
        self.g.u32(classes::DEFAULT_BUS)
    }

    /// Voice graphs currently held by all players' slots.
    pub fn live_voices(&self) -> Result<usize> {
        let mut n = 0;
        for &p in &self.runtime.players {
            for i in 0..2 {
                if self
                    .g
                    .u32(p + player::SLOTS + player::SLOT_BYTES * i + player::SLOT_GRAPH)?
                    != 0
                {
                    n += 1;
                }
            }
        }
        Ok(n)
    }

    /// The single at `player + offset`, for diagnostics.
    pub fn single(&self, at: u32) -> Result<f64> {
        load_single(self.g, at)
    }

    /// `sub_824C8878` for `player`: build its bus chain once and return the send target its voices
    /// use (graph 1's `Sub0`), to pass to [`Grains::bind`]. A player that already has a chain gets
    /// the same one back (see [`chain`]'s note on teardown).
    pub fn chain(&mut self, player: u32, config: &chain::ChainConfig) -> Result<u32> {
        install(self.g, self.heap, self.runtime)?;
        if let Some(&record) = self.runtime.chains.get(&player) {
            return chain::voice_bus(self.g, record);
        }
        let record = self.heap.alloc(self.g, chain::RECORD_BYTES, 16)?;
        if record == 0 {
            return Err(Error::new(
                0,
                "guest heap exhausted allocating a grain chain",
            ));
        }
        chain::build(self.g, self.heap, &mut Image, record, config, self.sp)?;
        self.runtime.chains.insert(player, record);
        chain::voice_bus(self.g, record)
    }

    /// The 24-byte chain record of `player` (`owner+1192+24k` in retail), once built.
    pub fn chain_record(&self, player: u32) -> Option<u32> {
        self.runtime.chains.get(&player).copied()
    }

    /// `sub_824C9058` for one truck: post `values` into the chains of its players A and B.
    pub fn push_chain(&mut self, a: u32, b: u32, values: &ChainValues) -> Result<()> {
        let record = |p: u32| {
            self.runtime
                .chains
                .get(&p)
                .copied()
                .ok_or_else(|| Error::new(p, "grain player has no chain"))
        };
        let (ra, rb) = (record(a)?, record(b)?);
        chain::push(self.g, ra, rb, values)
    }
}

/// The header length word of a grain member, validated against the member.
fn skate_grain_header(bytes: &[u8]) -> Result<u32> {
    let h = bytes
        .get(0..4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| Error::new(0, "grain member is shorter than its header"))?;
    if h < 16 || h as usize + 8 > bytes.len() {
        return Err(Error::new(
            0,
            format!("grain header length {h} is out of range"),
        ));
    }
    Ok(h)
}
