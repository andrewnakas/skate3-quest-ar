//! Typed access to converted stock skater XML values, without engine objects.
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Deserialize)]
pub struct Field {
    #[serde(rename = "type")]
    pub type_name: String,
    pub data: String,
    /// Array attributes (`vlt.py` `array_items`): `data` holds only the header lanes, the
    /// elements are here, one hex string per item.
    #[serde(default)]
    pub array: Option<FieldArray>,
}

#[derive(Debug, Deserialize)]
pub struct FieldArray {
    pub capacity: u32,
    pub element_size: u32,
    pub alignment: u32,
    pub items: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Collection {
    #[serde(rename = "class")]
    pub class_name: String,
    pub key: String,
    pub parent: String,
    pub fields: BTreeMap<String, Field>,
    pub source: String,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
pub struct Collections {
    version: u32,
    collections: Vec<Collection>,
}

impl Collections {
    pub fn entries(&self) -> &[Collection] {
        &self.collections
    }

    pub fn load(asset_root: &Path) -> Result<Self, String> {
        let path = asset_root.join("private/stock/skater-collections.json");
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let data: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if data.version != 1 {
            return Err(format!(
                "Unsupported skater collections version {}",
                data.version
            ));
        }
        let mut identities = std::collections::BTreeSet::new();
        for item in &data.collections {
            if !identities.insert((
                crate::attrib_hash::numeric_name(&item.class_name),
                crate::attrib_hash::numeric_name(&item.key),
            )) {
                return Err(format!(
                    "Duplicate collection {}/{}",
                    item.class_name, item.key
                ));
            }
        }
        Ok(data)
    }

    pub fn field(&self, class: &str, key: &str, name: &str) -> Result<&Field, String> {
        // Converted vaults can retain numeric identities when debug names were
        // unavailable. Both spellings identify the same native AttribSys entry.
        let class_id = crate::attrib_hash::numeric_name(class);
        let field_id = crate::attrib_hash::numeric_name(name);
        let mut current = key;
        for _ in 0..=self.collections.len() {
            let current_id = crate::attrib_hash::numeric_name(current);
            let item = self
                .collections
                .iter()
                .find(|c| {
                    (c.class_name == class
                        || crate::attrib_hash::numeric_name(&c.class_name) == class_id)
                        && (c.key == current
                            || crate::attrib_hash::numeric_name(&c.key) == current_id)
                })
                .ok_or_else(|| format!("Missing stock collection {class}/{current}"))?;
            if let Some(field) = item.fields.get(name).or_else(|| {
                item.fields
                    .iter()
                    .find(|(key, _)| crate::attrib_hash::numeric_name(key) == field_id)
                    .map(|(_, field)| field)
            }) {
                return Ok(field);
            }
            if item.parent.is_empty() {
                return Err(format!("Missing stock field {class}/{key}/{name}"));
            }
            current = &item.parent;
        }
        Err(format!("Cyclic stock collection inheritance {class}/{key}"))
    }

    pub fn float(&self, class: &str, key: &str, name: &str) -> Result<f32, String> {
        let field = self.field(class, key, name)?;
        if field.type_name != "EA::Reflection::Float" {
            return Err(format!("Expected float at {class}/{key}/{name}"));
        }
        let words = decode_words::<1>(&field.data)?;
        let value = f32::from_bits(words[0]);
        if !value.is_finite() {
            return Err(format!("Non-finite stock float {class}/{key}/{name}"));
        }
        Ok(value)
    }

    pub fn integer(&self, class: &str, key: &str, name: &str) -> Result<u32, String> {
        let field = self.field(class, key, name)?;
        if !matches!(
            field.type_name.as_str(),
            "EA::Reflection::Int32" | "EA::Reflection::UInt32"
        ) {
            return Err(format!("Expected integer at {class}/{key}/{name}"));
        }
        Ok(decode_words::<1>(&field.data)?[0])
    }

    pub fn boolean(&self, class: &str, key: &str, name: &str) -> Result<bool, String> {
        let field = self.field(class, key, name)?;
        if field.type_name != "EA::Reflection::Bool" {
            return Err(format!("Expected boolean at {class}/{key}/{name}"));
        }
        // Attributes store the byte followed by padding; layout bools may
        // contain only the byte. Reading a big-endian u32 gives the wrong bit.
        match field.data.get(..2) {
            Some("00") => Ok(false),
            Some("01") => Ok(true),
            _ => Err(format!("Invalid stock boolean {class}/{key}/{name}")),
        }
    }

    /// Every element of a float array attribute, in order (the index AttribSys
    /// `sub_82B72420` takes as its third argument).
    pub fn float_items(&self, class: &str, key: &str, name: &str) -> Result<Vec<f32>, String> {
        let field = self.field(class, key, name)?;
        if field.type_name != "EA::Reflection::Float" {
            return Err(format!("Expected float array at {class}/{key}/{name}"));
        }
        let array = field
            .array
            .as_ref()
            .ok_or_else(|| format!("Expected array payload at {class}/{key}/{name}"))?;
        array
            .items
            .iter()
            .map(|item| {
                let value = f32::from_bits(decode_words::<1>(item)?[0]);
                if value.is_finite() {
                    Ok(value)
                } else {
                    Err(format!("Non-finite stock float {class}/{key}/{name}"))
                }
            })
            .collect()
    }

    pub fn words<const N: usize>(
        &self,
        class: &str,
        key: &str,
        name: &str,
    ) -> Result<[u32; N], String> {
        decode_words(&self.field(class, key, name)?.data)
    }
}

fn decode_words<const N: usize>(text: &str) -> Result<[u32; N], String> {
    let hex: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if hex.len() != N * 8 || !hex.is_ascii() {
        return Err(format!(
            "Expected {N} big-endian words, found {} bytes of hex",
            hex.len()
        ));
    }
    let mut words = [0; N];
    for (i, word) in words.iter_mut().enumerate() {
        *word = u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16)
            .map_err(|e| format!("Invalid collection payload: {e}"))?;
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_export_names_resolve_readable_manual_settings() {
        // Identities and payload from the prepared vault; no asset IO or gameplay.
        let data: Collections = serde_json::from_value(serde_json::json!({
            "version": 1, "collections": [{
                "class": "anim_motion", "key": crate::attrib_hash::numeric_name("manual"),
                "parent": "", "source": "fixture", "sha256": "",
                "fields": { "Hash_A353CE1670D3AA40": { "type": "EA::Reflection::Float", "data": "3D23D70A" } }
            }]
        })).unwrap();
        assert_eq!(
            data.float("anim_motion", "manual", "manual_clamp_vel")
                .unwrap()
                .to_bits(),
            0x3d23d70a
        );
        assert!(data.field("anim_motion", "manual", "missing").is_err());
    }

    #[test]
    fn readable_names_resolve_numeric_queries_through_inheritance() {
        let data: Collections = serde_json::from_value(serde_json::json!({
            "version": 1, "collections": [
                {"class":"example", "key":"child", "parent":crate::attrib_hash::numeric_name("base"),
                 "source":"fixture", "sha256":"", "fields":{}},
                {"class":"example", "key":"base", "parent":"", "source":"fixture", "sha256":"",
                 "fields":{"value":{"type":"EA::Reflection::Float", "data":"3F800000"}}}
            ]
        })).unwrap();
        assert_eq!(
            data.float(
                &crate::attrib_hash::numeric_name("example"),
                &crate::attrib_hash::numeric_name("child"),
                &crate::attrib_hash::numeric_name("value")
            )
            .unwrap(),
            1.0
        );
    }

    #[test]
    fn float_arrays_read_their_items_not_the_header_lanes() {
        let data: Collections = serde_json::from_value(serde_json::json!({
            "version": 1, "collections": [{
                "class": "example", "key": "default", "parent": "", "source": "fixture",
                "sha256": "", "fields": { "thresholds": {
                    "type": "EA::Reflection::Float", "data": "00030003",
                    "array": {"capacity": 3, "element_size": 4, "alignment": 4,
                              "items": ["3FC00000", "40266666", "405CCCCD"]}
                }}
            }]
        }))
        .unwrap();
        let items = data.float_items("example", "default", "thresholds").unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], 1.5);
        assert_eq!(items[2].to_bits(), 0x405C_CCCD);
    }
}
