//! Skin stamps modify albedo before the unchanged StandardMaterial lighting.
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

pub(crate) type SkaterMaterial = ExtendedMaterial<StandardMaterial, SkinStamp>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub(crate) struct SkinStamp {
    #[texture(105)]
    #[sampler(106)]
    pub retail_mask: Option<Handle<Image>>,
    #[uniform(107)]
    #[reflect(ignore)]
    pub retail: crate::retail_character::CharacterParams,
    #[texture(100)]
    #[sampler(101)]
    pub texture: Option<Handle<Image>>,
    /// cac_hair_defaultPS samples opacity.r on TEXCOORD1, independently of diffuse alpha.
    #[texture(103)]
    #[sampler(104)]
    pub hair_opacity: Option<Handle<Image>>,
    /// Scale.xy, translation.zw in the authored secondary UV stream.
    #[uniform(102)]
    pub transform: Vec4,
    /// min.xy, max.zw of the selected arm/leg's stampable rectangle.
    #[uniform(102)]
    pub rectangle: Vec4,
    #[uniform(102)]
    pub enabled: Vec4,
}
impl MaterialExtension for SkinStamp {
    fn fragment_shader() -> ShaderRef {
        AssetPath::from(embedded_path!("customiser_stamp.wgsl"))
            .with_source("embedded")
            .into()
    }
    fn prepass_fragment_shader() -> ShaderRef {
        AssetPath::from(embedded_path!("customiser_prepass.wgsl"))
            .with_source("embedded")
            .into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        Self::fragment_shader()
    }
}
pub(crate) fn register(app: &mut App) {
    embedded_asset!(app, "customiser_stamp.wgsl");
    embedded_asset!(app, "customiser_prepass.wgsl");
    app.add_plugins(MaterialPlugin::<SkaterMaterial>::default());
}

pub(crate) fn placement(target: [f32; 4], art: [f32; 4]) -> (Vec4, Vec4) {
    // Retail rectangles are [top, bottom, left, right], with V increasing
    // upwards. Texture pixels and glTF use downward V. Artwork pixel bounds
    // and both limb meshes corroborate this conversion.
    let target_min = Vec2::new(target[2], 1. - target[0]);
    let target_max = Vec2::new(target[3], 1. - target[1]);
    let art_min = Vec2::new(art[2], 1. - art[0]);
    let art_max = Vec2::new(art[3], 1. - art[1]);
    let scale = (art_max - art_min) / (target_max - target_min);
    let offset = art_min - target_min * scale;
    (
        Vec4::new(scale.x, scale.y, offset.x, offset.y),
        Vec4::new(target_min.x, target_min.y, target_max.x, target_max.y),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_shader_paths_match_this_binary() {
        for (shader, registered) in [
            (
                SkinStamp::fragment_shader(),
                embedded_path!("customiser_stamp.wgsl"),
            ),
            (
                SkinStamp::prepass_fragment_shader(),
                embedded_path!("customiser_prepass.wgsl"),
            ),
        ] {
            let ShaderRef::Path(path) = shader else {
                panic!("Expected embedded shader path")
            };
            assert_eq!(path, AssetPath::from(registered).with_source("embedded"));
        }
    }
    #[test]
    fn customiser_stamp_maps_authored_borders() {
        let (t, r) = placement([0.5, 0.0, 0.1, 0.4], [0.9, 0.1, 0.2, 0.8]);
        assert!((Vec2::new(r.x, r.y) * t.xy() + t.zw()).abs_diff_eq(Vec2::new(0.2, 0.1), 1e-6));
        assert!((Vec2::new(r.z, r.w) * t.xy() + t.zw()).abs_diff_eq(Vec2::new(0.8, 0.9), 1e-6));
        assert_eq!(
            bevy::asset::embedded_path!("customiser_stamp.wgsl"),
            std::path::PathBuf::from(format!(
                "{}/customiser_stamp.wgsl",
                module_path!().split("::").next().unwrap()
            ))
        );
    }
    #[test]
    fn customiser_shader_compiles_offline() {
        use bevy::render::render_resource::{DownlevelFlags, WgpuFeatures};
        use bevy::shader::{ShaderCache, ShaderDefVal};
        let Ok(path) = std::env::var("SKATE_CAC_TEST_BEVY_SOURCES") else {
            return;
        };
        fn walk(path: &std::path::Path, result: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(path).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, result);
                } else if p.extension().is_some_and(|e| e == "wgsl") {
                    result.push(p);
                }
            }
        }
        let mut files = vec![];
        for entry in std::fs::read_dir(path).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("bevy_") && name.ends_with("-0.18.1") {
                walk(&entry.path().join("src"), &mut files);
            }
        }
        let mut assets = Assets::<Shader>::default();
        let mut cache =
            ShaderCache::<(), ()>::new(WgpuFeatures::empty(), DownlevelFlags::all(), |_, _, _| {
                Ok(())
            });
        let defs: Vec<_> = [
            ("MAX_DIRECTIONAL_LIGHTS", 10),
            ("MAX_CASCADES_PER_LIGHT", 4),
            ("MAX_VIEW_LIGHT_PROBES", 8),
            ("PER_OBJECT_BUFFER_BATCH_SIZE", 64),
            ("MATERIAL_BIND_GROUP", 3),
            ("AVAILABLE_STORAGE_BUFFER_BINDINGS", 8),
        ]
        .into_iter()
        .map(|(n, v)| ShaderDefVal::UInt(n.into(), v))
        .collect();
        for path in files {
            let source = std::fs::read_to_string(&path).unwrap();
            if !source.contains("#define_import_path") {
                continue;
            }
            let shader = Shader::from_wgsl_with_defs(
                source,
                path.to_string_lossy().to_string(),
                defs.clone(),
            );
            let h = assets.add(shader.clone());
            cache.set_shader(h.id(), shader);
        }
        for (source, path, prepass) in [
            (
                include_str!("retail_character_common.wgsl"),
                "retail_character_common.wgsl",
                false,
            ),
            (
                include_str!("retail_character.wgsl"),
                "retail_character.wgsl",
                false,
            ),
            (
                include_str!("customiser_stamp.wgsl"),
                "customiser_stamp.wgsl",
                false,
            ),
            (
                include_str!("customiser_prepass.wgsl"),
                "customiser_prepass.wgsl",
                true,
            ),
        ] {
            let shader = Shader::from_wgsl(source, path);
            let h = assets.add(shader.clone());
            cache.set_shader(h.id(), shader);
            for secondary_uv in [false, true] {
                let mut d = defs.clone();
                for name in [
                    "VERTEX_UVS",
                    "MAY_DISCARD",
                    "VERTEX_UVS_A",
                    "VERTEX_OUTPUT_INSTANCE_INDEX",
                    "VERTEX_NORMALS",
                    "MESH_PIPELINE",
                    "STANDARD_MATERIAL",
                    "STANDARD_MATERIAL_BASE_COLOR_TEXTURE",
                ] {
                    d.push(ShaderDefVal::Bool(name.into(), true));
                }
                if secondary_uv {
                    d.push(ShaderDefVal::Bool("VERTEX_UVS_B".into(), true));
                }
                if prepass {
                    d.push(ShaderDefVal::Bool("PREPASS_PIPELINE".into(), true));
                }
                cache
                    .get(&(), 0, h.id(), &d)
                    .unwrap_or_else(|e| panic!("{path} secondary UV {secondary_uv}: {e:?}"));
                if prepass {
                    for name in [
                        "PREPASS_FRAGMENT",
                        "NORMAL_PREPASS",
                        "NORMAL_PREPASS_OR_DEFERRED_PREPASS",
                    ] {
                        d.push(ShaderDefVal::Bool(name.into(), true));
                    }
                    cache
                        .get(&(), 0, h.id(), &d)
                        .unwrap_or_else(|e| panic!("Normal {path} UV {secondary_uv}: {e:?}"));
                }
            }
        }
    }
}
