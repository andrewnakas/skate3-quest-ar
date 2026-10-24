//! The store build (no `retail` feature): stand-ins for the imported-disc tier's pad plumbing, so
//! the XR layer (which hands the Touch pad on to it in that tier) builds unchanged. Nothing reads
//! them here: the clean tier reads the controllers itself.
use bevy::prelude::*;

#[derive(Resource, Default)]
pub(crate) struct ExternalPad;

impl ExternalPad {
    pub(crate) fn set(&mut self, _state: crate::xr::touch::XboxState) {}
}

/// (Never scheduled: systems that order themselves before it just run.)
#[allow(dead_code)]
pub(crate) fn poll_controllers() {}
