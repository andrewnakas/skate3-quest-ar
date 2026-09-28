fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("Usage: check_mod path/to/mod.zip-or-folder")?;
    let manifest = skate_mods::validate_package(std::path::Path::new(&path))?;
    println!(
        "Valid package: {} {} (API {})",
        manifest.id, manifest.version, manifest.api
    );
    Ok(())
}
