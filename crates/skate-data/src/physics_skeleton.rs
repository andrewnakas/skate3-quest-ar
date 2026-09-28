//! Lossless stock physics-bone records. Core consumes typed values derived here.
use serde::Deserialize;
use std::{collections::BTreeSet, path::Path};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicsBone {
    pub name: String,
    pub source_offset: u64,
    pub words: [u32; 28],
}
impl PhysicsBone {
    pub fn size(&self) -> [f32; 3] {
        [self.words[8], self.words[9], self.words[10]].map(f32::from_bits)
    }
    pub fn rotation(&self) -> [f32; 4] {
        [
            self.words[12],
            self.words[13],
            self.words[14],
            self.words[15],
        ]
        .map(f32::from_bits)
    }
    pub fn translation(&self) -> [f32; 3] {
        [self.words[16], self.words[17], self.words[18]].map(f32::from_bits)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicsSkeleton {
    pub name: String,
    pub source_offset: u64,
    pub bones: Vec<PhysicsBone>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    source_sha256: String,
    skeletons: Vec<PhysicsSkeleton>,
}

impl PhysicsSkeleton {
    pub fn load(path: &Path, bank_sha256: &str, name: &str) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let file: File = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if file.version != 1 || file.source_sha256 != bank_sha256 {
            return Err(
                "Physics skeleton bank identity or format differs from animation data".into(),
            );
        }
        let mut identities = BTreeSet::new();
        if file.skeletons.iter().any(|s| !identities.insert(&s.name)) {
            return Err("Converted physics skeleton has unresolved duplicate names".into());
        }
        let selected = file
            .skeletons
            .into_iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("Required stock physics skeleton {name} is absent"))?;
        let mut names = BTreeSet::new();
        if selected.bones.is_empty() || selected.bones.len() > 255 {
            return Err("Invalid stock physical bone count".into());
        }
        for bone in &selected.bones {
            if !names.insert(&bone.name)
                || bone
                    .size()
                    .iter()
                    .chain(&bone.rotation())
                    .chain(&bone.translation())
                    .any(|v| !v.is_finite())
            {
                return Err(format!(
                    "Invalid or repeated stock physics bone {}",
                    bone.name
                ));
            }
        }
        Ok(selected)
    }
}
