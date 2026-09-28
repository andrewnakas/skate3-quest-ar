//! Original physical WipeoutGround300. Off-board recovery destinations are
//! separate states; this owner retains the native request and timing fields.
mod contact;
mod drive_settings;
mod lifecycle;
mod output;
mod prediction;
pub(crate) mod ragdoll;
mod settings;
mod skeleton;
mod update;
use skate_core::player::wipeout_state::{
    State, contact_response::ContactResponse, drives, profiles::Profile,
};
use skate_data::collections::Collections;
use std::path::Path;

pub(crate) struct WipeoutState {
    pub(crate) ragdoll: ragdoll::RagdollSetup,
    pub state: State,
    settings: settings::Settings,
    drives: drives::Settings,
    profiles: [Profile; 5],
    contact: ContactResponse,
    prediction: prediction::Prediction,
}
impl WipeoutState {
    pub fn load(data: &Collections, assets: &Path, primary_bank_sha: &str) -> Result<Self, String> {
        //Ctor82D3B1EC..B25C full64 handle hashes, verified against the stock keys.
        let names = [
            "free_fall",
            "cannon_ball",
            "judo_kick",
            "swan_dive",
            "torpedo",
        ];
        let profiles = names.map(|name| settings::load_profile(data, name));
        let [a, b, c, d, e] = profiles;
        Ok(Self {
            ragdoll: ragdoll::RagdollSetup::load(data, assets, primary_bank_sha)?,
            state: State::default(),
            settings: settings::Settings::load(data)?,
            drives: drive_settings::load(data, assets, primary_bank_sha)?,
            profiles: [a?, b?, c?, d?, e?],
            contact: ContactResponse::new(),
            prediction: prediction::Prediction::new(),
        })
    }
}
pub(crate) use lifecycle::{enter, exit, post_physics};
pub(crate) use output::fill;
pub(crate) use update::advance;
