//! OpenXR actions for the two Touch controllers: every input the skate pad
//! needs, plus the grip and aim poses used for placing the miniature.
use super::touch::{TouchMapper, TouchState};
use crate::input::ExternalPad;
use bevy::prelude::*;
use bevy_mod_openxr::{
    action_binding::{OxrSendActionBindings, OxrSuggestActionBinding},
    action_set_attaching::OxrAttachActionSet,
    action_set_syncing::{OxrActionSetSyncSet, OxrSyncActionSet},
    openxr_session_running,
    resources::OxrInstance,
    session::OxrSession,
};
use bevy_mod_xr::session::{XrSessionCreated, session_available};

const TOUCH: &str = "/interaction_profiles/oculus/touch_controller";

#[derive(Resource)]
pub(crate) struct TouchActions {
    set: openxr::ActionSet,
    left_stick: openxr::Action<openxr::Vector2f>,
    right_stick: openxr::Action<openxr::Vector2f>,
    left_trigger: openxr::Action<f32>,
    right_trigger: openxr::Action<f32>,
    left_grip: openxr::Action<f32>,
    right_grip: openxr::Action<f32>,
    a: openxr::Action<bool>,
    b: openxr::Action<bool>,
    x: openxr::Action<bool>,
    y: openxr::Action<bool>,
    left_click: openxr::Action<bool>,
    right_click: openxr::Action<bool>,
    menu: openxr::Action<bool>,
    pub(crate) left_aim: openxr::Action<openxr::Posef>,
    pub(crate) right_aim: openxr::Action<openxr::Posef>,
}

/// The latest raw controller frame, for systems other than the pad (the rig
/// uses it for placement mode).
#[derive(Resource, Default)]
pub(crate) struct LatestTouch(pub TouchState);

/// Marks the controller aim-pose space entities.
#[derive(Component)]
pub(crate) enum Hand {
    Left,
    Right,
}

/// Touch state for this frame is read (and the pad published) in this set.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TouchRead;

pub(crate) struct TouchActionsPlugin;

impl Plugin for TouchActionsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LatestTouch>()
            .add_systems(Startup, create.run_if(session_available))
            .add_systems(OxrSendActionBindings, suggest)
            .add_systems(XrSessionCreated, (attach, spawn_aim_spaces))
            .add_systems(
                PreUpdate,
                (
                    request_sync.before(OxrActionSetSyncSet),
                    read.in_set(TouchRead)
                        .after(OxrActionSetSyncSet)
                        .before(crate::input::poll_controllers),
                )
                    .run_if(openxr_session_running),
            );
    }
}

fn create(instance: Res<OxrInstance>, mut commands: Commands) {
    let set = instance
        .create_action_set("skate", "Skate", 0)
        .expect("create the skate action set");
    macro_rules! action {
        ($name:literal, $label:literal) => {
            set.create_action($name, $label, &[])
                .unwrap_or_else(|e| panic!("create XR action {}: {e}", $name))
        };
    }
    commands.insert_resource(TouchActions {
        left_stick: action!("left_stick", "Left stick"),
        right_stick: action!("right_stick", "Right stick"),
        left_trigger: action!("left_trigger", "Left trigger"),
        right_trigger: action!("right_trigger", "Right trigger"),
        left_grip: action!("left_grip", "Left grip"),
        right_grip: action!("right_grip", "Right grip"),
        a: action!("a", "A"),
        b: action!("b", "B"),
        x: action!("x", "X"),
        y: action!("y", "Y"),
        left_click: action!("left_stick_click", "Left stick click"),
        right_click: action!("right_stick_click", "Right stick click"),
        menu: action!("menu", "Menu"),
        left_aim: action!("left_aim", "Left aim pose"),
        right_aim: action!("right_aim", "Right aim pose"),
        set,
    });
}

fn suggest(actions: Res<TouchActions>, mut bindings: MessageWriter<OxrSuggestActionBinding>) {
    let pairs: [(openxr::sys::Action, &'static str); 15] = [
        (actions.left_stick.as_raw(), "/user/hand/left/input/thumbstick"),
        (actions.right_stick.as_raw(), "/user/hand/right/input/thumbstick"),
        (actions.left_trigger.as_raw(), "/user/hand/left/input/trigger/value"),
        (actions.right_trigger.as_raw(), "/user/hand/right/input/trigger/value"),
        (actions.left_grip.as_raw(), "/user/hand/left/input/squeeze/value"),
        (actions.right_grip.as_raw(), "/user/hand/right/input/squeeze/value"),
        (actions.a.as_raw(), "/user/hand/right/input/a/click"),
        (actions.b.as_raw(), "/user/hand/right/input/b/click"),
        (actions.x.as_raw(), "/user/hand/left/input/x/click"),
        (actions.y.as_raw(), "/user/hand/left/input/y/click"),
        (actions.left_click.as_raw(), "/user/hand/left/input/thumbstick/click"),
        (actions.right_click.as_raw(), "/user/hand/right/input/thumbstick/click"),
        (actions.menu.as_raw(), "/user/hand/left/input/menu/click"),
        (actions.left_aim.as_raw(), "/user/hand/left/input/aim/pose"),
        (actions.right_aim.as_raw(), "/user/hand/right/input/aim/pose"),
    ];
    for (action, path) in pairs {
        bindings.write(OxrSuggestActionBinding {
            action,
            interaction_profile: TOUCH.into(),
            bindings: vec![path.into()],
        });
    }
}

fn attach(actions: Res<TouchActions>, mut attach: MessageWriter<OxrAttachActionSet>) {
    attach.write(OxrAttachActionSet(actions.set.clone()));
}

fn spawn_aim_spaces(actions: Res<TouchActions>, session: Res<OxrSession>, mut commands: Commands) {
    for (action, hand) in [(&actions.left_aim, Hand::Left), (&actions.right_aim, Hand::Right)] {
        match session.create_action_space(action, openxr::Path::NULL, Isometry3d::IDENTITY) {
            Ok(space) => {
                commands.spawn((space, hand));
            }
            Err(error) => error!("XR aim space: {error}"),
        }
    }
}

fn request_sync(actions: Res<TouchActions>, mut sync: MessageWriter<OxrSyncActionSet>) {
    sync.write(OxrSyncActionSet(actions.set.clone()));
}

fn read(
    actions: Res<TouchActions>,
    session: Res<OxrSession>,
    time: Res<Time<Real>>,
    mut mapper: Local<TouchMapper>,
    mut latest: ResMut<LatestTouch>,
    mut pad: ResMut<ExternalPad>,
    ui: Option<Res<super::ui::UiOwnsControllers>>,
) {
    let session: &openxr::Session<openxr::AnyGraphics> = &session;
    let null = openxr::Path::NULL;
    let vec2 = |action: &openxr::Action<openxr::Vector2f>| {
        action
            .state(session, null)
            .map_or([0.0; 2], |s| [s.current_state.x, s.current_state.y])
    };
    let float = |action: &openxr::Action<f32>| action.state(session, null).map_or(0.0, |s| s.current_state);
    let button = |action: &openxr::Action<bool>| {
        action.state(session, null).is_ok_and(|s| s.current_state)
    };
    let touch = TouchState {
        left_stick: vec2(&actions.left_stick),
        right_stick: vec2(&actions.right_stick),
        left_trigger: float(&actions.left_trigger),
        right_trigger: float(&actions.right_trigger),
        left_grip: float(&actions.left_grip),
        right_grip: float(&actions.right_grip),
        a: button(&actions.a),
        b: button(&actions.b),
        x: button(&actions.x),
        y: button(&actions.y),
        left_stick_click: button(&actions.left_click),
        right_stick_click: button(&actions.right_click),
        menu: button(&actions.menu),
    };
    // A game pad (Quest: a Bluetooth Xbox controller) drives everything the Touch controllers do.
    // (Only axes that have moved off where they rested when the pad first spoke count: a device
    // that reports a stick or trigger parked away from zero can't hold the Touch sticks over.)
    #[cfg(target_os = "android")]
    let touch = {
        use std::sync::Mutex;
        static REST: Mutex<Option<([f32; 8], [bool; 8], u32)>> = Mutex::new(None);
        let pad = winit::platform::android::gamepad_state();
        if pad.events > 0 {
            let mut guard = REST.lock().unwrap();
            let (rest, moved, logged) = guard.get_or_insert((pad.axes, [false; 8], 0));
            let mut axes = pad.axes;
            for i in 0..8 {
                if (pad.axes[i] - rest[i]).abs() > 0.25 {
                    moved[i] = true;
                }
                if !moved[i] {
                    axes[i] = 0.0;
                }
            }
            if pad.events / 200 != *logged {
                *logged = pad.events / 200;
                info!("PAD events {} axes {:?} rest {:?} moved {:?} buttons {:#x}", pad.events, pad.axes, rest, moved, pad.buttons);
            }
            super::touch::pad_into_touch(touch, axes, pad.buttons)
        } else {
            touch
        }
    };
    latest.0 = touch;
    let mut state = mapper.map(&touch, time.delta_secs());
    // A menu tap is Start, which opens the flat pause menu; its camera is off
    // in XR, so the game would freeze behind a menu nobody can see.
    state.buttons &= !super::touch::START;
    // A headset menu owns the controllers: the skater gets a neutral pad.
    if ui.is_some_and(|ui| ui.0) {
        state = super::touch::XboxState { buttons: 0, triggers: [0, 0], left: [0, 0], right: [0, 0] };
    }
    pad.set(state);
}
