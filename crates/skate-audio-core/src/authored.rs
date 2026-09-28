//! Concrete single-thread owner of the recovered rider/board audio graph.
//!
//! The host supplies the unmodified guest constants, CSI projects, ABK banks and decoded PCM.
//! Every block runs the bank evaluator, drains its graph commands, feeds SndPlayer1's segment
//! ring, runs the installed voice and bus graphs, and submits the final six-channel Dac buffer.
//! File access and codec decoding never occur in this owner.

mod mixmap_host;
pub mod oneshot;

use std::collections::HashMap;

use crate::eval::interp;
use crate::patch::{self, Heap};
use crate::pcm::{CachedPcm, PcmStreams};
use crate::voice::{OpenRequest, VoiceDevice};
use crate::{
    Error, Guest, Result, classes, commands, device, graph, kernels, mathlib, modules, output,
    play, pump, scheduler, stream, symbols, voices, worker,
};

pub const PCM_CHANNELS: u16 = 6;
pub const PCM_SAMPLE_RATE: u32 = 48_000;
pub const PCM_FRAMES_PER_BLOCK: u32 = 256;

const SPACE: u32 = 0x4000_0000;
const SYSTEM: u32 = SPACE;
const ROOT: u32 = SPACE + 0x1000;
const MANAGER: u32 = SPACE + 0x2000;
const DEVICE: u32 = SPACE + 0x3000;
const PASS: u32 = SPACE + 0x4000;
const PARAMS: u32 = SPACE + 0x4100;
const WORKER: u32 = SPACE + 0x4200;
const OUTPUT_HOLDER: u32 = SPACE + 0x4300;
const OUTPUT_DESC: u32 = SPACE + 0x4400;
const SCRATCH: u32 = SPACE + 0x5000;
const RING: u32 = SPACE + 0x10000;
const PLANE_A: u32 = SPACE + 0x20000;
const PLANE_B: u32 = SPACE + 0x22000;
const PLANE_C: u32 = SPACE + 0x24000;
const PLANES_OUT: u32 = SPACE + 0x26000;
const PCM: u32 = SPACE + 0x28000;
const STACK: u32 = SPACE + 0x3F000;
const ARENA: u32 = 0x5000_0000;
const PATCH_HEAP: u32 = 0x6000_0000;
const GRAPH_HEAP: u32 = 0x6400_0000;
const HEAP_BYTES: u32 = 0x0400_0000;

/// Host allocator with reusable guest allocations, owned by the audio thread.
#[derive(Default)]
struct ArenaHeap {
    next: u32,
    end: u32,
    live: HashMap<u32, u32>,
    free: Vec<(u32, u32)>,
}

impl ArenaHeap {
    fn new(base: u32) -> Self {
        Self {
            next: base,
            end: base + HEAP_BYTES,
            ..Self::default()
        }
    }
}

impl Heap for ArenaHeap {
    fn alloc(&mut self, g: &mut Guest, bytes: u32, align: u32) -> Result<u32> {
        let align = align.max(16);
        let bytes = (bytes.max(16) + 15) & !15;
        let found = self
            .free
            .iter()
            .position(|(at, size)| *size >= bytes && at % align == 0);
        let at = if let Some(index) = found {
            let (at, size) = self.free.swap_remove(index);
            if size > bytes {
                self.free.push((at + bytes, size - bytes));
            }
            at
        } else {
            let at = (self.next + align - 1) & !(align - 1);
            if at.checked_add(bytes).is_none_or(|end| end > self.end) {
                return Err(Error::new(at, "authored audio guest heap exhausted"));
            }
            self.next = at + bytes;
            at
        };
        g.fill(at, 0, bytes)?;
        self.live.insert(at, bytes);
        Ok(at)
    }

    fn free(&mut self, _g: &mut Guest, at: u32) -> Result<()> {
        if let Some(size) = self.live.remove(&at) {
            self.free.push((at, size));
        }
        Ok(())
    }
}

/// Small diagnostic counters; no output device or filesystem is retained by this object.
#[derive(Clone, Copy, Debug, Default)]
pub struct RuntimeStats {
    pub blocks: u64,
    pub voices_opened: u64,
    pub live_voices: usize,
    pub commands: u64,
    pub peak: f32,
}

pub struct AuthoredRuntime {
    guest: Guest,
    owner: Owner,
    banks: HashMap<String, u32>,
    objects: HashMap<String, u32>,
    messages: HashMap<u32, u32>,
}

struct Owner {
    patch_heap: ArenaHeap,
    device: AuthoredDevice,
    submitted: Vec<f32>,
    stats: RuntimeStats,
}

struct InspectKernels<'a> {
    inner: kernels::VoiceKernels<'a, mathlib::Image>,
    trace: bool,
}
impl graph::GraphHost for InspectKernels<'_> {
    fn prepare(&mut self, g: &mut Guest, f: u32, o: u32, p: u32, flag: u32, n: u64) -> Result<u64> {
        graph::GraphHost::prepare(&mut self.inner, g, f, o, p, flag, n)
    }
    fn process(&mut self, g: &mut Guest, f: u32, o: u32, p: u32, flag: u32) -> Result<u64> {
        let result = graph::GraphHost::process(&mut self.inner, g, f, o, p, flag)?;
        if self.trace {
            let d = g.u32(p + 28)?;
            let data = g.u32(d + 4)?;
            let peak = (0..256)
                .map(|i| g.f32(data + i * 4).unwrap_or(0.0).abs())
                .fold(0.0f32, f32::max);
            eprintln!(
                "kernel {f:08x} object={o:08x} result={result} frames={} channels={} peak={peak}",
                g.u32(p + 48)?,
                g.u8(p + 60)?
            );
        }
        Ok(result)
    }
}

struct SourceState {
    decoder: u32,
    loop_range: Option<(usize, usize)>,
}

struct AuthoredDevice {
    heap: ArenaHeap,
    sources: HashMap<u32, CachedPcm>,
    streams: PcmStreams,
    players: HashMap<u32, SourceState>, // SndPlayer1 -> decoder state
    live: HashMap<u32, u32>,            // voice -> player
    opened: u64,
    /// Voices whose player has published an active span; see `query`.
    sounded: std::collections::HashSet<u32>,
    /// Grain-player hook (`grain::host`): grain files, pending grain plays and the plug-in clock.
    grains: crate::grain::host::GrainRuntime,
}

impl AuthoredRuntime {
    pub fn new(
        mut guest: Guest,
        projects: Vec<Vec<u8>>,
        banks: Vec<(String, Vec<u8>)>,
    ) -> Result<Self> {
        guest.put(SPACE, vec![0; 0x40000]);
        guest.put(PATCH_HEAP, vec![0; HEAP_BYTES as usize]);
        guest.put(GRAPH_HEAP, vec![0; HEAP_BYTES as usize]);
        for cell in [
            symbols::PROJECT_LIST_HEAD,
            interp::LIST_HEAD,
            patch::BANK_LIST,
            interp::DELTA_CACHE,
            interp::FRAME_COUNT,
            interp::COUNTDOWN,
            interp::SCALE_GLOBAL,
            modules::SUBMIX_HEAD,
        ] {
            guest.set_u32(cell, 0)?;
        }
        guest.set_u16(symbols::GENERATION, 0)?;
        guest.set_u32(interp::PERIOD_DENOM, 30.0f32.to_bits())?;
        guest.set_u8(classes::REGISTERED, 0)?;
        let mut next = ARENA;
        let mut place = |g: &mut Guest, bytes: Vec<u8>| -> Result<u32> {
            let at = next;
            next = at
                .checked_add(bytes.len() as u32 + 0xFFF)
                .map(|end| end & !0xFFF)
                .ok_or_else(|| Error::new(at, "bank address overflow"))?;
            if next >= PATCH_HEAP {
                return Err(Error::new(at, "banks exceed guest arena"));
            }
            g.put(at, bytes);
            Ok(at)
        };
        let mut objects = HashMap::new();
        let mut patch_heap = ArenaHeap::new(PATCH_HEAP);
        for bytes in projects {
            let at = place(&mut guest, bytes)?;
            symbols::install_project(&mut guest, at)?;
            let table = guest.u32(at + 24)?;
            for i in 0..u32::from(guest.u16(at + 12)?) {
                let record = table + 12 * i;
                let name_at = guest.u32(record + 4)?;
                let mut name = Vec::new();
                while guest.u8(name_at + name.len() as u32)? != 0 {
                    name.push(guest.u8(name_at + name.len() as u32)?);
                }
                let slot = patch_heap.alloc(&mut guest, 8, 4)?;
                guest.set_u32(slot, record)?;
                guest.set_u32(slot + 4, guest.u32(record + 8)?)?;
                objects.insert(String::from_utf8_lossy(&name).into_owned(), slot);
            }
        }
        let mut installed = HashMap::new();
        for (name, bytes) in banks {
            // `SPLC` sound banks (the one-shot voices' banks) are not patch banks: the retail
            // loader takes its other branch for them (`sub_828DC660`'s `[resource+4] == 2`, which
            // registers their sound table instead of installing patches), so `load_bank`'s `.abk`
            // fixups must not run over one. They are placed as they are and addressed by
            // `bank_base` + the sample's offset.
            let splice = bytes.get(..4) == Some(b"SPLC");
            let at = place(&mut guest, bytes)?;
            if !splice {
                patch::load_bank(&mut guest, at, STACK)?;
            }
            installed.insert(name, at);
        }
        let mut device = AuthoredDevice {
            heap: ArenaHeap::new(GRAPH_HEAP),
            sources: HashMap::new(),
            streams: PcmStreams::default(),
            players: HashMap::new(),
            live: HashMap::new(),
            opened: 0,
            sounded: Default::default(),
            grains: crate::grain::host::GrainRuntime::default(),
        };
        device.initialize(&mut guest)?;
        Ok(Self {
            guest,
            owner: Owner {
                patch_heap,
                device,
                submitted: Vec::new(),
                stats: RuntimeStats::default(),
            },
            banks: installed,
            objects,
            messages: HashMap::new(),
        })
    }

    /// The graph arena's high-water mark and how many blocks are currently live. A long session
    /// must see `live` plateau: it climbing without bound is the voice-graph leak that once ended
    /// in "authored audio guest heap exhausted".
    pub fn arena_usage(&self) -> (u32, usize) {
        (
            self.owner.device.heap.next - GRAPH_HEAP,
            self.owner.device.heap.live.len(),
        )
    }

    /// Any sample key with decoded PCM installed. For tests that need a voice to open and do not
    /// care which sound it is.
    pub fn any_installed_sample(&self) -> Option<u32> {
        self.owner.device.sources.keys().min().copied()
    }

    pub fn bank_base(&self, name: &str) -> Option<u32> {
        self.banks.get(name).copied()
    }

    pub fn insert_pcm(&mut self, sample_key: u32, pcm: CachedPcm) -> Result<()> {
        if pcm.sample_rate == 0 || pcm.source.frames() == 0 {
            return Err(Error::new(sample_key, "empty decoded PCM source"));
        }
        self.owner.device.sources.insert(sample_key, pcm);
        Ok(())
    }

    pub fn has_object(&self, object: &str) -> bool {
        self.objects.contains_key(object)
    }

    /// Post the exact recovered payload. The returned handle can be redelivered or released.
    pub fn post(&mut self, object: &str, payload: &[u32]) -> Result<u32> {
        self.post_relocated(object, payload, &[])
    }

    /// Post a recovered message whose native packet includes pointers into that packet.
    ///
    /// Trace payloads are process-relative: fields which pointed into the caller's packet in the
    /// retail process cannot be copied verbatim into this guest. `relocations` gives each field's
    /// word index and byte offset from the payload base, preserving the same self-reference after
    /// the packet has been allocated here.
    pub fn post_relocated(
        &mut self,
        object: &str,
        payload: &[u32],
        relocations: &[(usize, u32)],
    ) -> Result<u32> {
        if payload.len() > 128 {
            return Err(Error::new(0, "audio payload exceeds 128 words"));
        }
        if relocations.iter().any(|(index, _)| *index >= payload.len()) {
            return Err(Error::new(
                0,
                "audio payload relocation is outside the packet",
            ));
        }
        let slot = *self
            .objects
            .get(object)
            .ok_or_else(|| Error::new(0, format!("unknown audio object {object}")))?;
        let message = self.owner.patch_heap.alloc(&mut self.guest, 516, 16)?;
        for (i, word) in payload.iter().enumerate() {
            self.guest.set_u32(message + 4 + i as u32 * 4, *word)?;
        }
        for &(index, offset) in relocations {
            self.guest
                .set_u32(message + 4 + index as u32 * 4, message + 4 + offset)?;
        }
        let status = patch::post(
            &mut self.guest,
            &mut self.owner.patch_heap,
            slot,
            message + 4,
            message,
        )?;
        if status != 0 {
            return Err(Error::new(slot, format!("post {object} returned {status}")));
        }
        let node = self.guest.u32(message)?;
        self.messages.insert(node, message);
        Ok(node)
    }

    pub fn redeliver(&mut self, handle: u32, payload: &[u32]) -> Result<()> {
        self.redeliver_relocated(handle, payload, &[])
    }

    /// Re-deliver a recovered packet with payload-relative pointer fields relocated into the
    /// already allocated guest message.
    pub fn redeliver_relocated(
        &mut self,
        handle: u32,
        payload: &[u32],
        relocations: &[(usize, u32)],
    ) -> Result<()> {
        let Some(&message) = self.messages.get(&handle) else {
            return Ok(());
        };
        if payload.len() > 128 {
            return Err(Error::new(handle, "audio payload exceeds 128 words"));
        }
        if relocations.iter().any(|(index, _)| *index >= payload.len()) {
            return Err(Error::new(
                handle,
                "audio payload relocation is outside the packet",
            ));
        }
        for (i, word) in payload.iter().enumerate() {
            self.guest.set_u32(message + 4 + i as u32 * 4, *word)?;
        }
        for &(index, offset) in relocations {
            self.guest
                .set_u32(message + 4 + index as u32 * 4, message + 4 + offset)?;
        }
        patch::redeliver(&mut self.guest, handle, message + 4)?;
        Ok(())
    }

    pub fn release(&mut self, handle: u32) -> Result<()> {
        if let Some(message) = self.messages.remove(&handle) {
            patch::release_message(&mut self.guest, &mut self.owner.patch_heap, handle)?;
            self.owner.patch_heap.free(&mut self.guest, message)?;
        }
        Ok(())
    }

    pub fn stats(&self) -> RuntimeStats {
        self.owner.stats
    }

    /// Grain-player hook: the game-side grain API (`grain::host::Grains`) over this owner's guest,
    /// device heap and decoded sources.
    pub fn grains(&mut self) -> crate::grain::host::Grains<'_> {
        let device = &mut self.owner.device;
        crate::grain::host::Grains {
            g: &mut self.guest,
            heap: &mut device.heap,
            sources: &mut device.sources,
            runtime: &mut device.grains,
            sp: STACK,
        }
    }

    /// Grain-player hook: voice starts the grain players have made so far (drained).
    pub fn take_grain_starts(&mut self) -> Vec<crate::grain::host::StartEvent> {
        std::mem::take(&mut self.owner.device.grains.starts)
    }

    pub fn pump_once(&mut self) -> Result<Vec<f32>> {
        self.owner.submitted.clear();
        worker::pump_once(&mut self.guest, &mut self.owner, WORKER)?;
        Ok(std::mem::take(&mut self.owner.submitted))
    }
}

impl AuthoredDevice {
    fn initialize(&mut self, g: &mut Guest) -> Result<()> {
        g.set_u32(modules::SYSTEM, SYSTEM)?;
        g.set_u32(SYSTEM, ROOT)?;
        g.set_u32(SYSTEM + 48, RING)?;
        g.set_u32(SYSTEM + 276, 64)?;
        g.set_u32(SYSTEM + 252, 6)?;
        g.set_u32(ROOT + 32, STACK - 0x2000)?;
        g.set_u32(ROOT + 8, SYSTEM)?;
        g.set_u32(ROOT + 44, ROOT + 64)?;
        g.set_u32(classes::BUS_ROOT, ROOT)?;
        g.set_u32(device::BUS_MANAGER, MANAGER)?;
        // The intermediate send is disabled by the title's null-target branch until an authored
        // environmental bus is supplied. The dry output and eight material buses remain real.
        g.set_u32(MANAGER + 52, ROOT + 68)?;
        let registry = classes::class_registry(g, &mut self.heap, SYSTEM)?;
        g.set_u32(ROOT + 12, registry)?;
        let submix = classes::register_class(g, registry, device::SUBMIX_DESCRIPTOR)?;
        let delay = classes::register_class(g, registry, device::DELAY_DESCRIPTOR)?;
        classes::register_class(g, registry, device::SEND_DESCRIPTOR)?;
        classes::register_voice_classes(g, &mut self.heap)?;
        // The final Sub0 and measured 15 ms Del0 precede the recovered output-pass route/clamp.
        for (i, class) in [submix, delay].into_iter().enumerate() {
            let at = SCRATCH + i as u32 * 12;
            g.set_u32(at, if i == 1 { SCRATCH + 32 } else { 0 })?;
            g.set_u32(at + 4, class)?;
            g.set_u8(at + 8, 6)?;
        }
        g.set_u32(SCRATCH + 32, classes::TAG_SINGLE)?;
        g.set_u32(SCRATCH + 36, g.u32(device::OUTPUT_DELAY_SECONDS)?)?;
        let master = modules::build_graph(
            g,
            &mut self.heap,
            &mut mathlib::Image,
            SYSTEM,
            255,
            2,
            SCRATCH,
        )?;
        let bus = g.u32(master + 80)?;
        g.set_u32(ROOT + 64, bus)?;
        g.set_u32(classes::DEFAULT_BUS, bus)?;
        device::init_bus_players(g, &mut self.heap, &mut mathlib::Image, MANAGER, SCRATCH)?;
        // Material buses retain the actual class default parameter blocks installed above.
        for index in 0..device::BUS_GRAPH_COUNT {
            g.set_u8(MANAGER + device::BUS_CREATED_FLAGS + index, 1)?;
        }
        device::init_output_players(g, &mut self.heap, &mut mathlib::Image, MANAGER, SCRATCH)?;
        g.set_u32(PASS, PLANE_A)?;
        g.set_u32(PASS + 4, PLANE_B)?;
        g.set_u32(PASS + 8, PLANE_C)?;
        g.set_u32(PASS + 24, SYSTEM)?;
        g.set_u32(OUTPUT_DESC + 4, PLANES_OUT)?;
        g.set_u16(OUTPUT_DESC + 14, 256)?;
        g.set_u32(OUTPUT_HOLDER + 32, OUTPUT_DESC)?;
        g.set_u32(WORKER + output::DESC_HOLDER, OUTPUT_HOLDER)?;
        g.set_u32(WORKER + worker::PCM_BASE, PCM)?;
        g.set_u8(output::CHANNEL_COUNT_BYTE, 6)?;
        g.set_u8(output::RAMP_FLAG_BYTE, 0)?;
        Ok(())
    }

    fn source_pump(&mut self, g: &mut Guest, stream: u32) -> Result<()> {
        // The pump's two image doubles, as `sub_82B31EE0` loads them (`lis -32208` − 31232, +256
        // and +4192). The idle time is 0.0, not a far-future marker: with an invented 1e30 a
        // finished one-shot reported an elapsed time no patch ever sees in retail, so the skid's
        // second layer never re-triggered and a revert kept only its quiet first voice.
        let constants = pump::PumpConstants {
            far_future: f64::from_bits(g.u64(0x822F_8700)?),
            time_bump: f64::from_bits(g.u64(0x822F_9660)?),
        };
        pump::pump(g, self, stream, constants)
    }

    /// `heap` is passed in rather than taken from `self`, because the command drain moves the
    /// device's arena out for the duration of the drain — freeing through `self.heap` there frees
    /// into a temporary and silently leaks.
    fn stop_graph(&mut self, g: &mut Guest, heap: &mut dyn Heap, player: u32) -> Result<()> {
        for i in 0..u32::from(g.u8(player + 68)?) {
            let child = g.u32(player + 80 + 4 * i)?;
            if g.u32(child)? == modules::SEND_VTABLE {
                voices::release_voice(g, u64::from(child), STACK)?;
            }
            if let Some(source) = self.players.remove(&child) {
                self.streams.detach(source.decoder);
                heap.free(g, source.decoder)?;
                scheduler::detach_instance(
                    g,
                    u64::from(SYSTEM + scheduler::SYSTEM_SCHEDULER),
                    child + 72,
                )?;
                self.heap.free(g, g.u32(child + 440)?)?;
            }
        }
        voices::remove_handle(g, player)?;
        g.set_u8(player + 71, 2)?;
        self.heap.free(g, player)?;
        Ok(())
    }

    /// The recoverable half of `sub_82B1E458`: append the exact eight-byte deferred player-stop
    /// record before destroying the small voice wrapper. The player, its SndPlayer1 child and the
    /// host decoder must remain alive until command phase reaches that record; a patch is allowed
    /// to open and release a voice in the same evaluator walk, after its play command was queued.
    fn defer_stop_graph(&mut self, g: &mut Guest, player: u32) -> Result<()> {
        let system = g.u32(player + 16)?;
        let offset = g.u32(system + commands::COMMAND_WRITE_OFFSET)?;
        let ring = g.u32(system + commands::COMMAND_BUFFER)?;
        let record = ring.wrapping_add(offset);
        // Preserve the producer's observed publication order.
        g.set_u32(
            system + commands::COMMAND_WRITE_OFFSET,
            offset.wrapping_add(8),
        )?;
        g.set_u32(record, device::COMMAND_PLAYER_STOP)?;
        g.set_u32(record + 4, player)?;
        Ok(())
    }
}

impl VoiceDevice for AuthoredDevice {
    fn open(&mut self, g: &mut Guest, request: &OpenRequest) -> Result<u32> {
        let sample = request.sample as u32;
        let cached = self.sources.get(&sample).cloned().ok_or_else(|| {
            Error::new(
                sample,
                format!("authored sample {} has no decoded PCM", request.index),
            )
        })?;
        for (i, word) in request.shifted.iter().enumerate() {
            g.set_u32(SCRATCH + i as u32 * 4, *word)?;
        }
        g.set_u32(SCRATCH + 24, request.record_count)?;
        g.set_u32(SCRATCH + 28, request.records)?;
        let voice = device::open_voice_graph(
            g,
            &mut self.heap,
            &mut mathlib::Image,
            &mut device::NoBuses,
            DEVICE,
            sample,
            u32::from(request.byte2),
            SCRATCH,
            request.bank_72,
            request.arg8 as u32,
            SCRATCH + 24,
            STACK,
        )?;
        let stream = g.u32(voice + 8)?;
        let decoder = self.heap.alloc(g, 128, 16)?;
        self.players.insert(
            stream,
            SourceState {
                decoder,
                loop_range: cached.source.loop_range(),
            },
        );
        self.live.insert(voice, g.u32(voice + 4)?);
        self.opened += 1;
        self.sounded.remove(&voice);
        crate::voice::report_open(g, request, voice)?;
        if std::env::var_os("SKATE_AUDIO_VOICE_TRACE").is_some() {
            eprintln!(
                "SKATE_AUDIO_VOICE open voice={voice:08x} sample={sample:08x} live={}",
                self.live.len()
            );
        }
        Ok(voice)
    }

    fn release(&mut self, g: &mut Guest, voice: u32) -> Result<()> {
        if let Some(player) = self.live.remove(&voice) {
            crate::voice::report_release(voice);
            if std::env::var_os("SKATE_AUDIO_VOICE_TRACE").is_some() {
                eprintln!(
                    "SKATE_AUDIO_VOICE release voice={voice:08x} live={}",
                    self.live.len()
                );
            }
            self.defer_stop_graph(g, player)?;
            self.heap.free(g, voice)?;
        }
        Ok(())
    }
    fn suspend(&mut self, g: &mut Guest, voice: u32) -> Result<()> {
        g.set_u8(g.u32(voice + 4)? + 72, 3)
    }
    fn resume(&mut self, g: &mut Guest, voice: u32) -> Result<()> {
        g.set_u8(g.u32(voice + 4)? + 72, 1)
    }
    fn set(&mut self, g: &mut Guest, voice: u32, id: u32, value: u32) -> Result<()> {
        if std::env::var_os("SKATE_AUDIO_KERNEL_TRACE").is_some() {
            eprintln!("set {voice:08x} id={id} value={value}");
        }
        let value = f64::from(value as i32);
        match id {
            0 => device::post_property(g, g.u32(voice + 12)?, 0, value / 4096.0)?,
            2 | 5 | 8 => {
                let offset = match id {
                    2 => 40,
                    5 => 48,
                    _ => 44,
                };
                // Store single exactly as the setter's stfs, then multiply its lfs values.
                g.set_u32(voice + offset, ((value / 32767.0) as f32).to_bits())?;
                let master = f64::from(g.f32(voice + 40)?);
                let send = g.u32(voice + 24)?;
                if send != 0 {
                    device::post_property(g, send, 0, master * f64::from(g.f32(voice + 48)?))?;
                }
                device::post_property(
                    g,
                    g.u32(voice + 28)?,
                    0,
                    master * f64::from(g.f32(voice + 44)?),
                )?;
            }
            6 => device::post_property(g, g.u32(voice + 20)?, 0, value)?,
            7 => device::post_property(g, g.u32(voice + 16)?, 0, value)?,
            11 if g.u32(voice + 88)? != 0 => {
                device::post_property(g, g.u32(voice + 84)?, 0, value / 32767.0)?
            }
            _ => {}
        }
        Ok(())
    }
    fn set_alternate(&mut self, g: &mut Guest, voice: u32, value: u32) -> Result<()> {
        device::post_property(
            g,
            g.u32(voice + 32)?,
            0,
            f64::from(value as i32) * (360.0 / 65536.0),
        )
    }
    fn query(&mut self, g: &mut Guest, voice: u32, out: &mut [u32; 11]) -> Result<()> {
        out.fill(0);
        if g.u8(g.u32(voice + 4)? + 71)? == 2 {
            return Ok(());
        }
        out[0] = 1;
        let stream = g.u32(voice + 8)?;
        // A voice whose player has played and then gone idle has ended.
        //
        // **Approximation, deliberate.** Retail reports this through the player's state byte: a
        // finished player is retired by `sub_82B49100`, which stores 2 into `+71`, and the test
        // above is retail's own (`sub_824A2DD8`). Nothing retires a player here, because the
        // per-block `rw_system` driver (`sub_82B48530`, ported as `runtime::run_frame`) is not
        // driven by this runtime and its object list at `SYSTEM+20` stays empty, so a one-shot
        // that finished still read as live for ever. A patch that re-arms a layer when its voice
        // ends -- the skid's second layer -- then never fired again for the life of the message:
        // a powerslide kept one quiet voice instead of retriggering, and the revert that followed
        // stayed ~10 dB below retail's. Replacing this with the real retirement path is the fix.
        // `SKATE_AUDIO_NO_VOICE_END=1` restores the old behaviour, for an A/B by ear.
        let idle = g.u64(stream + pump::END_TIME)? == 0
            && std::env::var_os("SKATE_AUDIO_NO_VOICE_END").is_none();
        if idle && self.sounded.contains(&voice) {
            out[0] = 0;
            return Ok(());
        }
        if !idle {
            self.sounded.insert(voice);
        }
        let status = g.u32(stream + 16)?;
        let duration = f64::from_bits(g.u64(voice + 56)?);
        let unit = f64::from_bits(g.u64(0x822F_8890)?);
        let elapsed = if g.f32(status + 4)? < g.f32(voice + 52)? {
            0.0
        } else {
            f64::from_bits(g.u64(status + 8)?)
        };
        out[1] = crate::fp::fctiwz_low_word((duration - elapsed) * unit);
        out[2] = crate::fp::fctiwz_low_word(elapsed * unit);
        Ok(())
    }
    fn poke(&mut self, _g: &mut Guest, _voice: u32) -> Result<()> {
        Ok(())
    }
}

impl play::PlayHost for AuthoredDevice {
    fn prepare(&mut self, g: &mut Guest, stream: u32, index: u8, sample: u32) -> Result<bool> {
        let cached = self
            .sources
            .get(&sample)
            .ok_or_else(|| Error::new(sample, "play has no cached PCM"))?
            .clone();
        let source = self
            .players
            .get(&stream)
            .ok_or_else(|| Error::new(stream, "play has no decoder owner"))?;
        let decoder = source.decoder;
        let slot = play::slot_address(g, stream, index)?;
        let record = play::record_address(g, stream, index)?;
        let frames = cached
            .source
            .loop_range()
            .map_or(cached.source.frames(), |(_, end)| end) as u32;
        g.set_u32(slot + 8, decoder)?;
        g.set_u32(
            slot + play::SLOT_RATE,
            (cached.sample_rate as f32).to_bits(),
        )?;
        g.set_u32(slot + play::SLOT_QUEUED, frames)?;
        g.set_u32(
            slot + play::SLOT_CURSOR,
            cached
                .source
                .loop_range()
                .map_or(u32::MAX, |(start, _)| start as u32),
        )?;
        g.set_u8(slot + play::SLOT_MODE, cached.source.channels())?;
        g.set_u8(record + play::RECORD_KIND, 0)?;
        g.set_u32(decoder + 36, 64)?;
        g.set_u32(decoder + 64 + 12, frames)?;
        g.set_u8(decoder + 46, cached.source.channels())?;
        g.set_u8(decoder + 50, 1)?;
        g.set_u32(stream + 436, frames)?;
        g.set_u32(stream + 424, g.u32(slot + 12)?)?;
        g.set_u32(stream + 428, (cached.sample_rate as f32).to_bits())?;
        self.streams.attach(decoder, cached.source);
        Ok(true)
    }
    fn decode(&mut self, g: &mut Guest, stream: u32, index: u8, detail: u32) -> Result<bool> {
        // Grain-player hook: a grain play starts at its retail seek frame (`grain::host`).
        if let Some(decoder) = self.players.get(&stream).map(|s| s.decoder) {
            self.grains.seek(
                g,
                &mut self.streams,
                stream,
                index,
                detail,
                decoder,
                STACK - 0x1000,
            )?;
        }
        Ok(true)
    }
}

impl commands::CommandHost for AuthoredDevice {
    fn play(&mut self, g: &mut Guest, record: u32) -> Result<u32> {
        // Grain-player hook: grain graphs are built by the player, not opened through the voice
        // device, so their SndPlayer1 gets its decoder here, as `open` gives bank voices theirs.
        if let Some(stream) = self.grains.begin_play(g, record)? {
            if !self.players.contains_key(&stream) {
                let decoder = self
                    .grains
                    .take_decoder()
                    .ok_or_else(|| Error::new(stream, "no spare decoder for a grain voice"))?;
                self.players.insert(
                    stream,
                    SourceState {
                        decoder,
                        loop_range: None,
                    },
                );
            }
        }
        let result = play::append_resident(g, self, record);
        self.grains.end_play();
        result
    }
    fn stop_player(&mut self, g: &mut Guest, heap: &mut dyn Heap, record: u32) -> Result<u32> {
        let player = g.u32(record + 4)?;
        self.stop_graph(g, heap, player)?;
        // `build_graph` takes the whole graph — header, module table and every module — out of the
        // arena in one allocation, and this deferred teardown is the point at which nothing
        // references it any more. Without this the block leaked: `release` freed only the voice,
        // so every one-shot cost a graph for the rest of the session and a long session ended in
        // "authored audio guest heap exhausted".
        heap.free(g, player)?;
        Ok(8)
    }
}

impl pump::PumpHost for AuthoredDevice {
    fn preflight(&mut self, g: &mut Guest, stream: u32) -> Result<()> {
        let Some(source) = self.players.get(&stream) else {
            return Ok(());
        };
        if let Some((start, end)) = source.loop_range {
            if g.u32(source.decoder + 64 + 12)? == 0 {
                g.set_u32(source.decoder + 28, start as u32)?;
                g.set_u32(source.decoder + 64 + 8, start as u32)?;
                g.set_u32(source.decoder + 64 + 12, end as u32)?;
            }
        }
        for index in 0..20 {
            if g.u8(stream + index * 16 + 113)? == 2 {
                g.set_u8(stream + index * 16 + 113, 0)?;
            }
        }
        Ok(())
    }
    fn retire(&mut self, g: &mut Guest, stream: u32, index: u8) -> Result<()> {
        g.set_u8(play::slot_address(g, stream, index)? + play::SLOT_STATE, 0)
    }
    fn prepare(
        &mut self,
        _g: &mut Guest,
        _stream: u32,
        _index: u8,
        _counter: &mut u32,
    ) -> Result<bool> {
        Ok(true)
    }
    fn at_end(&mut self, g: &mut Guest, stream: u32, index: u8, counter: &mut u32) -> Result<bool> {
        self.submit(g, stream, index, counter)
    }
    fn at_work(
        &mut self,
        _g: &mut Guest,
        _stream: u32,
        _index: u8,
        _counter: &mut u32,
    ) -> Result<(bool, bool)> {
        Ok((false, false))
    }
    fn submit(
        &mut self,
        g: &mut Guest,
        stream: u32,
        _index: u8,
        _counter: &mut u32,
    ) -> Result<bool> {
        let consumer = u32::from(g.u8(stream + 474)?);
        let at = stream + consumer * 16;
        if g.u8(at + 113)? != 1 {
            g.set_u8(at + 112, 0)?;
            g.set_u32(at + 108, 0)?;
            g.set_u8(at + 113, 1)?;
        }
        Ok(false) // all resident frames are ready; one publication completes this pump
    }
}

impl stream::StreamFill for AuthoredDevice {
    fn fill(&mut self, g: &mut Guest, stream: u32, descriptor: u32, frames: u64) -> Result<u64> {
        stream::StreamFill::fill(&mut self.streams, g, stream, descriptor, frames)
    }
}

impl worker::WorkerHost for Owner {
    fn render(&mut self, g: &mut Guest, _worker: u32) -> Result<bool> {
        interp::tick_with(
            g,
            (256.0f32 / 48000.0) as f64,
            &mut patch::PatchHost {
                heap: &mut self.patch_heap,
                device: &mut self.device,
            },
        )?;
        // Grain-player hook: retail phase 1 ticks scheduler bucket 0 (the grain plug-ins) before
        // the command drain (`grain::host::tick_bucket_zero`).
        crate::grain::host::tick_bucket_zero(
            g,
            &mut self.device.heap,
            &mut self.device.grains,
            STACK,
        )?;
        let mut heap = std::mem::take(&mut self.device.heap);
        let drained = commands::drain(g, &mut heap, &mut self.device, SYSTEM, STACK);
        self.device.heap = heap;
        self.stats.commands += u64::from(drained?.records);
        for stream in self.device.players.keys().copied().collect::<Vec<_>>() {
            self.device.source_pump(g, stream)?;
        }
        g.set_u64(
            PARAMS,
            (self.stats.blocks as f64 * 256.0 / 48000.0).to_bits(),
        )?;
        g.set_u32(PARAMS + 8, g.u32(SYSTEM + 108)?)?;
        g.set_u32(PARAMS + 12, 48000.0f32.to_bits())?;
        g.set_u16(PARAMS + 20, g.u16(SYSTEM + 280)?)?;
        let trace =
            std::env::var_os("SKATE_AUDIO_KERNEL_TRACE").is_some() && self.stats.blocks == 60;
        if trace {
            eprintln!(
                "nodes={} sources={}",
                g.u16(PARAMS + 20)?,
                self.device.players.len()
            );
            for (&s, state) in &self.device.players {
                let slot = play::slot_address(g, s, g.u8(s + 469)?)?;
                eprintln!(
                    "source {s:08x} slot {slot:08x} words={:?} state={} ready={} decoder={:08x} remaining={}",
                    (0..12)
                        .map(|i| g.u32(slot + i * 4).unwrap())
                        .collect::<Vec<_>>(),
                    g.u8(slot + 46)?,
                    g.u8(s + 113)?,
                    state.decoder,
                    crate::leaves::stream_remaining(g, state.decoder, 0)?
                );
            }
        }
        graph::run_pass(
            g,
            &mut InspectKernels {
                inner: kernels::VoiceKernels {
                    trig: &mut mathlib::Image,
                    fill: &mut self.device,
                    sp: STACK,
                },
                trace,
            },
            PASS,
            PARAMS,
        )?;
        g.set_u32(OUTPUT_HOLDER + 28, g.u32(PASS + 28)?)?;
        output::output_pass(g, WORKER, STACK)?;
        self.stats.blocks += 1;
        self.stats.voices_opened = self.device.opened;
        self.stats.live_voices = self.device.live.len();
        Ok(true)
    }
    fn queued_buffers(&mut self, _g: &mut Guest, _worker: u32) -> Result<u32> {
        Ok(0)
    }
    fn submit(&mut self, g: &mut Guest, worker: u32, descriptor: u32) -> Result<()> {
        let index = (descriptor - worker - worker::DESCRIPTORS) / worker::DESCRIPTOR_BYTES;
        let pcm = g.u32(worker + worker::PCM_BASE)? + index * worker::BUFFER_BYTES;
        for sample in 0..worker::BUFFER_BYTES / 4 {
            let value = g.f32(pcm + sample * 4)?;
            if !value.is_finite() {
                return Err(Error::new(
                    pcm + sample * 4,
                    "authored graph produced nonfinite PCM",
                ));
            }
            self.stats.peak = self.stats.peak.max(value.abs());
            self.submitted.push(value);
        }
        g.set_u32(worker + (index + worker::SLOT_BASE) * 4, 0)
    }
}
