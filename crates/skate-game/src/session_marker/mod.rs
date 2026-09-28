//! Manual session markers. Automatic bail recovery owns a different checkpoint.
mod effect;
mod hud;
mod noise;
mod state;
mod validation;
use crate::{
    app::SimulationSet,
    input::ControllerInput,
    map_transition::CurrentMap,
    physics::{GamePhysics, SkaterRuntime},
};
use bevy::prelude::*;
use skate_core::physics::{board::BodyId, skeleton_animation_record::AnimationPartTransform};

#[derive(Clone, Copy, Debug)]
struct Marker {
    transform: AnimationPartTransform,
    on_board: bool,
    foot_forward: bool,
    generation: u64,
}

#[derive(Resource, Default)]
pub(crate) struct SessionMarker {
    marker: Option<Marker>,
    hold: state::Hold,
    generation: u64,
    pub visible: bool,
    pub can_place: bool,
    pub can_return: bool,
    pub progress: f32,
    blocked_until_release: bool,
    ui_time: f64,
    last_batch: u64,
}

pub(crate) struct SessionMarkerPlugin;
impl Plugin for SessionMarkerPlugin {
    fn build(&self, app: &mut App) {
        let root = &app.world().resource::<crate::config::Config>().asset_root;
        match validation::Validation::load(root) {
            Ok(settings) => {
                app.insert_resource(settings);
            }
            Err(e) => {
                error!("Session marker validation data: {e}");
                return;
            }
        }
        app.init_resource::<SessionMarker>()
            .add_message::<crate::skate_audio::FrontEndSoundRequest>()
            .add_systems(
                PreUpdate,
                suspend.after(crate::map_transition::MapTransitionSet),
            )
            .add_systems(
                FixedUpdate,
                update
                    .after(SimulationSet::Input)
                    .before(SimulationSet::Controls)
                    .run_if(crate::graphics_menu::gameplay_active),
            );
        hud::install(app);
        effect::install(app);
    }
}

fn suspend(
    mut marker: ResMut<SessionMarker>,
    map: Res<CurrentMap>,
    menu: Res<crate::graphics_menu::Menu>,
    replay: Res<crate::replay::Replay>,
    time: Res<Time<Real>>,
) {
    if marker.generation != map.generation {
        *marker = SessionMarker {
            generation: map.generation,
            blocked_until_release: true,
            ..default()
        };
    }
    if !crate::graphics_menu::gameplay_active(Some(menu)) || replay.active {
        marker.hold.cancel();
        marker.visible = false;
        marker.progress = 0.;
        marker.blocked_until_release = true;
        marker.ui_time = 0.;
    } else if !marker.blocked_until_release {
        // PlayerUI runs on the UI clock, independently of physics substeps.
        marker.ui_time += time.delta_secs_f64();
    }
}

fn update(
    vehicles: Res<crate::modding::vehicles::Vehicles>,
    mut session: ResMut<SessionMarker>,
    input: Res<ControllerInput>,
    map: Res<CurrentMap>,
    physics: Res<GamePhysics>,
    mut skater: ResMut<SkaterRuntime>,
    validation: Res<validation::Validation>,
    replay: Res<crate::replay::Replay>,
    mut audio: MessageWriter<crate::skate_audio::FrontEndSoundRequest>,
) {
    if vehicles.occupied() {
        session.blocked_until_release = true;
        return;
    }
    if replay.active {
        return;
    }
    let (modifier, set, held) = input.session_marker_actions();
    if session.blocked_until_release {
        if !modifier {
            session.blocked_until_release = false;
        }
        return;
    }
    let p = &skater.player_input.physical;
    let on_board = p.state.category_12 != 500;
    let deck = physics.board.part_transforms()[BodyId::Deck.index()];
    let mut transform = skater.animated_skeleton.roots.animation_to_world;
    if on_board {
        for i in 0..3 {
            transform[i][..3].copy_from_slice(&deck.basis.columns[i]);
        }
        transform[3] = [
            deck.translation.x,
            deck.translation.y + 0.2,
            deck.translation.z,
            0.,
        ];
        //82591E30: above .5m/s, project normalized velocity onto world Up.
        //The cross products are deliberately not normalized a second time.
        let velocity = Vec3::from_slice(&p.skateboard.vector_80.map(f32::from_bits)[..3]);
        if velocity.length_squared() > 0.25 {
            let right = Vec3::Y.cross(velocity.normalize());
            let forward = right.cross(Vec3::Y);
            if forward.length_squared() > 0.9 {
                transform[0] = right.extend(0.).to_array();
                transform[1] = Vec3::Y.extend(0.).to_array();
                transform[2] = forward.extend(0.).to_array();
            }
        }
    }
    let state = p.state.state_16;
    let state_allowed = (p.state.category_12 == 100
        && p.collision.wheel_count_0 >= 2
        && state != 104
        && deck.basis.columns[1][1] > 0.71)
        || (p.state.category_12 == 500 && state == 500);
    session.can_place = modifier
        && state_allowed
        && p.surface_default_mode != 8
        && validation.check(physics.world(), transform[3]);
    session.can_return = session
        .marker
        .is_some_and(|m| m.generation == map.generation)
        && !matches!(state, 104 | 502);
    session.visible = modifier;
    // A retained Pad publication must not turn one .pressed into repeated sets.
    if set && session.last_batch != input.consumed_batches {
        if session.can_place {
            session.marker = Some(Marker {
                transform,
                on_board,
                foot_forward: skater.animation.foot_forward(),
                generation: map.generation,
            });
            audio.write(crate::skate_audio::FrontEndSoundRequest(0x0d6c_88a3_b91c_828f));
        } else {
            audio.write(crate::skate_audio::FrontEndSoundRequest(0x66b3_afe3_b602_918c));
        }
    }
    session.last_batch = input.consumed_batches;
    let ready = p.state.flag_69 == 0
        && state != 702
        && !(physics.board_wiping_out && p.skeleton.teleport_pending_604 != 0)
        && skater.player_input.pending_teleport().is_none();
    let distance = session.marker.map_or(0., |m| {
        // 82DB6EC0 -> 82BE1AE8 publishes animation-to-world at output+368;
        // UpdateSessionMarker reads its translation at output+416.
        Vec3::from_slice(&skater.animated_skeleton.roots.animation_to_world[3][..3])
            .distance(Vec3::from_slice(&m.transform[3][..3]))
    });
    let usable = session.can_return;
    while session.ui_time >= 1. / 60. {
        session.ui_time -= 1. / 60.;
        let step = session.hold.update(held, usable, distance, ready);
        session.progress = step.progress;
        if step.relocate {
            if let Some(target) = session.marker {
                match skater.player_input.request_teleport(target.transform) {
                    Ok(()) => {
                        skater.animation.restore_foot_forward(target.foot_forward);
                        skater
                            .teleport_state
                            .request_manual(target.transform, target.on_board);
                        audio.write(crate::skate_audio::FrontEndSoundRequest(0x7f13_5f9f_d28f_7f21));
                    }
                    Err(e) => {
                        warn!("Session marker return rejected: {e}");
                        session.hold.cancel();
                    }
                }
            }
        }
    }
}
