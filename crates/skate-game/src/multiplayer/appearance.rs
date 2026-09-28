//! Appearance identities reference prepared retail assets; only imported GLBs travel online.
use super::{
    Multiplayer,
    appearance_transfer::{Exchange, MAX_BLOB},
};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::OnceLock,
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) enum Look {
    Stock,
    Outfit(Value),
    Native(String),
    Imported(String),
}
#[derive(Component)]
pub(crate) struct RemoteCharacter;
#[derive(Resource, Default)]
pub(super) struct Appearances {
    exchange: Exchange,
    connection: Option<(u64, u64)>,
    local: String,
    pub looks: BTreeMap<u64, ([u8; 32], Look)>,
    seen: BTreeMap<u64, [u8; 32]>,
    cached: BTreeMap<[u8; 32], Look>,
    pub status: String,
    pub progress: String,
}
pub(super) fn cleanup(mut exit: MessageReader<AppExit>) {
    // Generated process-private directory, never the personal import library.
    if exit.read().next().is_some() {
        let _ = std::fs::remove_dir_all(cache_directory());
    }
}
pub(crate) fn cache_directory() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| std::env::temp_dir().join(format!("skate-online-{:016x}", super::unique())))
}
pub(super) fn sync(
    mut state: ResMut<Appearances>,
    mut net: ResMut<Multiplayer>,
    models: Res<crate::custom_models::CustomModels>,
    parts: Res<crate::customiser_parts::Parts>,
) {
    let Some(lobby) = net.lobby.as_mut() else {
        if state.connection.is_some() {
            let _ = std::fs::remove_dir_all(cache_directory());
            *state = default();
        }
        return;
    };
    let connection = (lobby.session, lobby.local);
    if state.connection != Some(connection) {
        *state = Appearances {
            connection: Some(connection),
            ..default()
        };
    }
    let selection = models.online_selection();
    let signature = models
        .active
        .as_ref()
        .map(|id| format!("model:{id}"))
        .unwrap_or_else(|| format!("outfit:{}", parts.applied));
    if state.local != signature {
        let payload = (|| -> Result<Vec<u8>, String> {
            if let Some((native, path)) = selection {
                if let Some(key) = native {
                    return serde_json::to_vec(&Look::Native(key)).map_err(|e| e.to_string());
                }
                if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() as usize >= MAX_BLOB {
                    return Err("Imported character exceeds the 64 MiB online limit".into());
                }
                let glb = std::fs::read(path).map_err(|e| e.to_string())?;
                validate_glb(&glb)?;
                let mut bytes = vec![0];
                bytes.extend(glb);
                Ok(bytes)
            } else {
                let look = if parts.applied["selections"]
                    .as_object()
                    .is_some_and(|s| !s.is_empty())
                {
                    Look::Outfit(parts.applied.clone())
                } else {
                    Look::Stock
                };
                serde_json::to_vec(&look).map_err(|e| e.to_string())
            }
        })();
        match payload {
            Ok(bytes) => {
                info!(
                    "ONLINE_CHARACTER_PUBLISH bytes={} selection={signature}",
                    bytes.len()
                );
                state.exchange.publish(bytes);
                state.status.clear();
            }
            Err(e) => {
                state.status = e;
                state
                    .exchange
                    .publish(serde_json::to_vec(&Look::Stock).unwrap());
            }
        }
        state.local = signature;
    }
    let now = net.started.elapsed().as_millis() as u64;
    let lobby = net.lobby.as_mut().unwrap();
    state.exchange.tick(lobby, now);
    state.progress = state.exchange.progress();
    state.looks.retain(|id, _| lobby.actors.contains_key(id));
    state.seen.retain(|id, _| lobby.actors.contains_key(id));
    let updates: Vec<_> = state
        .exchange
        .ready
        .iter()
        .filter(|(id, (key, _))| state.seen.get(id) != Some(&key.hash))
        .map(|(&id, (key, _))| (id, key.hash))
        .collect();
    for (id, hash) in updates {
        let bytes = std::mem::take(&mut state.exchange.ready.get_mut(&id).unwrap().1);
        state.seen.insert(id, hash);
        let result = (|| -> Result<Look, String> {
            if bytes.is_empty() {
                return state
                    .cached
                    .get(&hash)
                    .cloned()
                    .ok_or("Cached appearance missing".into());
            }
            if bytes.first() == Some(&0) {
                validate_glb(&bytes[1..])?;
                let name = format!("{}.glb", blake3::Hash::from_bytes(hash).to_hex());
                std::fs::create_dir_all(cache_directory()).map_err(|e| e.to_string())?;
                let path = cache_directory().join(&name);
                if !path.exists() {
                    let pending = path.with_extension("tmp");
                    std::fs::write(&pending, &bytes[1..]).map_err(|e| e.to_string())?;
                    std::fs::rename(pending, path).map_err(|e| e.to_string())?;
                }
                Ok(Look::Imported(format!("online-characters://{name}")))
            } else {
                if bytes.len() > 32768 {
                    return Err("Oversized outfit".into());
                }
                let look: Look = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                match &look {
                    Look::Imported(_) => return Err("Peer supplied an asset path".into()),
                    Look::Native(key) if key.len() > 96 => {
                        return Err("Invalid native identity".into());
                    }
                    _ => (),
                }
                Ok(look)
            }
        })();
        match result {
            Ok(look) => {
                let identity = state.exchange.ready[&id].0.clone();
                state.exchange.remember(identity);
                state.cached.insert(hash, look.clone());
                info!(
                    "ONLINE_CHARACTER_READY peer={id} kind={} hash={}",
                    if matches!(look, Look::Imported(_)) {
                        "import"
                    } else {
                        "retail"
                    },
                    blake3::Hash::from_bytes(hash)
                );
                state.looks.insert(id, (hash, look));
            }
            Err(e) => warn!("Remote appearance {id}: {e}"),
        }
    }
}
/// Online imports are self-contained data. Never let peer JSON load a URI or path.
pub(super) fn validate_glb(bytes: &[u8]) -> Result<(), String> {
    let bad = || "Invalid or unsupported online character GLB".to_owned();
    let word = |offset: usize| {
        bytes
            .get(offset..offset + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
    };
    if bytes.len() < 28
        || bytes.len() >= MAX_BLOB
        || &bytes[..4] != b"glTF"
        || word(4) != Some(2)
        || word(8) != Some(bytes.len() as u32)
        || word(16) != Some(0x4e4f534a)
    {
        return Err(bad());
    }
    let length = word(12).ok_or_else(bad)? as usize;
    if length > 8 * 1024 * 1024 {
        return Err(bad());
    }
    let json: Value =
        serde_json::from_slice(bytes.get(20..20 + length).ok_or_else(bad)?).map_err(|_| bad())?;
    fn external(v: &Value) -> bool {
        match v {
            Value::Object(m) => {
                m.contains_key("uri")
                    || m.iter().any(|(k, v)| {
                        k == "extensionsRequired" && v.as_array().is_some_and(|a| !a.is_empty())
                            || external(v)
                    })
            }
            Value::Array(a) => a.iter().any(external),
            _ => false,
        }
    }
    if external(&json) {
        return Err("Online GLBs must embed all buffers and textures and use standard glTF".into());
    }
    let binary = 20 + length;
    if word(binary + 4) != Some(0x004e4942)
        || word(binary).is_none_or(|n| binary + 8 + n as usize != bytes.len())
    {
        return Err(bad());
    }
    let bin = &bytes[binary + 8..];
    if json["buffers"].as_array().is_none_or(|a| a.len() != 1)
        || json["buffers"][0]["byteLength"]
            .as_u64()
            .is_none_or(|n| n > bin.len() as u64)
    {
        return Err(bad());
    }
    for (key, max) in [
        ("nodes", 4096),
        ("meshes", 256),
        ("skins", 128),
        ("images", 128),
        ("materials", 256),
        ("accessors", 8192),
        ("bufferViews", 8192),
    ] {
        if json[key].as_array().is_some_and(|a| a.len() > max) {
            return Err(bad());
        }
    }
    for a in json["accessors"].as_array().into_iter().flatten() {
        if a["count"].as_u64().is_none_or(|n| n > 2_000_000) {
            return Err(bad());
        }
    }
    for v in json["bufferViews"].as_array().into_iter().flatten() {
        let offset = v["byteOffset"].as_u64().unwrap_or(0);
        let size = v["byteLength"].as_u64().ok_or_else(bad)?;
        if v["buffer"].as_u64() != Some(0) || offset.saturating_add(size) > bin.len() as u64 {
            return Err(bad());
        }
    }
    let mut pixels = 0u64;
    for image in json["images"].as_array().into_iter().flatten() {
        let i = image["bufferView"].as_u64().ok_or_else(bad)? as usize;
        let view = json["bufferViews"]
            .as_array()
            .and_then(|a| a.get(i))
            .ok_or_else(bad)?;
        let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let size = view["byteLength"].as_u64().ok_or_else(bad)? as usize;
        let reader = image::ImageReader::new(std::io::Cursor::new(&bin[offset..offset + size]))
            .with_guessed_format()
            .map_err(|_| bad())?;
        if !matches!(
            reader.format(),
            Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
        ) {
            return Err(bad());
        }
        let (w, h) = reader.into_dimensions().map_err(|_| bad())?;
        pixels = pixels.saturating_add(w as u64 * h as u64);
        if w == 0 || h == 0 || w > 8192 || h > 8192 || pixels > 64 * 1024 * 1024 {
            return Err("Online character textures exceed the memory limit".into());
        }
    }
    let nodes = json["nodes"].as_array().ok_or_else(bad)?;
    let mut parents = vec![false; nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        for c in node["children"].as_array().into_iter().flatten() {
            let c = c.as_u64().ok_or_else(bad)? as usize;
            if c >= nodes.len() || c == i || parents[c] {
                return Err(bad());
            }
            parents[c] = true;
        }
    }
    fn visit(i: usize, nodes: &[Value], seen: &mut [u8], depth: usize) -> bool {
        if depth > 128 || seen[i] == 1 {
            return false;
        }
        if seen[i] == 2 {
            return true;
        }
        seen[i] = 1;
        for c in nodes[i]["children"].as_array().into_iter().flatten() {
            if !visit(c.as_u64().unwrap() as usize, nodes, seen, depth + 1) {
                return false;
            }
        }
        seen[i] = 2;
        true
    }
    let mut seen = vec![0; nodes.len()];
    for i in 0..nodes.len() {
        if !visit(i, nodes, &mut seen, 0) {
            return Err(bad());
        }
    }
    if json["skins"].as_array().is_none_or(|s| s.is_empty()) {
        return Err("Online character has no skin".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn glb(json: Value) -> Vec<u8> {
        let mut j = serde_json::to_vec(&json).unwrap();
        while j.len() % 4 != 0 {
            j.push(b' ');
        }
        let mut b = b"glTF".to_vec();
        b.extend(2u32.to_le_bytes());
        b.extend((28u32 + j.len() as u32).to_le_bytes());
        b.extend((j.len() as u32).to_le_bytes());
        b.extend(0x4e4f534au32.to_le_bytes());
        b.extend(j);
        b.extend(0u32.to_le_bytes());
        b.extend(0x004e4942u32.to_le_bytes());
        b
    }
    #[test]
    fn online_appearance_glb_rejects_paths_cycles_and_bad_ranges() {
        let base = serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":0}],"nodes":[{}],"skins":[{"joints":[0]}]});
        assert!(validate_glb(&glb(base.clone())).is_ok());
        let mut external = base.clone();
        external["images"] = serde_json::json!([{"uri":"../../private/file.png"}]);
        assert!(validate_glb(&glb(external)).is_err());
        let mut cyclic = base.clone();
        cyclic["nodes"] = serde_json::json!([{"children":[1]},{"children":[0]}]);
        assert!(validate_glb(&glb(cyclic)).is_err());
        let mut range = base.clone();
        range["bufferViews"] = serde_json::json!([{"buffer":0,"byteLength":99999}]);
        assert!(validate_glb(&glb(range)).is_err());
        let mut bytes = glb(base);
        bytes[8] = 0;
        assert!(validate_glb(&bytes).is_err());
    }
    #[test]
    #[ignore = "requires an owned imported GLB via SKATE_ONLINE_TEST_GLB"]
    fn online_appearance_accepts_owned_import() {
        let path = std::env::var("SKATE_ONLINE_TEST_GLB").unwrap();
        validate_glb(&std::fs::read(path).unwrap()).unwrap();
    }
}
