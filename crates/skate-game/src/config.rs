use bevy::prelude::Resource;
use std::path::PathBuf;

#[derive(Resource)]
pub(crate) struct Config {
    pub asset_root: PathBuf,
    pub verification_capture: Option<PathBuf>,
    pub map: Option<skate_data::skate_map::SkateMap>,
    pub map_path: Option<PathBuf>,
    pub difficulty: crate::difficulty::Difficulty,
    pub check_assets: bool,
    pub start_paused: bool,
    pub multiplayer: crate::multiplayer::Options,
    pub map_fingerprint: u64,
    pub teleport: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let mut config = Self {
            asset_root: crate::setup::asset_root()?,
            verification_capture: None,
            map: None,
            map_path: None,
            difficulty: crate::difficulty::Difficulty::default(),
            check_assets: false,
            start_paused: false,
            multiplayer: crate::multiplayer::Options::default(),
            map_fingerprint: 0,
            teleport: None,
        };
        let mut difficulty_override = None;
        let mut explicit_map = false;
        let mut args = std::env::args_os().skip(1);
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--trace" | "--trace-seconds" | "--trace-delay" | "--trace-min-us") => {
                    args.next().ok_or("Trace option requires a value")?;
                }
                Some("--trace-wait" | "--trace-gpu") => {}
                Some("--net-host") => {
                    config.multiplayer.host = Some(
                        args.next()
                            .ok_or("Missing host bind address")?
                            .to_string_lossy()
                            .parse()
                            .map_err(|_| "Invalid host bind address")?,
                    )
                }
                Some("--net-local") => {
                    let bind = args
                        .next()
                        .ok_or("--net-local requires bind and peer addresses")?
                        .to_string_lossy()
                        .parse()
                        .map_err(|_| "Invalid bind address")?;
                    let peer = args
                        .next()
                        .ok_or("--net-local requires peer address")?
                        .to_string_lossy()
                        .parse()
                        .map_err(|_| "Invalid peer address")?;
                    config.multiplayer.direct = Some((bind, peer));
                }
                Some("--net-session") => {
                    config.multiplayer.session = args
                        .next()
                        .ok_or("Missing session")?
                        .to_string_lossy()
                        .parse()
                        .map_err(|_| "Invalid session")?
                }
                Some("--spawn-offset") => {
                    let offset: f32 = args
                        .next()
                        .ok_or("Missing spawn offset")?
                        .to_string_lossy()
                        .parse()
                        .map_err(|_| "Invalid spawn offset")?;
                    if !offset.is_finite() || offset.abs() > 20. {
                        return Err("Spawn offset must be within 20 metres".into());
                    }
                    config.multiplayer.spawn_offset = offset;
                }
                Some("--player-title") => {
                    config.multiplayer.title =
                        Some(args.next().ok_or("Missing title")?.to_string_lossy().into())
                }
                Some("--appearance") => {
                    let value = args
                        .next()
                        .ok_or("Missing appearance")?
                        .to_string_lossy()
                        .into_owned();
                    if value.len() > 128 {
                        return Err("Appearance ID too long".into());
                    }
                    config.multiplayer.appearance = Some(value);
                }
                Some("--controller") => {
                    let slot: u32 = args
                        .next()
                        .ok_or("Missing controller index")?
                        .to_string_lossy()
                        .parse()
                        .map_err(|_| "Invalid controller index")?;
                    if slot > 3 {
                        return Err("Controller index must be 0 to 3".into());
                    }
                    config.multiplayer.controller = Some(slot);
                }
                Some("--assets") => {
                    config.asset_root = args.next().ok_or("--assets requires a directory")?.into()
                }
                Some("--map") => {
                    let path = PathBuf::from(args.next().ok_or("--map requires a .skate file")?);
                    config.map = Some(skate_data::skate_map::SkateMap::load(&path)?);
                    config.map_path = Some(path.canonicalize().map_err(|e| e.to_string())?);
                    explicit_map = true;
                }
                Some("--test-world") => {
                    explicit_map = true;
                    config.map = None;
                    config.map_path = None;
                }
                Some("--check-assets") => config.check_assets = true,
                Some("--start-paused") => config.start_paused = true,
                Some("--teleport") => {
                    config.teleport = Some(
                        args.next()
                            .ok_or("--teleport requires a destination ID")?
                            .to_string_lossy()
                            .into_owned(),
                    )
                }
                Some("--difficulty") => {
                    let value = args
                        .next()
                        .ok_or("--difficulty requires easy, normal or hardcore")?;
                    difficulty_override = Some(crate::difficulty::Difficulty::parse(
                        &value.to_string_lossy(),
                    )?);
                }
                Some("--verify") => {
                    config.verification_capture = Some(
                        args.next()
                            .ok_or("--verify requires an output PNG path")?
                            .into(),
                    )
                }
                _ => {
                    return Err(format!(
                        "Unknown argument {arg:?}. Usage: skate3rust [--assets DIRECTORY] [--map MAP.skate | --test-world] [--difficulty easy|normal|hardcore] [--verify CAPTURE.png] [--check-assets] [--start-paused]"
                    ));
                }
            }
        }
        config.asset_root = config
            .asset_root
            .canonicalize()
            .map_err(|e| format!("Asset root {}: {e}", config.asset_root.display()))?;
        config.difficulty = match difficulty_override {
            Some(mode) => mode,
            None => crate::difficulty::Difficulty::load(&config.asset_root)?,
        };
        if !explicit_map {
            if let Some(path) = crate::map_library::default_map(&config.asset_root)? {
                config.map = Some(skate_data::skate_map::SkateMap::load(&path)?);
                config.map_path = Some(path.canonicalize().map_err(|e| e.to_string())?);
            }
        }
        config.map_fingerprint = map_fingerprint(config.map_path.as_deref())?;
        if config.multiplayer.direct.is_some() && config.multiplayer.session == 0 {
            return Err("Direct multiplayer requires --net-session (a nonzero number shared by both players)".into());
        }
        if let Some(id) = &config.teleport {
            let destinations = crate::teleport_menu::load(&config.asset_root)?;
            let target = destinations
                .iter()
                .find(|d| &d.id == id)
                .ok_or("Unknown teleport destination")?;
            if target.matrix.is_none()
                || !config
                    .map_path
                    .as_ref()
                    .is_some_and(|p| crate::teleport_menu::same_map(p, &target.map))
            {
                return Err(
                    "Teleport destination is unavailable or belongs to a different map".into(),
                );
            }
        }
        if let Some(path) = &mut config.verification_capture {
            if path.extension().and_then(|x| x.to_str()) != Some("png") {
                return Err("--verify output must be a PNG file".into());
            }
            if !path.is_absolute() {
                *path = std::env::current_dir()
                    .map_err(|e| e.to_string())?
                    .join(&path);
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        Ok(config)
    }
}

/// Hash map bytes identically for startup and background map replacement.
pub(crate) fn map_fingerprint(path: Option<&std::path::Path>) -> Result<u64, String> {
    Ok(if let Some(path) = path {
        use std::io::Read;
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut hash = 0xcbf29ce484222325u64;
        let mut buffer = [0; 65536];
        loop {
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            for b in &buffer[..n] {
                hash = (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3);
            }
        }
        hash
    } else {
        skate_net::hash(b"skate-test-world-v1")
    })
}
