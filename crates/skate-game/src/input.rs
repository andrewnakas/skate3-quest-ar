//! Platform input adapter; no animation or physics state mutation here.
use crate::app::SimulationSet;
use bevy::prelude::*;

mod controllers;
pub(crate) mod gesture_catalog;
pub(crate) mod gesture_input;
pub(crate) mod gesture_mapping;
mod gesture_mapping_data;
pub(crate) mod platform;
pub(crate) use controllers::{ControllerInput, ControllerStatus};
use skate_core::input::tick::TickInput;

#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct PublishedTickInput(pub TickInput);

impl Default for PublishedTickInput {
    fn default() -> Self {
        Self(TickInput::new(
            0,
            skate_core::input::gameplay_map::GameplayActions::from_values([0.0; 18]),
            false,
        ))
    }
}

/// A pad supplied by something other than XInput (the XR Touch
/// controllers). When this resource exists it owns controller slot 0.
#[derive(Resource, Default)]
pub(crate) struct ExternalPad {
    state: Option<(u16, [u8; 2], [i16; 2], [i16; 2])>,
    packet: u32,
}

impl ExternalPad {
    pub(crate) fn set(&mut self, state: skate_core::input::xbox::XboxState) {
        let next = Some((state.buttons, state.triggers, state.left, state.right));
        if next != self.state {
            self.packet = self.packet.wrapping_add(1);
            self.state = next;
        }
    }

    fn packet(&self) -> Result<platform::DevicePacket, platform::DeviceError> {
        let (buttons, triggers, left, right) =
            self.state.ok_or(platform::DeviceError::Disconnected)?;
        Ok(platform::DevicePacket {
            number: self.packet,
            state: skate_core::input::xbox::XboxState {
                buttons,
                triggers,
                left,
                right,
            },
            // XINPUT_DEVSUBTYPE_GAMEPAD.
            subtype: 1,
        })
    }
}

pub(crate) struct InputPlugin;
impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ControllerInput>()
            .init_resource::<PublishedTickInput>()
            .add_systems(
                PreUpdate,
                poll_controllers.run_if(crate::graphics_menu::gameplay_active),
            )
            .add_systems(FixedUpdate, publish_actions.in_set(SimulationSet::Input));
    }
}

pub(crate) fn poll_controllers(
    mut input: ResMut<ControllerInput>,
    config: Res<crate::config::Config>,
    net: Option<Res<crate::multiplayer::Multiplayer>>,
    windows: Query<&Window>,
    external: Option<Res<ExternalPad>>,
    mut capabilities: Local<[platform::CapabilityCache; 4]>,
) {
    let previous = input.status;
    let focused = windows.iter().any(|w| w.focused);
    let active = net.is_some_and(|n| n.active());
    input.collect(std::array::from_fn(|slot| {
        if slot == 0
            && let Some(external) = &external
        {
            external.packet()
        } else if active
            && ((!focused && config.multiplayer.controller.is_none())
                || config
                    .multiplayer
                    .controller
                    .is_some_and(|selected| selected as usize != slot))
        {
            capabilities[slot].invalidate();
            Err(platform::DeviceError::Disconnected)
        } else {
            platform::poll_cached(slot, &mut capabilities[slot])
        }
    }));
    for (index, (&before, &after)) in previous.iter().zip(&input.status).enumerate() {
        if before != after {
            match after {
                ControllerStatus::Ready if index == 0 && external.is_some() => {
                    info!("Controller {index}: XR Touch controllers ready")
                }
                ControllerStatus::Ready => info!("Controller {index}: raw XInput ready"),
                ControllerStatus::Unavailable(platform::DeviceError::Disconnected) => {
                    info!("Controller {index}: disconnected");
                }
                _ => warn!("Controller {index}: {after:?}"),
            }
        }
    }
}

fn publish_actions(
    mut input: ResMut<ControllerInput>,
    mut published: ResMut<PublishedTickInput>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
) {
    if !crate::graphics_menu::gameplay_active(menu) {
        input.discard_gameplay();
    }
    input.publish_actions();
    published.0 = input.tick_input();
}

#[cfg(test)]
pub(crate) mod manual_replay;
