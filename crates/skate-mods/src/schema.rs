use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub api: u32,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub entry: String,
    #[serde(default)]
    pub settings: BTreeMap<String, Setting>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Setting {
    pub label: String,
    pub description: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub default: Value,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    #[serde(default)]
    pub choices: Vec<String>,
}
pub fn valid_id(s: &str) -> bool {
    let stem = s.split('.').next().unwrap_or("");
    let reserved = matches!(
        stem,
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
    );
    !reserved
        && !s.is_empty()
        && s.len() <= 64
        && s.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-' || b == b'.'
        })
        && s != "."
        && s != ".."
}
impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.id) || self.api != 1 {
            return Err("Invalid ID or unsupported API (expected 1)".into());
        }
        if self.name.is_empty()
            || self.name.len() > 120
            || self.author.len() > 120
            || self.description.len() > 2048
        {
            return Err("Invalid metadata length".into());
        }
        if self.version.split('.').count() != 3
            || self.version.split('.').any(|p| p.parse::<u32>().is_err())
        {
            return Err("version must be MAJOR.MINOR.PATCH".into());
        }
        if self.settings.len() > 24 {
            return Err("At most 24 settings".into());
        }
        for (key, s) in &self.settings {
            if !valid_id(key) || s.label.len() > 120 || s.description.len() > 512 {
                return Err(format!("Invalid setting {key}"));
            }
            if s.kind == "number"
                && !(s
                    .min
                    .zip(s.max)
                    .is_some_and(|(a, b)| a.is_finite() && b.is_finite() && a <= b)
                    && s.step.is_some_and(|v| v.is_finite() && v > 0.))
            {
                return Err(format!(
                    "{key}: numbers need finite min, max and positive step"
                ));
            }
            if s.kind == "choice"
                && (s.choices.is_empty()
                    || s.choices.len() > 32
                    || s.choices.iter().any(|s| s.len() > 128))
            {
                return Err(format!("{key}: invalid choices"));
            }
            if !s.accepts(&s.default) {
                return Err(format!("{key}: invalid type/default"));
            }
        }
        Ok(())
    }
}
impl Setting {
    pub fn accepts(&self, v: &Value) -> bool {
        match self.kind.as_str() {
            "boolean" => v.is_boolean(),
            "number" => v.as_f64().is_some_and(|v| {
                v.is_finite()
                    && self.min.is_some_and(|a| v >= a)
                    && self.max.is_some_and(|b| v <= b)
            }),
            "string" => v
                .as_str()
                .is_some_and(|s| s.chars().count() <= 128 && !s.chars().any(char::is_control)),
            "choice" => v
                .as_str()
                .is_some_and(|s| self.choices.iter().any(|c| c == s)),
            _ => false,
        }
    }
}

/// Whitelisted native trainer multipliers. 1.0 preserves the installed stock value.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrainerTuning {
    pub pop: f32,
    pub grind_pop: f32,
    pub push_speed: f32,
    pub push_power: f32,
    pub braking: f32,
    pub steering: f32,
    pub wobble: f32,
    pub offboard_jump: f32,
    pub grip: f32,
    pub turn_power: f32,
    pub manual_drag: f32,
    pub hold_fakie: bool,
    /// Endless Tricks. **Not retail**: lets a held flip keep cycling past the authored quad
    /// while the pop has the air for it. False leaves the shipped four-rung ladder alone.
    pub endless_flips: bool,
    /// Upper bound on the extra rungs past the authored quad, so a stuck stick cannot run the
    /// counter away. 12 reaches rung 16, which is the last one the extension table names.
    pub endless_flip_max: u32,
    /// Require enough air for another whole rung before starting one. False -- the default --
    /// keeps flipping for as long as the input is held and the skater is off the ground, which is
    /// what makes this feel like holding a trick rather than clearing a height bar. True restores
    /// the conservative budget, which will not begin a rung it cannot finish.
    pub endless_air_check: bool,
}
impl Default for TrainerTuning {
    fn default() -> Self {
        Self {
            pop: 1.,
            grind_pop: 1.,
            push_speed: 1.,
            push_power: 1.,
            braking: 1.,
            steering: 1.,
            wobble: 1.,
            offboard_jump: 1.,
            grip: 1.,
            turn_power: 1.,
            manual_drag: 1.,
            hold_fakie: false,
            endless_flips: false,
            endless_flip_max: 12,
            endless_air_check: false,
        }
    }
}
impl TrainerTuning {
    pub fn valid(&self) -> bool {
        [
            self.pop,
            self.grind_pop,
            self.push_speed,
            self.push_power,
            self.braking,
            self.steering,
            self.offboard_jump,
            self.grip,
            self.turn_power,
            self.manual_drag,
        ]
        .into_iter()
        .all(|v| v.is_finite() && (0.25..=4.).contains(&v))
            && self.wobble.is_finite()
            && (0. ..=2.).contains(&self.wobble)
            && (1..=12).contains(&self.endless_flip_max)
    }
}
