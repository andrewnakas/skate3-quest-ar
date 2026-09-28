use bevy::prelude::*;
use serde::Deserialize;
use std::collections::BTreeMap;
#[derive(Default)]
pub(super) struct Clips(pub BTreeMap<String, Clip>);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    bone_names: Vec<String>,
    clips: BTreeMap<String, Clip>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Clip {
    fps: f32,
    frames: Vec<Vec<[f32; 16]>>,
}
impl Clips {
    pub fn load(
        root: &std::path::Path,
        definition: &skate_vehicles::VehicleDefinition,
        names: &[String],
    ) -> Result<Self, String> {
        let Some(path) = &definition.animations.file else {
            return Ok(Self::default());
        };
        let bytes = skate_mods::read_bounded(root, path, 16 * 1024 * 1024)?;
        let file: File = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if file.version != 1 || file.bone_names != names || file.clips.len() > 32 {
            return Err("Vehicle animation version/skeleton mismatch".into());
        }
        for clip in file.clips.values() {
            if !clip.fps.is_finite()
                || !(1. ..=120.).contains(&clip.fps)
                || clip.frames.is_empty()
                || clip.frames.len() > 3600
            {
                return Err("Invalid vehicle clip timing".into());
            }
            for frame in &clip.frames {
                if frame.len() != names.len() {
                    return Err("Vehicle clip missing bones".into());
                }
                for values in frame {
                    let m = Mat4::from_cols_array(values);
                    let (scale, rotation, translation) = m.to_scale_rotation_translation();
                    if !m.is_finite()
                        || m.determinant() <= 0.
                        || !rotation.is_finite()
                        || scale.min_element() < 0.9
                        || scale.max_element() > 1.1
                        || translation.abs().max_element() > 20.
                        || (m.w_axis.w - 1.).abs() > 0.001
                        || m.x_axis.w.abs() + m.y_axis.w.abs() + m.z_axis.w.abs() > 0.001
                    {
                        return Err("Invalid vehicle bone transform".into());
                    }
                }
            }
        }
        let a = &definition.animations;
        for name in [
            &a.enter,
            &a.exit,
            &a.drive,
            &a.idle,
            &a.reverse,
            &a.brake,
            &a.steer_left,
            &a.steer_right,
        ]
        .into_iter()
        .flatten()
        .chain(a.tricks.iter())
        {
            if !file.clips.contains_key(name) {
                return Err(format!("Missing vehicle animation {name}"));
            }
        }
        Ok(Self(file.clips))
    }
    pub fn duration(&self, name: Option<&String>) -> f32 {
        name.and_then(|n| self.0.get(n))
            .map_or(0., |c| c.frames.len() as f32 / c.fps)
    }
    pub fn pose(&self, name: Option<&String>, time: f32, looping: bool) -> Option<Vec<Mat4>> {
        let c = self.0.get(name?)?;
        let at = time * c.fps;
        let at = if looping {
            at % c.frames.len() as f32
        } else {
            at.min((c.frames.len() - 1) as f32)
        };
        let i = at.floor() as usize;
        let j = if looping {
            (i + 1) % c.frames.len()
        } else {
            (i + 1).min(c.frames.len() - 1)
        };
        Some(
            c.frames[i]
                .iter()
                .zip(&c.frames[j])
                .map(|(a, b)| {
                    let (sa, ra, ta) = Mat4::from_cols_array(a).to_scale_rotation_translation();
                    let (sb, rb, tb) = Mat4::from_cols_array(b).to_scale_rotation_translation();
                    Mat4::from_scale_rotation_translation(
                        sa.lerp(sb, at.fract()),
                        ra.slerp(rb, at.fract()),
                        ta.lerp(tb, at.fract()),
                    )
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vehicle_clips_validate_and_play_once_or_loop() {
        let root = std::env::temp_dir().join(format!("skate-vehicle-clips-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let a = Mat4::IDENTITY.to_cols_array();
        let b = Mat4::from_translation(Vec3::X).to_cols_array();
        let file = serde_json::json!({"version":1,"bone_names":["HIPS"],"clips":{"enter":{"fps":1,"frames":[[a],[b]]}}});
        std::fs::write(root.join("rider.json"), file.to_string()).unwrap();
        let mut definition = skate_vehicles::VehicleDefinition::default();
        definition.animations.file = Some("rider.json".into());
        definition.animations.enter = Some("enter".into());
        let clips = Clips::load(&root, &definition, &["HIPS".into()]).unwrap();
        let name = definition.animations.enter.as_ref();
        assert_eq!(clips.duration(name), 2.);
        assert!((clips.pose(name, 0.5, false).unwrap()[0].w_axis.x - 0.5).abs() < 0.001);
        assert_eq!(clips.pose(name, 3., false).unwrap()[0].w_axis.x, 1.);
        assert_eq!(clips.pose(name, 2., true).unwrap()[0].w_axis.x, 0.);
        assert!(Clips::load(&root, &definition, &["WRONG".into()]).is_err());
        definition.animations.drive = Some("missing".into());
        assert!(Clips::load(&root, &definition, &["HIPS".into()]).is_err());
        definition.animations.drive = None;
        let mut invalid = file;
        invalid["clips"]["enter"]["frames"][0][0][0] = serde_json::json!(0);
        std::fs::write(root.join("rider.json"), invalid.to_string()).unwrap();
        assert!(Clips::load(&root, &definition, &["HIPS".into()]).is_err());
        // Remove only this fixture's known file; do not recursively delete a computed directory.
        std::fs::remove_file(root.join("rider.json")).unwrap();
        let _ = std::fs::remove_dir(root);
    }
    #[test]
    fn missing_animation_uses_immediate_hidden_rider_fallback() {
        let clips = Clips::default();
        assert_eq!(clips.duration(None), 0.);
        assert!(clips.pose(None, 0., false).is_none());
    }
}
