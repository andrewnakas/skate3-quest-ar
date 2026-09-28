use serde::Deserialize;
use std::{
    fmt, fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameAssets {
    pub version: u32,
    pub character_scene: String,
    pub initial_animation: String,
    pub action_graph: String,
    pub motion_graph: String,
}

#[derive(Debug)]
pub struct AssetError(pub String);
impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for AssetError {}

impl GameAssets {
    pub fn load(root: &Path) -> Result<Self, AssetError> {
        let file = root.join("private/game.json");
        let json = fs::read_to_string(&file).map_err(|e| AssetError(format!("Cannot read {}: {e}. Run pipeline/Prepare-Assets.ps1 or select the correct --assets directory.", file.display())))?;
        let manifest = Self::parse(&json)?;
        for (kind, relative) in [
            ("Character scene", manifest.character_scene.as_str()),
            ("ActionGraph", manifest.action_graph.as_str()),
            ("MotionGraph", manifest.motion_graph.as_str()),
        ] {
            let path = root.join(relative);
            if !path.is_file() {
                return Err(AssetError(format!("{kind} missing: {}", path.display())));
            }
        }
        Ok(manifest)
    }

    pub fn parse(json: &str) -> Result<Self, AssetError> {
        let manifest: Self = serde_json::from_str(json)
            .map_err(|e| AssetError(format!("Invalid game asset manifest: {e}")))?;
        if manifest.version != 1 {
            return Err(AssetError(format!(
                "Unsupported asset manifest version {}",
                manifest.version
            )));
        }
        validate_path(&manifest.character_scene, "character_scene", "glb")?;
        validate_path(&manifest.action_graph, "action_graph", "stategraph")?;
        validate_path(&manifest.motion_graph, "motion_graph", "stategraph")?;
        if manifest.initial_animation.trim().is_empty() {
            return Err(AssetError(
                "initial_animation must name a stock animation".into(),
            ));
        }
        Ok(manifest)
    }
}

fn validate_path(value: &str, field: &str, extension: &str) -> Result<(), AssetError> {
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty()
        || !path.components().all(|c| matches!(c, Component::Normal(_)))
        || value.contains([':', '\\', '#'])
    {
        return Err(AssetError(format!(
            "{field} must be a relative asset path using forward slashes"
        )));
    }
    if path.extension().and_then(|x| x.to_str()) != Some(extension) {
        return Err(AssetError(format!("{field} must name a .{extension} file")));
    }
    Ok(())
}
