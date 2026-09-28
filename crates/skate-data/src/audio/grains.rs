//! `grains.big` loading for the retail grain player: each `.grain` member's bytes (placed in guest
//! memory unmodified, as the title's loader does) with its EAAC stream decoded once through the
//! same ffmpeg XMA path and a content-validated disk cache, and the per-surface vault tuning of
//! class `0x7AB23C11B6ADA2DE` from the owner's `skater-collections.json`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use skate_audio_core::grain::board::SurfaceTuning;
use skate_audio_core::grain::chain::ChainConfig;
use skate_audio_core::grain::envelope::PushTuning;
use skate_audio_formats::grain::{self, Grain};

use super::Error;

/// The grain surface class.
pub const CLASS: &str = "Hash_7AB23C11B6ADA2DE";
/// The board owner's audio tuning class read by `sub_824CA938` (the tuning holder's `+132`).
pub const CHAIN_CLASS: &str = "Hash_6E878344774A7999";
/// `Sk8::Audio::eEQChain`'s class (the tuning holder's `+140`).
pub const EQ_CLASS: &str = "Hash_42AFE160E647167C";

/// One decoded grain member.
#[derive(Clone, Debug)]
pub struct GrainMember {
    pub name: String,
    pub bytes: Vec<u8>,
    pub samples: Arc<[i16]>,
    pub channels: u8,
    pub rate: u32,
}

const CACHE_MAGIC: &[u8; 8] = b"S3GRN001";

fn io_error(path: &Path, error: std::io::Error) -> Error {
    Error::Format(format!("{}: {error}", path.display()))
}

fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
    })
}

fn cache_path(cache: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    cache.join(format!("grain-{name}-{:016x}.pcm", fingerprint(bytes)))
}

fn read_cache(path: &Path, member: &[u8], expected: usize) -> Option<Vec<i16>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.get(..8)? != CACHE_MAGIC || bytes.get(8..8 + member.len())? != member {
        return None;
    }
    let pcm = &bytes[8 + member.len()..];
    (pcm.len() == expected * 2).then(|| {
        pcm.chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect()
    })
}

fn write_cache(path: &Path, member: &[u8], pcm: &[i16]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = PathBuf::from(format!("{}.{}.tmp", path.display(), std::process::id()));
    let mut file = std::io::BufWriter::new(std::fs::File::create(&temporary)?);
    file.write_all(CACHE_MAGIC)?;
    file.write_all(member)?;
    for sample in pcm {
        file.write_all(&sample.to_le_bytes())?;
    }
    file.flush()?;
    drop(file);
    std::fs::rename(temporary, path)
}

/// Load the named members of `grains.big` (all of them when `names` is empty).
pub fn load_grains(
    archive: &Path,
    names: &[&str],
    cache: Option<&Path>,
) -> Result<Vec<GrainMember>, Error> {
    let data = std::fs::read(archive).map_err(|e| io_error(archive, e))?;
    let mut out = Vec::new();
    for (name, range) in grain::members(&data)? {
        if !names.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            continue;
        }
        let bytes = data[range].to_vec();
        let parsed = Grain::parse(&bytes)?;
        let channels = parsed.stream.channels();
        let expected = parsed.stream.num_samples as usize * usize::from(channels);
        let cached = cache
            .map(|c| cache_path(c, &name, &bytes))
            .and_then(|p| read_cache(&p, &bytes, expected));
        let samples = match cached {
            Some(pcm) => pcm,
            None => {
                let pcm = super::ffmpeg::decode_resident(parsed.stream_bytes())
                    .map_err(|e| Error::Format(format!("{name}: {e}")))?;
                if pcm.len() != expected {
                    return Err(Error::Format(format!(
                        "{name}: decoded {} frames, EAAC declares {}",
                        pcm.len() / usize::from(channels),
                        parsed.stream.num_samples
                    )));
                }
                if let Some(c) = cache {
                    // A failed cache write only costs a decode next time.
                    let _ = write_cache(&cache_path(c, &name, &bytes), &bytes, &pcm);
                }
                pcm
            }
        };
        out.push(GrainMember {
            name,
            channels,
            rate: parsed.stream.sample_rate,
            samples: Arc::from(samples),
            bytes,
        });
    }
    for name in names {
        if !out.iter().any(|m| m.name.eq_ignore_ascii_case(name)) {
            return Err(Error::Format(format!("grains.big has no member {name}")));
        }
    }
    Ok(out)
}

/// Load the named members of an EB archive (`wheels.big`) as the resource loaders place them.
/// `.snr` members (an EAAC header at offset 0 and one resident block) also get their stream
/// decoded, through the same ffmpeg path and cache as the grains. Every other member (`.sek` seek
/// tables) comes back with its bytes only (`samples` empty, `channels` and `rate` 0).
pub fn load_members(
    archive: &Path,
    names: &[&str],
    cache: Option<&Path>,
) -> Result<Vec<GrainMember>, Error> {
    let data = std::fs::read(archive).map_err(|e| io_error(archive, e))?;
    let parsed = skate_audio_formats::eb::Archive::parse(&data)?;
    let mut out = Vec::new();
    for name in names {
        let entry = parsed
            .find(name)
            .ok_or_else(|| Error::Format(format!("{} has no member {name}", archive.display())))?;
        if entry.is_compressed() {
            return Err(Error::Format(format!("{name}: compressed members are unsupported")));
        }
        let range = entry.range();
        let bytes = data
            .get(range)
            .ok_or_else(|| Error::Format(format!("{name}: member out of bounds")))?
            .to_vec();
        if !name.to_ascii_lowercase().ends_with(".snr") {
            out.push(GrainMember {
                name: (*name).to_owned(),
                bytes,
                samples: Arc::from(Vec::new()),
                channels: 0,
                rate: 0,
            });
            continue;
        }
        let header = skate_audio_formats::eaac::Header::parse(&bytes, 0)?;
        let channels = header.channels();
        let expected = header.num_samples as usize * usize::from(channels);
        let cached = cache
            .map(|c| cache_path(c, name, &bytes))
            .and_then(|p| read_cache(&p, &bytes, expected));
        let samples = match cached {
            Some(pcm) => pcm,
            None => {
                let pcm = super::ffmpeg::decode_resident(&bytes)
                    .map_err(|e| Error::Format(format!("{name}: {e}")))?;
                if pcm.len() != expected {
                    return Err(Error::Format(format!(
                        "{name}: decoded {} frames, EAAC declares {}",
                        pcm.len() / usize::from(channels),
                        header.num_samples
                    )));
                }
                if let Some(c) = cache {
                    let _ = write_cache(&cache_path(c, name, &bytes), &bytes, &pcm);
                }
                pcm
            }
        };
        out.push(GrainMember {
            name: (*name).to_owned(),
            channels,
            rate: header.sample_rate,
            samples: Arc::from(samples),
            bytes,
        });
    }
    Ok(out)
}

/// The vault rows of the grain class, keyed by collection key.
pub struct GrainVault {
    rows: Vec<Value>,
}

fn word(hex: &str) -> Result<u32, Error> {
    u32::from_str_radix(hex, 16).map_err(|_| Error::Format(format!("bad vault word {hex}")))
}

impl GrainVault {
    /// `private/stock/skater-collections.json` under the installation's assets.
    pub fn load(assets: &Path) -> Result<Self, Error> {
        let path = assets.join("private/stock/skater-collections.json");
        let bytes = std::fs::read(&path).map_err(|e| io_error(&path, e))?;
        let all: Value = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Format(format!("{}: {e}", path.display())))?;
        let rows = all["collections"]
            .as_array()
            .ok_or_else(|| Error::Format("skater collections have no rows".into()))?
            .iter()
            .filter(|row| {
                [CLASS, CHAIN_CLASS, EQ_CLASS]
                    .iter()
                    .any(|c| row["class"] == *c)
            })
            .cloned()
            .collect();
        Ok(Self { rows })
    }

    fn row(&self, class: &str, key: &str) -> Option<&Value> {
        self.rows
            .iter()
            .find(|row| row["class"] == class && row["key"] == key)
    }

    /// A field with the collection's parent chain (`sub_82B72420`'s fallback).
    fn field_of(&self, class: &str, key: &str, field: &str) -> Result<&Value, Error> {
        let mut current = key.to_owned();
        for _ in 0..=self.rows.len() {
            let row = self
                .row(class, &current)
                .ok_or_else(|| Error::Format(format!("missing grain collection {current}")))?;
            if let Some(value) = row["fields"].get(field) {
                return Ok(value);
            }
            match row["parent"].as_str() {
                Some(parent) if !parent.is_empty() => current = parent.to_owned(),
                _ => break,
            }
        }
        Err(Error::Format(format!(
            "grain collection {key} has no {field}"
        )))
    }

    fn field(&self, key: &str, field: &str) -> Result<&Value, Error> {
        self.field_of(CLASS, key, field)
    }

    fn word_of(&self, class: &str, key: &str, field: &str) -> Result<u32, Error> {
        let value = self.field_of(class, key, field)?;
        let data = value["data"]
            .as_str()
            .ok_or_else(|| Error::Format(format!("{key}/{field} has no data")))?;
        word(data)
    }

    fn int(&self, key: &str, field: &str) -> Result<i32, Error> {
        Ok(self.word_of(CLASS, key, field)? as i32)
    }

    fn float(&self, key: &str, field: &str) -> Result<f32, Error> {
        let value = self.field(key, field)?;
        let data = value["data"]
            .as_str()
            .ok_or_else(|| Error::Format(format!("{key}/{field} has no data")))?;
        Ok(f32::from_bits(word(data)?))
    }

    /// The tuning of collection `key` (`sub_824C8370`'s key).
    pub fn tuning(&self, key: u64) -> Result<SurfaceTuning, Error> {
        // hash64("default") is the class's root collection, stored under its readable name.
        let key = if key == 0xD7ED_BD36_2D7D_2152 {
            "default".to_owned()
        } else {
            format!("Hash_{key:016X}")
        };
        let matrix = self.field(&key, "Hash_A985FBAA9326718D")?["data"]
            .as_str()
            .ok_or_else(|| Error::Format("bezier matrix has no data".into()))?
            .to_owned();
        let lane = |row: usize| -> Result<f32, Error> {
            let at = (row * 4 + 1) * 8;
            Ok(f32::from_bits(word(matrix.get(at..at + 8).ok_or_else(
                || Error::Format("short bezier matrix".into()),
            )?)?))
        };
        let params = self.field(&key, "Hash_D18D1174735E5CDE")?;
        let items = params["array"]["items"]
            .as_array()
            .ok_or_else(|| Error::Format("GrainParams has no items".into()))?;
        let mut grain_params = [[0f32; 5]; 2];
        for (i, item) in items.iter().take(2).enumerate() {
            let hex = item
                .as_str()
                .ok_or_else(|| Error::Format("GrainParams item is not text".into()))?;
            for k in 0..5 {
                grain_params[i][k] = f32::from_bits(word(&hex[k * 8..k * 8 + 8])?);
            }
        }
        Ok(SurfaceTuning {
            bezier: [lane(0)?, lane(1)?, lane(2)?, lane(3)?],
            grain: self.field(&key, "Hash_2C073BF8BC45063B")?["data"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            max_kmh: self.float(&key, "Hash_4890392C91829954")?,
            boost_gain: self.float(&key, "Hash_CEC749561306022A")?,
            boost_kmh: self.float(&key, "Hash_D380D303C64CF6F8")?,
            shift_boost: self.float(&key, "Hash_1F459FC797B2C6BA")?,
            intensity_cap: self.float(&key, "Hash_5C9AA28695C17004")?,
            shift_b: self.float(&key, "Hash_145D8340A9440DA3")?,
            params: grain_params,
            rise_step: self.float(&key, "Hash_281FF01081475899")?,
            fall_step: self.float(&key, "Hash_63764B8C7EB9EC9B")?,
            special_gain: self.float(&key, "Hash_6BDC44AE7C3C79D0")?,
            special_shift: self.float(&key, "Hash_F62BC5EBD8E5DDE8")?,
            shift_boost_b: self.float(&key, "Hash_7FFF3A8AD44809EF")?,
            push: PushTuning {
                ramp_kmh: self.float(&key, "Hash_2D751DEB89BB5E33")?,
                scale_low: self.float(&key, "Hash_E239B03F0E890686")?,
                scale_high: self.float(&key, "Hash_B87ECDDAAB0F8404")?,
                shift_low: self.float(&key, "Hash_C658A7923FC7B99E")?,
                shift_high: self.float(&key, "Hash_A15AD56E225ADBA6")?,
                scale_ms: [
                    self.int(&key, "Hash_B3D7468820AFC661")?,
                    self.int(&key, "Hash_DAC9DA910EF0316C")?,
                    self.int(&key, "Hash_0C3D5DBC262ED276")?,
                ],
                shift_ms: [
                    self.int(&key, "Hash_09A5CC79BA2178E7")?,
                    self.int(&key, "Hash_3206FD96427EA4D2")?,
                    self.int(&key, "Hash_DB597F672CA47138")?,
                ],
            },
        })
    }

    /// The owner-side chain values `sub_824C8878` reads: `+1528`, `+1548`, `+1552` from class
    /// `0x6E878344774A7999` (`sub_824CA938`), `+1556` = 0.0 (`sub_824C59C8`), and the eEQChain bus
    /// of class `0x42AFE160E647167C`.
    pub fn chain_config(&self, local: bool) -> Result<ChainConfig, Error> {
        let f = |field: &str| -> Result<f32, Error> {
            Ok(f32::from_bits(self.word_of(
                CHAIN_CLASS,
                "default",
                field,
            )?))
        };
        Ok(ChainConfig {
            local,
            create: 0,
            eq_chain: self.word_of(EQ_CLASS, "default", "Hash_A5D3ADA63608617F")?,
            clip: f("Hash_E64C04ED542DABC8")?,
            shelf_corner: f("Hash_55BEB30353F244A9")?,
            shelf_gain: f("Hash_45516395725ED16B")?,
            local_send: 0.0,
        })
    }
}
