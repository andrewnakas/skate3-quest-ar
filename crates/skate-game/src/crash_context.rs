//! Infrequent, allowlisted snapshots: no player names, addresses or lobby credentials.
use bevy::prelude::*;
use skate_core::player::state::PhysicalStateId;

type Transition = (u64, PhysicalStateId, PhysicalStateId);
// Recording a transition copies three scalars. Formatting happens once per second.
static EVENTS: std::sync::Mutex<([Option<Transition>; 64], usize, u64)> =
    std::sync::Mutex::new(([None; 64], 0, 0));

pub(crate) fn requested(tick: u64, from: PhysicalStateId, to: PhysicalStateId) {
    if let Ok(mut events) = EVENTS.try_lock() {
        let index = events.1;
        events.0[index] = Some((tick, from, to));
        events.1 = (index + 1) % 64;
        events.2 += 1;
    }
}

#[derive(Default)]
pub(crate) struct Snapshot {
    next: f64,
    state: String,
    graphics: String,
    gpu_recorded: bool,
}

pub(crate) fn sample(
    time: Res<Time<Real>>,
    config: Res<crate::config::Config>,
    physics: Res<crate::physics::GamePhysics>,
    skater: Res<crate::physics::SkaterRuntime>,
    map: Res<crate::map_transition::CurrentMap>,
    transition: Res<crate::map_transition::MapTransition>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    multiplayer: Option<Res<crate::multiplayer::Multiplayer>>,
    adapter: Option<Res<bevy::render::renderer::RenderAdapterInfo>>,
    mut previous: Local<Snapshot>,
) {
    let now = time.elapsed_secs_f64();
    if now < previous.next {
        return;
    }
    previous.next = now + 1.;
    if let Ok(mut events) = EVENTS.try_lock() {
        let count = events.2;
        let next = events.1;
        let batch = std::mem::replace(&mut events.0, [None; 64]);
        events.2 = 0;
        drop(events);
        if count > 64 {
            eprintln!(
                "REPORT_TRANSITION older_requests_overwritten:{}",
                count - 64
            );
        }
        for i in 0..64 {
            if let Some((tick, from, to)) = batch[(next + i) % 64] {
                eprintln!("REPORT_TRANSITION physics_tick:{tick} requested:{from:?}->{to:?}");
            }
        }
    }
    if !previous.gpu_recorded {
        if let Some(adapter) = adapter {
            eprintln!("REPORT_META gpu={:?}", &**adapter);
            previous.gpu_recorded = true;
        }
    }
    if let Some(multiplayer) = &multiplayer {
        eprintln!("REPORT_META network={}", multiplayer.diagnostic_summary());
    }
    let state = format!(
        "map_fingerprint:{:016x} generation:{} difficulty:{} physical:{:?} paused:{} map_loading:{} multiplayer_active:{} physics_failed:{}",
        config.map_fingerprint,
        map.generation,
        config.difficulty.key(),
        skater.player_state.current(),
        menu.as_ref().is_some_and(|m| m.open),
        transition.busy(),
        multiplayer.as_ref().is_some_and(|m| m.active()),
        physics.failed
    );
    if state != previous.state {
        eprintln!("REPORT_TRANSITION {state}");
        previous.state = state.clone();
    }
    eprintln!(
        "REPORT_META state={state} physics_tick:{} contacts:{} network_contacts:{}",
        physics.ticks, physics.contact_count, physics.network_contacts
    );
    if let Some(menu) = menu {
        let graphics = menu.diagnostic_settings();
        if graphics != previous.graphics {
            eprintln!("REPORT_META graphics={graphics}");
            previous.graphics = graphics;
        }
    }
}
