//! SkateAptString82CA14D0 and cFont828076D0/82808708 layout.
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Deserialize)]
pub struct Glyph {
    pub glyph_index: usize,
    pub width: f32,
    pub height: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    pub x_advance: f32,
    pub atlas_bounds: [f32; 4],
}
#[derive(Clone)]
pub struct Font {
    /// 825D6B68 attaches futuraheavy to Futura Shadow; 82CA1FD8 draws both.
    pub foreground: Option<i32>,
    pub texture: String,
    pub size: [u32; 2],
    pub scale: [f32; 2],
    pub offset: [f32; 2],
    pub ascent: f32,
    pub glyphs: BTreeMap<u32, Glyph>,
}
#[derive(Clone, Default)]
pub struct TextAssets {
    pub fonts: BTreeMap<i32, Font>,
    pub language: BTreeMap<String, String>,
}
impl TextAssets {
    pub fn load(json: &serde_json::Value) -> Result<Self, String> {
        let mut out = Self::default();
        out.language =
            serde_json::from_value(json["language"].clone()).map_err(|e| e.to_string())?;
        for c in json["characters"]
            .as_array()
            .ok_or("HUD characters missing")?
        {
            if c["type_name"] != "font" {
                continue;
            }
            let name = c["font"]["name"].as_str().ok_or("HUD font name missing")?;
            let asset = &json["fonts"][name];
            let d = &asset["definition"];
            let glyphs: Vec<Glyph> =
                serde_json::from_value(d["glyphs"].clone()).map_err(|e| e.to_string())?;
            let layout = &json["font_mappings"][name]["native_layout"];
            let f = |key: &str| {
                layout[key]
                    .as_f64()
                    .map(|v| v as f32)
                    .ok_or_else(|| format!("Missing native font {name}.{key}"))
            };
            let mut font = Font {
                foreground: None,
                texture: asset["texture"]
                    .as_str()
                    .ok_or("Font texture missing")?
                    .into(),
                size: [
                    d["textures"][0]["width"]
                        .as_u64()
                        .ok_or("Font width missing")? as u32,
                    d["textures"][0]["height"]
                        .as_u64()
                        .ok_or("Font height missing")? as u32,
                ],
                scale: [f("ScaleX")?, f("ScaleY")?],
                offset: [f("OffsetX")?, f("OffsetY")?],
                ascent: d["metrics"]["Ascent"]
                    .as_f64()
                    .ok_or("Font ascent missing")? as f32,
                glyphs: BTreeMap::new(),
            };
            for mapping in d["characters"]
                .as_array()
                .ok_or("Font character map missing")?
            {
                let index = mapping["glyph_index"]
                    .as_u64()
                    .ok_or("Invalid font glyph index")? as usize;
                let glyph = glyphs
                    .iter()
                    .find(|g| g.glyph_index == index)
                    .ok_or("Absent mapped glyph")?;
                font.glyphs.insert(
                    mapping["codepoint"]
                        .as_u64()
                        .ok_or("Invalid font codepoint")? as u32,
                    glyph.clone(),
                );
            }
            out.fonts.insert(
                c["id"].as_i64().ok_or("Invalid font character id")? as i32,
                font,
            );
        }
        for c in json["characters"].as_array().unwrap() {
            if c["type_name"] == "font" && c["font"]["name"] == "Futura Shadow" {
                let foreground = json["characters"].as_array().unwrap().iter().find(|f|
                    f["type_name"] == "font" && json["font_mappings"][f["font"]["name"].as_str().unwrap_or("")]["file_name"] == "futuraheavy")
                    .and_then(|f| f["id"].as_i64()).ok_or("Missing native Futura Shadow foreground font")? as i32;
                out.fonts
                    .get_mut(&(c["id"].as_i64().unwrap() as i32))
                    .unwrap()
                    .foreground = Some(foreground);
            }
        }
        Ok(out)
    }
    /// A leading `#` means *resolve every token*, not "leave this alone".
    ///
    /// The trick display's name is composed by `sub_825E51A0`, which formats with the
    /// literals `"#%s %d "` (8220E43C) and `"#%s "` (8220E444) and then `strcat`s further
    /// localisation ids onto the result. So a displayed name is one `#` followed by
    /// space-separated ids and bare numbers -- `#ID_TRICK_FLIP_KICKFLIP 360
    /// ID_TRICK_AIR_METRICS_FRONTFLIP` -- and each token is resolved separately. Treating
    /// `#` as "literal" is the inverse of that, and left every composed name unreadable.
    ///
    /// A token with no entry, such as a spin's `360`, is kept exactly as written.
    pub fn localize(&self, text: &str) -> String {
        let Some(composed) = text.strip_prefix('#') else {
            return self
                .language
                .get(text)
                .cloned()
                .unwrap_or_else(|| text.into());
        };
        composed
            .split_ascii_whitespace()
            .map(|token| {
                self.language
                    .get(token)
                    .map_or(token, String::as_str)
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}
impl Font {
    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs
            .get(&(c as u32))
            .or_else(|| self.glyphs.get(&65535))
    }
    pub fn width(&self, text: &str, height: f32) -> f32 {
        text.chars()
            .filter_map(|c| self.glyph(c))
            .fold(0.0, |x, g| g.x_advance.mul_add(self.scale[0] * height, x))
    }
}
