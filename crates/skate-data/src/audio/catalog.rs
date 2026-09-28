//! Player bank loading and offline PCM preparation from the owner's retail archives.
//!
//! The whitelist is the object-to-bank attribution in `docs/audio-player-sounds-handoff.md`.
//! It includes every variant in each bank; the authored patch chooses what to play. Dialogue,
//! Hall of Meat voices, ambient emitters, and unproven dynamic-object routes are not installed.
//! Archive IO, external decoding, and the content-validated disk cache happen before the audio
//! runtime starts. Guest addresses are assigned by the runtime, never persisted in the cache.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use skate_audio_core::pcm::{CachedPcm, PcmSource};
use skate_audio_core::{Guest, Segment};
use skate_audio_formats::{banks, eaac, eb, splc};

use super::{Error, describe};

/// Evidence-backed resident banks used by the rider and board's authored sound objects.
pub const PLAYER_BANKS: &[&str] = &[
    "Sk8_Air_Flip_Tricks.abk",
    "Foley_Cloth.abk",
    "Treatments.abk",
    "Brd_Squeaks.abk",
    "Seams_Bank.abk",
    "PatchBank_Rolling_Surfaces.abk",
    "PatchBank_Objects.abk",
    "PatchBank_SpiderCracks.abk",
    "PatchBank_RocksBounce.abk",
    "Rolling_Rattles.abk",
    "WHEEL_SKID_BANK.abk",
    "GRINDS.abk",
    "board_scrapes.abk",
    "Bodyslide.abk",
    "fstep_skateshoe1_sm.abk",
    "FOOT_DRAG.abk",
    "sense_of_speed.abk",
];

/// The `SPLC` sound banks the one-shot ("Splice") voices play from: wheel pops and the landing
/// impact (`sub_824B9CC8`, `sub_824BA630`) name `Skate_Collisions` samples through the vault.
/// `DLC_Cartoon_Collisions.bnk` is the pops' DLC variant and is absent from a base installation,
/// so it is loaded only when the archive holds it.
pub const SPLICE_BANKS: &[&str] = &["Skate_Collisions.bnk"];
/// `Skate_Metal.bnk` and `HOM_Set_1.bnk` are the other two banks the collision materials name:
/// `sub_824967F8` picks a material's sample out of the field family its kind word selects, and an
/// AttribSys field's type name is its bank (`Skate_Metal` -> `Skate_Metal.bnk`). A metal rail
/// resolves into `Skate_Metal.bnk`, which is why a rail grind does not sound like concrete. Both
/// are optional so a trimmed installation still boots.
/// `sk8_menu.bnk` holds the front-end one-shots. The session marker's three sounds name it the
/// same way a collision material does -- the vault's `fe` records carry their sample id in a field
/// whose *type name* is `sk8_menu`, so the bank falls out of the same convention:
///
/// | vault `fe` key | sample | |
/// |---|---|---|
/// | `cellphone_place_marker` | 237 | the marker is set |
/// | `cellphone_goto_marker` | 236 | the session returns to it |
/// | `cellphone_marker_error` | 209 | the placement was refused |
///
/// Optional like the rest, so a trimmed installation still boots -- without it the marker is
/// simply silent again rather than failing to start.
pub const MENU_BANK: &str = "sk8_menu.bnk";
pub const OPTIONAL_SPLICE_BANKS: &[&str] = &[
    "DLC_Cartoon_Collisions.bnk",
    "Skate_Metal.bnk",
    "HOM_Set_1.bnk",
    MENU_BANK,
];

/// Where the installer stages the MixMap under `assets`.
pub const MIXMAP_PATH: &str = "private/stock/data/audio/MixMapSK8.mxb";

/// One decoded sample, addressed by its original EAAC header offset within its bank.
#[derive(Clone, Debug)]
pub struct BankPcm {
    pub bank: String,
    pub index: usize,
    pub header_offset: u32,
    pub header: eaac::Header,
    pub pcm: CachedPcm,
}

/// Ready-to-install assets. Nothing here performs IO after construction.
pub struct PlayerAudioCatalog {
    pub guest: Guest,
    pub projects: Vec<Vec<u8>>,
    pub banks: Vec<(String, Vec<u8>)>,
    pub samples: Vec<BankPcm>,
    pub cache_hits: usize,
    /// `data/audio/MixMapSK8.mxb`, the MixMap the game loads in `sub_82484FE8` (a loose file,
    /// staged at `<assets>/private/stock/data/audio/`). Set by [`Self::from_assets`]; `None` when
    /// the file is not staged or the catalog was built from explicit paths. Install it with
    /// `AuthoredRuntime::load_mixmap`.
    pub mixmap: Option<Vec<u8>>,
}

impl PlayerAudioCatalog {
    /// Load the prepared setup layout under an installation's `assets` directory.
    pub fn from_assets(assets: &Path, cache: Option<&Path>) -> Result<Self, Error> {
        let mut catalog = Self::load(
            &assets.join("private/stock/data/audio/audiofiles.big"),
            &assets.join("private/stock/audio-runtime-image"),
            cache,
        )?;
        catalog.mixmap = std::fs::read(assets.join(MIXMAP_PATH)).ok();
        Ok(catalog)
    }

    /// Decode all rider/board samples, or load a cache whose source bytes match exactly.
    pub fn load(archive: &Path, image: &Path, cache: Option<&Path>) -> Result<Self, Error> {
        Self::load_banks(archive, image, cache, PLAYER_BANKS)
    }

    /// The `SPLC` sound banks of [`SPLICE_BANKS`], with every sample in their tables decoded. The
    /// samples land in [`Self::samples`] addressed by their EAAC header offset, exactly as the
    /// patch banks' are, so `AuthoredRuntime::insert_pcm(bank_base + header_offset, ..)` serves
    /// both.
    pub fn load_splice_banks(&mut self, archive: &Path, cache: Option<&Path>) -> Result<(), Error> {
        let data = std::fs::read(archive).map_err(|e| io_error(archive, e))?;
        let parsed = eb::Archive::parse(&data)?;
        let wanted: Vec<(&str, bool)> = SPLICE_BANKS
            .iter()
            .map(|n| (*n, true))
            .chain(OPTIONAL_SPLICE_BANKS.iter().map(|n| (*n, false)))
            .collect();
        for (name, required) in wanted {
            let member = parsed.entries.iter().find(|e| {
                e.name
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            });
            let member = match (member, required) {
                (Some(member), _) => member,
                (None, false) => continue,
                (None, true) => return Err(Error::Format(format!("missing sound bank {name}"))),
            };
            if member.is_compressed() {
                return Err(Error::Format(format!("compressed sound bank {name}")));
            }
            let bytes = member_bytes(&data, member)?;
            let bank = splc::Splc::parse(bytes)?;
            let ranges: Vec<std::ops::Range<usize>> = (0..bank.samples.len())
                .map(|index| {
                    let start = bank.streams_offset + bank.samples[index].offset as usize;
                    let end = bank.streams_offset + bank.samples[index].end as usize;
                    start..end.max(start)
                })
                .collect();
            let path = cache
                .map(|root| root.join(format!("{name}.{:016x}.v3.pcm", fingerprint(bytes))));
            let cached = path
                .as_deref()
                .and_then(|path| read_cache_ranges(path, bytes, &ranges));
            let decoded = match cached {
                Some(decoded) => {
                    self.cache_hits += decoded.len();
                    decoded
                }
                None => match decode_ranges(name, bytes, &ranges) {
                    Ok(decoded) => {
                        if let Some(path) = path.as_deref() {
                            let _ = write_cache(path, bytes, &decoded);
                        }
                        decoded
                    }
                    // An optional bank must never take the whole player-sound path down. Some of
                    // them use a container that needs ffmpeg, which is not present everywhere, and
                    // before these banks were listed that only cost the contacts they carry. Skip
                    // the bank and say so once; a material whose sample lives here then resolves
                    // to silence exactly as an uninstalled bank already does.
                    Err(error) if !required => {
                        eprintln!("SKATE_PLAYER_AUDIO optional sound bank {name} not decoded: {error}");
                        continue;
                    }
                    Err(error) => return Err(error),
                },
            };
            for (index, (offset, header, samples)) in decoded.into_iter().enumerate() {
                let mut source = PcmSource::new(Arc::from(samples), header.channels())
                    .map_err(|e| Error::Format(e.to_string()))?;
                if let Some(start) = header.loop_start {
                    source = source
                        .with_loop(start as usize, header.num_samples as usize)
                        .map_err(|e| Error::Format(format!("{name}#{index}: {e}")))?;
                }
                self.samples.push(BankPcm {
                    bank: name.into(),
                    index,
                    header_offset: offset,
                    header,
                    pcm: CachedPcm {
                        source,
                        sample_rate: header.sample_rate,
                    },
                });
            }
            self.banks.push((name.into(), bytes.to_vec()));
        }
        Ok(())
    }

    /// An explicit bank subset for fixture replay; normal gameplay loads the full catalog.
    pub fn load_banks(
        archive: &Path,
        image: &Path,
        cache: Option<&Path>,
        names: &[&str],
    ) -> Result<Self, Error> {
        let guest = load_guest_image(image)?;
        let data = std::fs::read(archive).map_err(|e| io_error(archive, e))?;
        let archive = eb::Archive::parse(&data)?;
        let mut projects = Vec::new();
        for member in &archive.entries {
            if member
                .name
                .as_deref()
                .is_some_and(|name| name.ends_with(".csi"))
            {
                if member.is_compressed() {
                    return Err(Error::Format(
                        "compressed CSI project is unsupported".into(),
                    ));
                }
                let bytes = member_bytes(&data, member)?;
                banks::Csi::parse(bytes)?;
                projects.push(bytes.to_vec());
            }
        }
        if projects.is_empty() {
            return Err(Error::Format("audiofiles.big has no CSI projects".into()));
        }
        let mut catalog = Self {
            guest,
            projects,
            banks: Vec::new(),
            samples: Vec::new(),
            cache_hits: 0,
            mixmap: None,
        };
        for &name in names {
            let member = archive
                .entries
                .iter()
                .find(|e| {
                    e.name
                        .as_deref()
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
                .ok_or_else(|| Error::Format(format!("missing player bank {name}")))?;
            if member.is_compressed() {
                return Err(Error::Format(format!(
                    "compressed player bank {name} is unsupported"
                )));
            }
            let bytes = member_bytes(&data, member)?;
            let parsed = banks::Abk::parse(bytes)?;
            let path =
                cache.map(|root| root.join(format!("{name}.{:016x}.v3.pcm", fingerprint(bytes))));
            let cached = path
                .as_deref()
                .and_then(|path| read_cache(path, bytes, &parsed));
            let decoded = match cached {
                Some(decoded) => {
                    catalog.cache_hits += decoded.len();
                    decoded
                }
                None => {
                    let decoded = decode_bank(name, bytes, &parsed)?;
                    if let Some(path) = path.as_deref() {
                        // A cache is an optimization. A read-only installation must still play.
                        let _ = write_cache(path, bytes, &decoded);
                    }
                    decoded
                }
            };
            for (index, (offset, header, samples)) in decoded.into_iter().enumerate() {
                let mut source = PcmSource::new(Arc::from(samples), header.channels())
                    .map_err(|e| Error::Format(e.to_string()))?;
                if let Some(start) = header.loop_start {
                    source = source
                        .with_loop(start as usize, header.num_samples as usize)
                        .map_err(|e| Error::Format(format!("{name}#{index}: {e}")))?;
                }
                catalog.samples.push(BankPcm {
                    bank: name.into(),
                    index,
                    header_offset: offset,
                    header,
                    pcm: CachedPcm {
                        source,
                        sample_rate: header.sample_rate,
                    },
                });
            }
            catalog.banks.push((name.into(), bytes.to_vec()));
        }
        Ok(catalog)
    }
}

fn io_error(path: &Path, error: std::io::Error) -> Error {
    Error::Format(format!("{}: {error}", path.display()))
}

fn member_bytes<'a>(data: &'a [u8], entry: &eb::Entry) -> Result<&'a [u8], Error> {
    data.get(entry.range())
        .ok_or_else(|| Error::Format(format!("archive member {:?} is out of bounds", entry.name)))
}

/// Load only correctly named guest-image regions and reject overlapping mappings.
pub fn load_guest_image(directory: &Path) -> Result<Guest, Error> {
    let mut segments = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(|e| io_error(directory, e))? {
        let path = entry.map_err(|e| io_error(directory, e))?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("bin") {
            continue;
        }
        let Some(hex) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix("g_"))
        else {
            continue;
        };
        if hex.len() != 4 {
            continue;
        }
        let page = u32::from_str_radix(hex, 16)
            .map_err(|_| Error::Format(format!("invalid guest image region {}", path.display())))?;
        let bytes = std::fs::read(&path).map_err(|e| io_error(&path, e))?;
        if bytes.is_empty() || page < 0x8200 || page > 0x8320 {
            return Err(Error::Format(format!(
                "invalid guest image span {}",
                path.display()
            )));
        }
        segments.push(Segment {
            base: page << 16,
            bytes,
        });
    }
    segments.sort_by_key(|s| s.base);
    for pair in segments.windows(2) {
        if u64::from(pair[0].base) + pair[0].bytes.len() as u64 > u64::from(pair[1].base) {
            return Err(Error::Format("guest image regions overlap".into()));
        }
    }
    let guest = Guest::from_segments(segments);
    for at in [
        0x82FC_E4CC,
        0x82FD_28C0,
        0x82FD_35F4,
        0x8302_D4A4,
        0x8303_6F4C,
    ] {
        guest
            .span(at, 4)
            .map_err(|e| Error::Format(format!("incomplete guest audio image: {e}")))?;
    }
    Ok(guest)
}

type Decoded = Vec<(u32, eaac::Header, Vec<i16>)>;

fn decode_bank(name: &str, bytes: &[u8], bank: &banks::Abk) -> Result<Decoded, Error> {
    let ranges = (0..bank.present())
        .map(|index| {
            bank.sample_range(index)
                .ok_or_else(|| Error::Format(format!("{name}#{index}: absent sample")))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    decode_ranges(name, bytes, &ranges)
}

/// Decode the EAAC stream in each range, as the resident-sample path needs it.
fn decode_ranges(
    name: &str,
    bytes: &[u8],
    ranges: &[std::ops::Range<usize>],
) -> Result<Decoded, Error> {
    let mut decoded = Vec::with_capacity(ranges.len());
    for (index, range) in ranges.iter().enumerate() {
        let range = range.clone();
        let data = bytes
            .get(range.clone())
            .ok_or_else(|| Error::Format(format!("{name}#{index}: sample outside bank")))?;
        let header = eaac::Header::parse(data, 0)?;
        if header.codec != eaac::Codec::Xma || header.stream_type != 0 {
            return Err(Error::Format(format!(
                "{name}#{index}: unsupported {:?} stream type {}",
                header.codec, header.stream_type
            )));
        }
        let blocks = eaac::blocks(&data[header.size()..])?;
        let frames: u64 = blocks
            .iter()
            .map(|block| u64::from(block.num_samples))
            .sum();
        if frames != u64::from(header.num_samples) {
            return Err(Error::Format(format!(
                "{name}#{index}: header/block sample counts disagree"
            )));
        }
        let info = describe(data, 0)?;
        let pcm = super::ffmpeg::decode_resident(data)
            .map_err(|e| Error::Format(format!("{name}#{index}: {e}")))?;
        let expected = info.num_samples as usize * usize::from(info.channels);
        if pcm.len() != expected {
            return Err(Error::Format(format!(
                "{name}#{index}: decoded {} frames, EAAC declares {}",
                pcm.len() / usize::from(info.channels),
                info.num_samples
            )));
        }
        decoded.push((range.start as u32, header, pcm));
    }
    Ok(decoded)
}

// The fingerprint selects a filename only: read_cache also compares the complete encoded bank.
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
    })
}

const CACHE_MAGIC: &[u8; 8] = b"S3PCM003";

fn read_cache(path: &Path, bank: &[u8], parsed: &banks::Abk) -> Option<Decoded> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.get(..8)? != CACHE_MAGIC || bytes.get(8..8 + bank.len())? != bank {
        return None;
    }
    let mut cursor = 8 + bank.len();
    let mut decoded = Vec::with_capacity(parsed.present());
    for index in 0..parsed.present() {
        let range = parsed.sample_range(index)?;
        let header = eaac::Header::parse(bank.get(range.clone())?, 0).ok()?;
        let length = header.num_samples as usize * usize::from(header.channels()) * 2;
        let pcm = bytes
            .get(cursor..cursor.checked_add(length)?)?
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        decoded.push((range.start as u32, header, pcm));
        cursor += length;
    }
    (cursor == bytes.len()).then_some(decoded)
}

/// [`read_cache`] for a bank whose samples are addressed by explicit ranges (`SPLC`).
fn read_cache_ranges(path: &Path, bank: &[u8], ranges: &[std::ops::Range<usize>]) -> Option<Decoded> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.get(..8)? != CACHE_MAGIC || bytes.get(8..8 + bank.len())? != bank {
        return None;
    }
    let mut cursor = 8 + bank.len();
    let mut decoded = Vec::with_capacity(ranges.len());
    for range in ranges {
        let header = eaac::Header::parse(bank.get(range.clone())?, 0).ok()?;
        let length = header.num_samples as usize * usize::from(header.channels()) * 2;
        let pcm = bytes
            .get(cursor..cursor.checked_add(length)?)?
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        decoded.push((range.start as u32, header, pcm));
        cursor += length;
    }
    (cursor == bytes.len()).then_some(decoded)
}

fn write_cache(path: &Path, bank: &[u8], decoded: &Decoded) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = PathBuf::from(format!("{}.{}.tmp", path.display(), std::process::id()));
    let mut file = std::io::BufWriter::new(std::fs::File::create(&temporary)?);
    file.write_all(CACHE_MAGIC)?;
    file.write_all(bank)?;
    for (_, _, samples) in decoded {
        for sample in samples {
            file.write_all(&sample.to_le_bytes())?;
        }
    }
    file.flush()?;
    drop(file);
    std::fs::rename(temporary, path)
}
