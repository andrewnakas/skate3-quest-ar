//! Immutable stock shot definitions. Field order/units follow TU382CA4660.
use skate_core::camera::{Shot, ShotDatabase, ShotDefinition};
use skate_data::collections::Collections;
use std::collections::BTreeMap;

const CLASS: &str = "camera_shots";
const CLASS_HASH: u64 = 0xf27dd93e059ef6cb;
const RADIANS: f32 = f32::from_bits(0x3c8efa35);

pub(crate) struct StockShots(BTreeMap<String, ShotDefinition>);

impl StockShots {
    pub fn from_collections(data: &Collections) -> Result<Self, String> {
        let mut names = BTreeMap::new();
        for entry in data.entries().iter().filter(|c| c.class_name == CLASS) {
            let text = data.field(CLASS, &entry.key, "CollectionName")?;
            if text.type_name != "EA::Reflection::Text" {
                return Err(format!("Invalid camera collection name {}", entry.key));
            }
            if names
                .insert(
                    super::stock_names::lookup8(entry.key.as_bytes()),
                    text.data.clone(),
                )
                .is_some()
            {
                return Err("Duplicate camera collection hash".into());
            }
        }
        let mut definitions = BTreeMap::new();
        for entry in data.entries().iter().filter(|c| c.class_name == CLASS) {
            let key = entry.key.as_str();
            let float = |field: &str| data.float(CLASS, key, field);
            let enumeration =
                |field| -> Result<u32, String> { Ok(data.words::<1>(CLASS, key, field)?[0]) };
            let boolean =
                |field| -> Result<u8, String> { Ok(u8::from(data.boolean(CLASS, key, field)?)) };
            let floats = |fields: &[&str]| -> Result<Vec<f32>, String> {
                fields.iter().map(|field| float(field)).collect()
            };
            let child = |field| -> Result<Option<String>, String> {
                let words = data.words::<6>(CLASS, key, field)?;
                let class = (u64::from(words[0]) << 32) | u64::from(words[1]);
                let hash = (u64::from(words[2]) << 32) | u64::from(words[3]);
                if hash == 0 {
                    return Ok(None);
                }
                if class != CLASS_HASH {
                    return Err(format!("Non-shot reference {key}/{field}"));
                }
                let name = names
                    .get(&hash)
                    .ok_or_else(|| format!("Missing camera child {key}/{field}: {hash:016X}"))?;
                Ok((!name.is_empty()).then(|| name.to_ascii_lowercase()))
            };
            let shot = Shot {
                distance: float("PositionDistance")?,
                lens_length: float("FramingLensLength")?,
                smoothing: floats(&[
                    "SmoothingDirection",
                    "SmoothingElevation",
                    "SmoothingYaw",
                    "SmoothingPitch",
                ])?
                .try_into()
                .unwrap(),
                reference_weights: floats(&[
                    "ReferenceWeightHead",
                    "ReferenceWeightHips",
                    "ReferenceWeightGrind",
                    "ReferenceWeightCOM",
                    "ReferenceWeightBoard",
                    "ReferenceWeightTrajectory",
                    "ReferenceWeightAnchor",
                    "ReferenceWeightFeet",
                    "ReferenceWeightDampedCOM",
                    "Hash_D70C95EDC16A7DFC",
                ])?
                .try_into()
                .unwrap(),
                board_offset: float("ReferenceBoardOffset")?,
                position_heading: float("PositionHeading")? * RADIANS,
                position_elevation: float("PositionElevation")? * RADIANS,
                framing: [
                    float("FramingRoll")? * RADIANS,
                    float("FramingYaw")? * RADIANS,
                    float("FramingPitch")? * RADIANS,
                ],
                follow_subject_in_air: boolean("OptionFollowSubjectInAir")?,
                mirror_for_stance: boolean("OptionMirrorForStance")?,
                snap_to_reference_point: boolean("OptionSnapToReferencePoint")?,
                use_previous_shot: boolean("OptionUsePreviousShot")?,
                use_drop_predictor: boolean("OptionUseDropPredictor")?,
                use_free_camera_stick: boolean("OptionUseFreeCamStick")?,
                // The sole dynamic attribute has the native missing-value zero.
                // All layout fields above are required in the converted data.
                avoidance_override: if has_field(data, key, "OptionAvoidanceOverride")? {
                    boolean("OptionAvoidanceOverride")?
                } else {
                    0
                },
                blur: float("FXBlurShot")?,
                transition_blur: float("FXBlurTransition")?,
                subject_opacity: float("FXSubjectOpacity")?,
                collision_hint: enumeration("OptionCollisionType")?,
                anchor: enumeration("OptionAnchor")?,
                compass_north: enumeration("OptionCompassNorth")?,
                world_heading: float("WorldHeading")? * RADIANS,
                ..Shot::new()
            };
            let definition = ShotDefinition {
                name: key.to_ascii_lowercase(),
                shot_type: enumeration("ShotType")?,
                shot,
                transition_time: float("TransitionTime")?,
                transition_units: enumeration("TransitionTimeUnits")?,
                children: [
                    child("BlendShot1")?,
                    child("BlendShot2")?,
                    child("BlendShot3")?,
                ],
                blend_points: [
                    float("BlendPoint1")?,
                    float("BlendPoint2")?,
                    float("BlendPoint3")?,
                ],
                blend_value: float("BlendValue")?,
                blend_type: enumeration("BlendType")?,
                blend_smoothing: float("SmoothingBlendValue")?,
            };
            if definitions
                .insert(definition.name.clone(), definition)
                .is_some()
            {
                return Err(format!("Duplicate lowercase camera shot {key}"));
            }
        }
        for definition in definitions.values() {
            for name in definition.children.iter().flatten() {
                if !definitions.contains_key(name) {
                    return Err(format!(
                        "Camera shot {} references missing named shot {name}",
                        definition.name
                    ));
                }
            }
        }
        Ok(Self(definitions))
    }
}

impl ShotDatabase for StockShots {
    fn load(&self, name: &str) -> Result<ShotDefinition, String> {
        self.0
            .get(&name.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| format!("Missing stock camera shot {name}"))
    }
}

fn has_field(data: &Collections, key: &str, name: &str) -> Result<bool, String> {
    let mut current = key;
    for _ in 0..=data.entries().len() {
        let entry = data
            .entries()
            .iter()
            .find(|c| c.class_name == CLASS && c.key == current)
            .ok_or_else(|| format!("Missing camera collection {current}"))?;
        if entry.fields.contains_key(name) {
            return Ok(true);
        }
        if entry.parent.is_empty() {
            return Ok(false);
        }
        current = &entry.parent;
    }
    Err(format!("Cyclic camera collection inheritance {key}"))
}
