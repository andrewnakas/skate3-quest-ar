//! Retained APT depth list. Placement flags update individual properties;
//! omitted properties on a move retain their previous values.
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize)]
pub struct Control {
    pub label: Option<String>,
    pub type_name: String,
    #[serde(default)]
    pub flags: u32,
    #[serde(default)]
    pub depth: i32,
    pub character_id: Option<i32>,
    pub matrix: Option<[f32; 6]>,
    pub color_transform: Option<[u8; 8]>,
    pub ratio: Option<f32>,
    pub name: Option<String>,
    pub clip_depth: Option<i32>,
    pub blend_mode: Option<i32>,
    #[serde(default)]
    pub filter_pointer: u32,
    #[serde(default)]
    pub actions_offset: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub character: i32,
    pub matrix: [f32; 6],
    pub color: [u8; 8],
    pub ratio: f32,
    pub name: String,
    pub clip_depth: i32,
    pub blend_mode: i32,
}

#[derive(Clone, Debug, Default)]
pub struct DisplayList {
    pub depths: BTreeMap<i32, Placement>,
}
impl DisplayList {
    pub fn apply(&mut self, control: &Control) -> Result<(), String> {
        match control.type_name.as_str() {
            "remove_object2" | "remove_object3" => {
                self.depths.remove(&control.depth);
            }
            "place_object2" | "place_object3" => {
                if control.filter_pointer != 0 {
                    return Err("APT placement filter is not implemented".into());
                }
                if control.flags & 0x80 != 0 && control.actions_offset != 0 {
                    return Err("APT placement clip actions are not implemented".into());
                }
                let mut placement = if control.flags & 1 != 0 {
                    self.depths.get(&control.depth).cloned().ok_or_else(|| {
                        format!("APT move references empty depth {}", control.depth)
                    })?
                } else {
                    Placement {
                        character: control
                            .character_id
                            .filter(|_| control.flags & 2 != 0)
                            .ok_or("APT new placement lacks character")?,
                        matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                        color: [255, 255, 255, 255, 0, 0, 0, 0],
                        ratio: 0.0,
                        name: String::new(),
                        clip_depth: -1,
                        blend_mode: -1,
                    }
                };
                if control.flags & 2 != 0 {
                    placement.character = control
                        .character_id
                        .ok_or("APT character flag lacks value")?;
                }
                if control.flags & 4 != 0 {
                    placement.matrix = control.matrix.ok_or("APT matrix flag lacks value")?;
                }
                if control.flags & 8 != 0 {
                    placement.color = control
                        .color_transform
                        .ok_or("APT color flag lacks value")?;
                }
                if control.flags & 0x10 != 0 {
                    placement.ratio = control.ratio.ok_or("APT ratio flag lacks value")?;
                }
                if control.flags & 0x20 != 0 {
                    placement.name = control.name.clone().ok_or("APT name flag lacks value")?;
                }
                if control.flags & 0x40 != 0 {
                    placement.clip_depth = control
                        .clip_depth
                        .ok_or("APT clip-depth flag lacks value")?;
                }
                if control.type_name == "place_object3" {
                    placement.blend_mode = control.blend_mode.unwrap_or(-1);
                }
                self.depths.insert(control.depth, placement);
            }
            "do_action" | "do_init_action" | "frame_label" | "background_color" => {}
            kind => return Err(format!("Unsupported APT control {kind}")),
        }
        Ok(())
    }
}
