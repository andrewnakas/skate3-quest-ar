//! Compatibility check for the stock gameplay expressions specialized by
//! skate-core's GameplayActions. This is not the full native expression VM.
use crate::AssetError;
use std::{collections::BTreeMap, fs, path::Path};

pub const STOCK_INPUT_PATH: &str = "private/stock/data/config/input.cfg";

pub struct StockGameplayConfig;

impl StockGameplayConfig {
    pub fn load(asset_root: &Path) -> Result<Self, AssetError> {
        let path = asset_root.join(STOCK_INPUT_PATH);
        let source = fs::read_to_string(&path)
            .map_err(|error| AssetError(format!("Cannot read {}: {error}", path.display())))?;
        Self::validate(&source)
            .map_err(|error| AssetError(format!("Input configuration {}: {error}", path.display())))
    }

    /// Require the actual file to agree with the supported native mapping.
    /// A modified expression fails explicitly instead of silently running the
    /// stock specialization against different user data.
    pub fn validate(source: &str) -> Result<Self, AssetError> {
        let source: String = source
            .lines()
            .map(|line| line.split("//").next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        let mut definitions = BTreeMap::new();
        for statement in source.split(';') {
            let Some((name, expression)) = statement.split_once('=') else {
                continue;
            };
            let name = name.trim();
            if BUTTON_NAMES.contains(&name) || GAMEPLAY.iter().any(|(key, _)| *key == name) {
                let expression: String =
                    expression.chars().filter(|c| !c.is_whitespace()).collect();
                if definitions.insert(name, expression).is_some() {
                    return Err(AssetError(format!(
                        "Duplicate gameplay input definition {name}"
                    )));
                }
            }
        }
        for (index, name) in BUTTON_NAMES.iter().enumerate() {
            require(&definitions, name, &format!("Button{index}"))?;
        }
        for (name, expression) in GAMEPLAY {
            require(&definitions, name, expression)?;
        }
        Ok(Self)
    }
}

fn require(
    definitions: &BTreeMap<&str, String>,
    name: &str,
    expected: &str,
) -> Result<(), AssetError> {
    match definitions.get(name) {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(AssetError(format!(
            "Unsupported {name} = {actual}; the implemented stock mapping requires {expected}"
        ))),
        None => Err(AssetError(format!(
            "Missing gameplay input definition {name}"
        ))),
    }
}

const BUTTON_NAMES: [&str; 24] = [
    "DPadU", "DPadD", "DPadL", "DPadR", "Start", "Back", "LStick", "RStick", "LBumper", "RBumper",
    "LTrigger", "RTrigger", "A", "B", "X", "Y", "LStickR", "LStickL", "LStickU", "LStickD",
    "RStickR", "RStickL", "RStickU", "RStickD",
];

// TU3 82697740 registers these eighteen actions at indices 64..81.
const GAMEPLAY: [(&str, &str); 18] = [
    ("GP_LStickX", "LStickR-LStickL"),
    ("GP_LStickY", "LStickU-LStickD"),
    ("GP_LStickIn", "LStick"),
    ("GP_RStickX", "RStickR-RStickL"),
    ("GP_RStickY", "RStickU-RStickD"),
    ("GP_RStickIn", "RStick"),
    ("GP_LTrigger", "LTrigger"),
    ("GP_RTrigger", "RTrigger"),
    ("GP_LBumper", "LBumper"),
    ("GP_RBumper", "RBumper"),
    ("GP_UDPad", "DPadU"),
    ("GP_DDPad", "DPadD"),
    ("GP_LDPad", "DPadL"),
    ("GP_RDPad", "DPadR"),
    ("GP_XFace", "X"),
    ("GP_YFace", "Y"),
    ("GP_AFace", "A"),
    ("GP_BFace", "B"),
];

#[cfg(test)]
#[path = "tests/input_config.rs"]
mod tests;
