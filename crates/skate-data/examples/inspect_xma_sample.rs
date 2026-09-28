//! Private decoder diagnostic: writes only into the explicitly supplied output directory.
use skate_audio_formats::{banks, eaac, eb};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let data = std::fs::read(&args[1])?;
    let archive = eb::Archive::parse(&data)?;
    let entry = archive
        .entries
        .iter()
        .find(|e| e.name.as_deref() == Some(&args[2]))
        .ok_or("bank missing")?;
    let bytes = &data[entry.range()];
    let bank = banks::Abk::parse(bytes)?;
    let bytes = &bytes[bank
        .sample_range(args[3].parse()?)
        .ok_or("sample missing")?];
    let header = eaac::Header::parse(bytes, 0)?;
    println!("{header:?}");
    let mut chunks = vec![Vec::new(); eaac::context_count(header.channels())];
    for block in eaac::blocks(&bytes[header.size()..])? {
        println!("block {block:?}");
        let payload = &bytes
            [header.size() + block.data_range().start..header.size() + block.data_range().end];
        for (i, c) in eaac::split_block(payload, chunks.len())?.iter().enumerate() {
            println!(
                "context {i} bytes {} field {:x} head {:02x?}",
                c.data.len(),
                c.field,
                &c.data[..8]
            );
            chunks[i].push(c.data.to_vec());
        }
    }
    let dir = std::path::Path::new(&args[4]);
    std::fs::create_dir_all(dir)?;
    for (i, c) in chunks.iter().enumerate() {
        let (data, align) = skate_data::audio::ffmpeg::pad_chain(c);
        for width in [1, 2] {
            std::fs::write(
                dir.join(format!("context-{i}-width-{width}.xma")),
                skate_data::audio::ffmpeg::riff_xma2(&data, width, header.sample_rate, align),
            )?;
        }
    }
    Ok(())
}
