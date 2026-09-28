//! Cook the voice device's module classes from a dumped image and print every parameter's default
//! block, decoded by tag. A reading aid for the device open, not a verifier: the dump predates
//! registration, so nothing here is compared with the game.
//!
//!     cargo run --release --example voice_classes -- <image dir>

use skate_audio_core::classes::{self, TAG_INTEGER, TAG_POINTER, TAG_SINGLE, TAG_STRING};
use skate_audio_core::{Guest, Segment};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let image = std::env::args()
        .nth(1)
        .ok_or("usage: voice_classes <image dir>")?;
    let mut segments = Vec::new();
    for entry in std::fs::read_dir(&image)? {
        let path = entry?.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(hex) = stem.strip_prefix("g_") else {
            continue;
        };
        segments.push(Segment {
            base: u32::from_str_radix(hex, 16)? << 16,
            bytes: std::fs::read(&path)?,
        });
    }
    let mut g = Guest::from_segments(segments);
    const OUT: u32 = 0x5000_0000;
    g.put(OUT, vec![0; 0x1000]);

    let mut list: Vec<(u32, &str)> = classes::CLASSES.iter().map(|(_, c, n)| (*c, *n)).collect();
    list.push((0x82FD_28C0, "Send"));
    list.push((0x82FC_E4CC, "GainFader"));
    for (class, name) in list {
        let id = g.u32(class + 36)?;
        let fourcc: String = id
            .to_be_bytes()
            .iter()
            .map(|&b| if b.is_ascii_graphic() { b as char } else { '.' })
            .collect();
        let (leading, instance, params) = (g.u8(class + 41)?, g.u8(class + 42)?, g.u8(class + 43)?);
        let counts = g.u32(class + 24)?;
        let per: Vec<u32> = (0..params as u32)
            .map(|i| g.u32(counts + 8 * i))
            .collect::<Result<_, _>>()?;
        println!(
            "{name} {class:#x}: id {id:#010x} '{fourcc}', kind {}, rows {leading}+{instance}, {params} parameters {per:?}",
            g.u8(class + 40)?
        );
        classes::cook_class(&mut g, class)?;
        for (i, &n) in per.iter().enumerate() {
            g.fill(OUT, 0, 0x1000)?;
            classes::class_defaults(&mut g, class, i as u32, OUT)?;
            let slots: Vec<String> = (0..n)
                .map(|k| {
                    let (tag, value) =
                        (g.u32(OUT + 8 * k).unwrap(), g.u32(OUT + 8 * k + 4).unwrap());
                    match tag {
                        TAG_SINGLE => format!("f{}", f32::from_bits(value)),
                        TAG_INTEGER => format!("i{}", value as i32),
                        TAG_POINTER => format!("p{value:#x}"),
                        TAG_STRING => format!("s{value:#x}"),
                        _ => format!("d{}", f64::from_bits(g.u64(OUT + 8 * k).unwrap())),
                    }
                })
                .collect();
            println!("  param {i}: {}", slots.join(" "));
        }
    }
    Ok(())
}
