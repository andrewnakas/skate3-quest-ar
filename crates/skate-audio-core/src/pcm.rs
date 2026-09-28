//! Decoded PCM backed [`crate::stream::StreamFill`].
//!
//! Archive/XMA decoding belongs outside the audio worker. This module receives immutable,
//! interleaved `i16` frames and writes the guest's planar `f32` descriptor layout on demand. A
//! cursor is owned per guest stream address, so two live voices can read the same cached sample
//! without sharing progress.

use std::collections::HashMap;
use std::sync::Arc;

use crate::commands::CommandHost;
use crate::patch::Heap;
use crate::play::{self, PlayHost};
use crate::player::{self, GraphReleaseHost};
use crate::stream::StreamFill;
use crate::{Error, Guest, Result};

/// An immutable interleaved PCM sample. `samples` contains complete frames only.
#[derive(Clone, Debug)]
pub struct PcmSource {
    samples: Arc<[i16]>,
    channels: u8,
    loop_range: Option<(usize, usize)>,
}

impl PcmSource {
    /// Build a finite PCM source. Trailing values that do not complete a frame are rejected.
    pub fn new(samples: Arc<[i16]>, channels: u8) -> Result<Self> {
        if channels == 0 {
            return Err(Error::new(0, "PCM source has zero channels"));
        }
        if samples.len() % usize::from(channels) != 0 {
            return Err(Error::new(0, "PCM source does not end on a frame boundary"));
        }
        Ok(Self {
            samples,
            channels,
            loop_range: None,
        })
    }

    /// Give this source a looping frame interval `[start, end)`.
    pub fn with_loop(mut self, start: usize, end: usize) -> Result<Self> {
        if start >= end || end > self.frames() {
            return Err(Error::new(0, "PCM loop range is outside the source frames"));
        }
        self.loop_range = Some((start, end));
        Ok(self)
    }

    pub fn channels(&self) -> u8 {
        self.channels
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels)
    }

    pub fn loop_range(&self) -> Option<(usize, usize)> {
        self.loop_range
    }

    /// Read one decoded sample without creating a guest stream cursor. This is used by the game
    /// host to mix real bank samples for gameplay families whose authored message programs are
    /// still being recovered.
    pub fn sample(&self, frame: usize, channel: usize) -> Option<f32> {
        if channel >= usize::from(self.channels) {
            return None;
        }
        self.samples
            .get(
                frame
                    .checked_mul(usize::from(self.channels))?
                    .checked_add(channel)?,
            )
            .copied()
            .map(sample_to_float)
    }
}

#[derive(Clone, Debug)]
struct Cursor {
    source: PcmSource,
    frame: usize,
}

/// PCM cursors keyed by the guest stream object that owns them.
#[derive(Default)]
pub struct PcmStreams {
    streams: HashMap<u32, Cursor>,
}

/// Archive-resolved decoded PCM supplied to the resident play-command path.
///
/// `sample_key` is the guest address carried in the command's source-offset word. The archive
/// layer owns resolving that address and decoding it off the render thread; this host owns only
/// the inexpensive cursor attachment performed by the audio command drain.
#[derive(Clone, Debug)]
pub struct CachedPcm {
    pub source: PcmSource,
    pub sample_rate: u32,
}

/// A cache-backed implementation of the resident SndPlayer1 play host.
///
/// The same [`CachedPcm`] may serve several player streams. Each accepted `prepare` attaches a
/// fresh cursor, so overlapping voices never share playback position. The current recovered
/// resident branch is type zero; type one/two source scheduling remains outside this host.
#[derive(Default)]
pub struct PcmPlayHost {
    sources: HashMap<u32, CachedPcm>,
    streams: PcmStreams,
}

/// Command-ring host for decoded resident samples.
///
/// It serves resident play commands and supplies the corresponding decoder cursor. Graph stop
/// remains on the concrete graph-runtime owner because the recovered handler releases the owning
/// graph through its dynamic child and allocator entries.
#[derive(Default)]
pub struct PcmCommandHost {
    pub playback: PcmPlayHost,
}

impl PcmPlayHost {
    /// Add or replace one archive-resolved source. A zero sample rate cannot establish the
    /// SndPlayer1 slot timing and is rejected before it can be queued.
    pub fn insert(&mut self, sample_key: u32, cached: CachedPcm) -> Result<()> {
        if cached.sample_rate == 0 {
            return Err(Error::new(sample_key, "cached PCM has zero sample rate"));
        }
        self.sources.insert(sample_key, cached);
        Ok(())
    }

    /// Release the per-stream cursor when its player graph retires.
    pub fn detach(&mut self, stream: u32) -> Option<PcmSource> {
        self.streams.detach(stream)
    }

    /// Whether this player stream currently has a private decoded-PCM cursor.
    pub fn is_attached(&self, stream: u32) -> bool {
        self.streams.is_attached(stream)
    }
}

impl PlayHost for PcmPlayHost {
    fn prepare(&mut self, g: &mut Guest, stream: u32, index: u8, sample_key: u32) -> Result<bool> {
        let Some(cached) = self.sources.get(&sample_key).cloned() else {
            return Ok(false);
        };
        let slot = play::slot_address(g, stream, index)?;
        let record = play::record_address(g, stream, index)?;
        g.set_u32(
            slot + play::SLOT_RATE,
            (cached.sample_rate as f32).to_bits(),
        )?;
        g.set_u32(
            slot + play::SLOT_QUEUED,
            u32::try_from(cached.source.frames()).unwrap_or(u32::MAX),
        )?;
        g.set_u8(record + play::RECORD_KIND, 0)?;
        self.streams.attach(stream, cached.source);
        Ok(true)
    }

    fn decode(&mut self, _g: &mut Guest, stream: u32, _index: u8, _detail: u32) -> Result<bool> {
        Ok(self.streams.is_attached(stream))
    }
}

impl CommandHost for PcmCommandHost {
    fn play(&mut self, g: &mut Guest, record: u32) -> Result<u32> {
        play::append_resident(g, &mut self.playback, record)
    }
}

/// Command-ring host for resident PCM backed by a concrete graph owner.
///
/// Unlike [`PcmCommandHost`], this host implements the recovered eight-byte player-stop command.
/// It snapshots the graph's module pointers, releases the graph through its exact dynamic child and
/// allocator entries, then drops PCM cursors belonging to those modules. The snapshot is necessary:
/// `sub_82B48F28` is allowed to release the graph allocation, so walking `player + 80` afterward
/// would read freed guest memory.
pub struct GraphPcmCommandHost<R> {
    pub playback: PcmPlayHost,
    pub release: R,
}

impl<R> GraphPcmCommandHost<R> {
    pub fn new(release: R) -> Self {
        Self {
            playback: PcmPlayHost::default(),
            release,
        }
    }
}

impl<R: GraphReleaseHost> CommandHost for GraphPcmCommandHost<R> {
    fn play(&mut self, g: &mut Guest, record: u32) -> Result<u32> {
        play::append_resident(g, &mut self.playback, record)
    }

    fn stop_player(&mut self, g: &mut Guest, heap: &mut dyn Heap, record: u32) -> Result<u32> {
        let player = g.u32(record + 4)?;
        let children = (0..u32::from(g.u8(player + player::GRAPH_CHILD_COUNT)?))
            .map(|index| g.u32(player + player::GRAPH_CHILDREN + 4 * index))
            .collect::<Result<Vec<_>>>()?;
        player::release_graph(g, heap, &mut self.release, player, 0)?;
        for child in children {
            self.playback.detach(child);
        }
        Ok(8)
    }
}

impl<R> StreamFill for GraphPcmCommandHost<R> {
    fn fill(&mut self, g: &mut Guest, stream: u32, descriptor: u32, frames: u64) -> Result<u64> {
        self.playback.streams.fill(g, stream, descriptor, frames)
    }
}

/// The command-drain owner is also the graph's PCM provider. Keeping both capabilities on one
/// object means a resident-play command attaches the cursor that the subsequent SndPlayer1 pass
/// reads, without exposing or duplicating the private stream table at the runtime boundary.
impl StreamFill for PcmCommandHost {
    fn fill(&mut self, g: &mut Guest, stream: u32, descriptor: u32, frames: u64) -> Result<u64> {
        self.playback.streams.fill(g, stream, descriptor, frames)
    }
}

impl PcmStreams {
    /// Associate a fresh cursor with `stream`. Replacing a stream deliberately restarts it.
    pub fn attach(&mut self, stream: u32, source: PcmSource) {
        self.streams.insert(stream, Cursor { source, frame: 0 });
    }

    /// Stop retaining a source after its voice/stream has retired.
    pub fn detach(&mut self, stream: u32) -> Option<PcmSource> {
        self.streams.remove(&stream).map(|cursor| cursor.source)
    }

    pub fn is_attached(&self, stream: u32) -> bool {
        self.streams.contains_key(&stream)
    }

    fn available(cursor: &Cursor) -> usize {
        match cursor.source.loop_range {
            Some(_) => usize::MAX,
            None => cursor.source.frames().saturating_sub(cursor.frame),
        }
    }
}

fn sample_to_float(sample: i16) -> f32 {
    if sample == i16::MIN {
        -1.0
    } else {
        f32::from(sample) / f32::from(i16::MAX)
    }
}

impl StreamFill for PcmStreams {
    fn fill(&mut self, g: &mut Guest, stream: u32, descriptor: u32, frames: u64) -> Result<u64> {
        let cursor = self
            .streams
            .get_mut(&stream)
            .ok_or_else(|| Error::new(stream, "no decoded PCM is attached to this stream"))?;
        let requested = usize::try_from(frames)
            .map_err(|_| Error::new(stream, "PCM fill request does not fit host usize"))?;
        let channels = g.u8(stream + 46)?;
        if channels != cursor.source.channels {
            return Err(Error::new(
                stream + 46,
                format!(
                    "guest stream has {channels} channels but PCM source has {}",
                    cursor.source.channels
                ),
            ));
        }
        let stride = usize::from(g.u16(descriptor + 14)?);
        if stride < requested {
            return Err(Error::new(
                descriptor + 14,
                "PCM descriptor stride is shorter than requested frames",
            ));
        }
        // The no-scratch branch of deliver_frames credits the whole request regardless of a fill
        // return value. Refuse an underflow before it can advance the guest cursor past PCM.
        if g.u8(stream + 51)? == 0 && PcmStreams::available(cursor) < requested {
            return Err(Error::new(
                stream,
                "direct PCM fill would underflow; use a ready scratch segment",
            ));
        }

        let data = g.u32(descriptor + 4)?;
        let mut produced = 0usize;
        while produced < requested {
            let end = cursor
                .source
                .loop_range
                .map_or(cursor.source.frames(), |(_, end)| end);
            if cursor.frame == end {
                match cursor.source.loop_range {
                    Some((start, _)) => cursor.frame = start,
                    None => break,
                }
            }
            for channel in 0..usize::from(channels) {
                let sample = cursor.source.samples[cursor.frame * usize::from(channels) + channel];
                let at = data.wrapping_add(((channel * stride + produced) * 4) as u32);
                g.set_u32(at, sample_to_float(sample).to_bits())?;
            }
            cursor.frame += 1;
            produced += 1;
        }
        Ok(produced as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::BumpHeap;
    use crate::play::{self, REQUEST_OFFSET, REQUEST_RESULT, REQUEST_STREAM};

    const BASE: u32 = 0x4000_0000;
    const STREAM: u32 = BASE;
    const DESC: u32 = BASE + 0x100;
    const DATA: u32 = BASE + 0x400;
    const STOP_RECORD: u32 = BASE + 0x800;
    const GRAPH: u32 = BASE + 0x1000;
    const OWNER: u32 = BASE + 0x1800;
    const ALLOCATOR: u32 = BASE + 0x1C00;
    const ALLOCATOR_VTABLE: u32 = BASE + 0x1D00;
    const CHILD_A: u32 = BASE + 0x2000;
    const CHILD_B: u32 = BASE + 0x2100;
    const CHILD_A_VTABLE: u32 = BASE + 0x2200;
    const CHILD_B_VTABLE: u32 = BASE + 0x2300;

    fn guest(channels: u8, scratch: u8) -> Guest {
        let mut g = Guest::single(BASE, 0x3000);
        g.set_u8(STREAM + 46, channels).unwrap();
        g.set_u8(STREAM + 51, scratch).unwrap();
        g.set_u32(DESC + 4, DATA).unwrap();
        g.set_u16(DESC + 14, 8).unwrap();
        g
    }

    #[derive(Default)]
    struct Releaser(Vec<(char, u32, u32)>);

    impl GraphReleaseHost for Releaser {
        fn acquire_child(&mut self, _g: &mut Guest, child: u32, entry: u32) -> Result<()> {
            self.0.push(('a', child, entry));
            Ok(())
        }

        fn release_child(
            &mut self,
            _g: &mut Guest,
            child: u32,
            entry: u32,
            _flag: u32,
        ) -> Result<()> {
            self.0.push(('r', child, entry));
            Ok(())
        }

        fn free_graph<H: Heap + ?Sized>(
            &mut self,
            _g: &mut Guest,
            _heap: &mut H,
            allocator: u32,
            entry: u32,
            graph: u32,
        ) -> Result<()> {
            self.0.push(('f', allocator, entry));
            assert_eq!(graph, GRAPH);
            Ok(())
        }
    }

    fn graph_for_stop() -> Guest {
        let mut g = guest(1, 1);
        g.set_u32(GRAPH + player::GRAPH_OWNER, OWNER).unwrap();
        g.set_u8(GRAPH + player::GRAPH_CHILD_COUNT, 2).unwrap();
        g.set_u8(GRAPH + player::GRAPH_TYPE, 0).unwrap();
        g.set_u32(GRAPH + player::GRAPH_CHILDREN, CHILD_A).unwrap();
        g.set_u32(GRAPH + player::GRAPH_CHILDREN + 4, CHILD_B)
            .unwrap();
        for (child, table, acquire, release) in [
            (CHILD_A, CHILD_A_VTABLE, 0xA001, 0xA00C),
            (CHILD_B, CHILD_B_VTABLE, 0xB001, 0xB00C),
        ] {
            g.set_u32(child, table).unwrap();
            g.set_u32(table + player::GRAPH_VTABLE_ACQUIRE, acquire)
                .unwrap();
            g.set_u32(table + player::GRAPH_VTABLE_RELEASE, release)
                .unwrap();
        }
        g.set_u32(OWNER + 36, ALLOCATOR).unwrap();
        g.set_u32(OWNER + crate::voices::HANDLE_ARRAY, 0).unwrap();
        g.set_u16(OWNER + crate::voices::HANDLE_COUNT, 0).unwrap();
        g.set_u32(ALLOCATOR, ALLOCATOR_VTABLE).unwrap();
        g.set_u32(ALLOCATOR_VTABLE + player::GRAPH_VTABLE_RELEASE, 0xF00C)
            .unwrap();
        g.set_u32(STOP_RECORD + 4, GRAPH).unwrap();
        g
    }

    #[test]
    fn writes_interleaved_pcm_as_planar_bounded_float_samples() {
        let mut g = guest(2, 1);
        let source = PcmSource::new(Arc::from([i16::MIN, i16::MAX, 0, 16384]), 2).unwrap();
        let mut streams = PcmStreams::default();
        streams.attach(STREAM, source);
        assert_eq!(streams.fill(&mut g, STREAM, DESC, 2).unwrap(), 2);
        assert_eq!(g.f32(DATA).unwrap(), -1.0);
        assert_eq!(g.f32(DATA + 4).unwrap(), 0.0);
        assert_eq!(g.f32(DATA + 8 * 4).unwrap(), 1.0);
        assert_eq!(g.f32(DATA + 9 * 4).unwrap(), 16384.0 / 32767.0);
    }

    #[test]
    fn loop_range_restarts_at_the_authored_frame_boundary() {
        let mut g = guest(1, 1);
        let source = PcmSource::new(Arc::from([100i16, 200, 300]), 1)
            .unwrap()
            .with_loop(1, 3)
            .unwrap();
        let mut streams = PcmStreams::default();
        streams.attach(STREAM, source);
        assert_eq!(streams.fill(&mut g, STREAM, DESC, 5).unwrap(), 5);
        let values: Vec<i16> = (0..5)
            .map(|i| (g.f32(DATA + 4 * i).unwrap() * 32767.0).round() as i16)
            .collect();
        assert_eq!(values, [100, 200, 300, 200, 300]);
    }

    #[test]
    fn direct_fill_refuses_to_over_credit_an_underflow() {
        let mut g = guest(1, 0);
        let source = PcmSource::new(Arc::from([100i16]), 1).unwrap();
        let mut streams = PcmStreams::default();
        streams.attach(STREAM, source);
        let error = streams.fill(&mut g, STREAM, DESC, 2).unwrap_err();
        assert!(error.message.contains("underflow"));
    }

    #[test]
    fn cached_pcm_prepares_a_resident_slot_and_keeps_a_private_cursor() {
        const REQUEST: u32 = BASE + 0x800;
        const RECORDS: u32 = BASE + 0xA00;
        const PENDING: u32 = BASE + 0xC00;
        let mut g = guest(1, 1);
        g.set_u32(STREAM + play::STREAM_RECORDS, RECORDS).unwrap();
        g.set_u32(STREAM + play::STREAM_PENDING, PENDING).unwrap();
        g.set_u32(PENDING, 1).unwrap();
        g.set_u16(STREAM + play::STREAM_SLOT_TABLE, 0x200).unwrap();
        g.set_u8(STREAM + play::STREAM_RING_SIZE, 2).unwrap();
        g.set_u32(REQUEST + REQUEST_STREAM, STREAM).unwrap();
        g.set_u32(REQUEST + REQUEST_OFFSET, 0x1234).unwrap();
        g.set_u16(REQUEST + REQUEST_RESULT, 64).unwrap();
        g.set_u32(REQUEST + play::REQUEST_LEVEL, 1.0f32.to_bits())
            .unwrap();

        let source = PcmSource::new(Arc::from([100i16, 200]), 1).unwrap();
        let mut host = PcmPlayHost::default();
        host.insert(
            0x1234,
            CachedPcm {
                source,
                sample_rate: 48_000,
            },
        )
        .unwrap();
        assert_eq!(
            play::append_resident(&mut g, &mut host, REQUEST).unwrap(),
            64
        );
        assert!(host.is_attached(STREAM));
        let slot = play::slot_address(&g, STREAM, 0).unwrap();
        assert_eq!(g.f32(slot + play::SLOT_RATE).unwrap(), 48_000.0);

        g.set_u32(DESC + 4, DATA).unwrap();
        g.set_u16(DESC + 14, 2).unwrap();
        assert_eq!(host.streams.fill(&mut g, STREAM, DESC, 2).unwrap(), 2);
        assert_eq!(
            [g.f32(DATA).unwrap(), g.f32(DATA + 4).unwrap()],
            [100.0 / 32767.0, 200.0 / 32767.0]
        );
        assert_eq!(host.detach(STREAM).unwrap().frames(), 2);
    }

    #[test]
    fn command_host_exposes_the_cursor_its_playback_host_attached() {
        let mut g = guest(1, 1);
        let mut host = PcmCommandHost::default();
        host.playback
            .streams
            .attach(STREAM, PcmSource::new(Arc::from([100i16, 200]), 1).unwrap());
        assert_eq!(host.fill(&mut g, STREAM, DESC, 2).unwrap(), 2);
        assert_eq!(
            [g.f32(DATA).unwrap(), g.f32(DATA + 4).unwrap()],
            [100.0 / 32767.0, 200.0 / 32767.0]
        );
    }

    #[test]
    fn graph_stop_releases_the_graph_before_detaching_its_pcm_cursors() {
        let mut g = graph_for_stop();
        let mut host = GraphPcmCommandHost::new(Releaser::default());
        host.playback.streams.attach(
            CHILD_A,
            PcmSource::new(Arc::from([100i16, 200]), 1).unwrap(),
        );
        let mut heap = BumpHeap {
            next: BASE + 0x2800,
            end: BASE + 0x2f00,
        };

        assert_eq!(host.stop_player(&mut g, &mut heap, STOP_RECORD).unwrap(), 8);
        assert!(!host.playback.is_attached(CHILD_A));
        assert_eq!(
            host.release.0,
            [
                ('a', CHILD_A, 0xA001),
                ('r', CHILD_A, 0xA00C),
                ('a', CHILD_B, 0xB001),
                ('r', CHILD_B, 0xB00C),
                ('f', ALLOCATOR, 0xF00C),
            ]
        );
    }
}
