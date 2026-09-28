//! Opt-in repeatable frame/physics timing: SKATE_PERF_REPORT=path.json.
use bevy::prelude::*;
use bevy::render::{Render, RenderApp, RenderSystems};
use std::sync::{Arc, Mutex};
use std::{collections::BTreeMap, sync::OnceLock};
use std::{path::PathBuf, time::Instant};

static EPOCH: OnceLock<Instant> = OnceLock::new();
static SLOW_SECTIONS: Mutex<Vec<(&'static str, f64, f64)>> = Mutex::new(Vec::new());
fn timestamp(now: Instant) -> f64 {
    EPOCH
        .get()
        .map_or(0., |epoch| now.duration_since(*epoch).as_secs_f64() * 1000.)
}

static SECTIONS: Mutex<BTreeMap<&'static str, (f64, u64)>> = Mutex::new(BTreeMap::new());
pub(crate) struct Scope {
    name: &'static str,
    start: Option<Instant>,
}
impl Scope {
    pub(crate) fn new(name: &'static str) -> Self {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        Self {
            name,
            start: ENABLED
                .get_or_init(|| std::env::var_os("SKATE_PERF_REPORT").is_some())
                .then(Instant::now),
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            let elapsed = start.elapsed().as_secs_f64() * 1000.;
            if elapsed >= 2. {
                let mut slow = SLOW_SECTIONS.lock().unwrap();
                if slow.len() < 256 {
                    slow.push((self.name, timestamp(start), elapsed));
                }
            }
            let mut sections = SECTIONS.lock().unwrap();
            let entry = sections.entry(self.name).or_default();
            entry.0 += elapsed;
            entry.1 += 1;
        }
    }
}

#[derive(Resource)]
pub(crate) struct Performance {
    path: PathBuf,
    start: Option<Instant>,
    frame_start: Instant,
    previous: Instant,
    physics_ms: f64,
    ticks: u32,
    samples: Vec<[f64; 4]>,
    sample_times: Vec<f64>,
    render: Arc<Mutex<Vec<[f64; 7]>>>,
}
impl Performance {
    pub(crate) fn physics(&mut self, elapsed: std::time::Duration) {
        self.physics_ms += elapsed.as_secs_f64() * 1000.;
        self.ticks += 1;
    }
}

pub(crate) struct PerformancePlugin;
impl Plugin for PerformancePlugin {
    fn build(&self, app: &mut App) {
        let Some(path) = std::env::var_os("SKATE_PERF_REPORT") else {
            return;
        };
        let now = Instant::now();
        let _ = EPOCH.set(now);
        let render = Arc::new(Mutex::new(Vec::with_capacity(16384)));
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(RenderPerformance {
                    start: None,
                    frame: now,
                    prepared: now,
                    mark: now,
                    phases: [0.; 3],
                    samples: render.clone(),
                })
                .add_systems(Render, render_begin.before(RenderSystems::ExtractCommands))
                .add_systems(
                    Render,
                    render_assets_done
                        .after(RenderSystems::PrepareMeshes)
                        .after(RenderSystems::PrepareAssets)
                        .before(RenderSystems::ManageViews),
                )
                .add_systems(
                    Render,
                    render_views_done
                        .after(RenderSystems::ManageViews)
                        .before(RenderSystems::Queue),
                )
                .add_systems(
                    Render,
                    render_queue_done
                        .after(RenderSystems::Queue)
                        .before(RenderSystems::PhaseSort),
                )
                .add_systems(
                    Render,
                    render_prepared
                        .after(RenderSystems::Prepare)
                        .before(RenderSystems::Render),
                )
                .add_systems(Render, render_finish.after(RenderSystems::PostCleanup));
            render_app
                .init_resource::<MeshBindingTimer>()
                .add_systems(
                    Render,
                    mesh_binding_begin
                        .in_set(RenderSystems::PrepareBindGroups)
                        .before(bevy::pbr::prepare_mesh_bind_groups),
                )
                .add_systems(
                    Render,
                    mesh_binding_end
                        .in_set(RenderSystems::PrepareBindGroups)
                        .after(bevy::pbr::prepare_mesh_bind_groups),
                );
        }
        // Benchmark measurements must not depend on whether the window has focus.
        app.insert_resource(bevy::winit::WinitSettings::continuous());
        app.add_systems(Startup, report_adapter);
        if std::env::var_os("SKATE_PERF_CAMERA_SWEEP").is_some() {
            app.add_systems(
                Update,
                sweep_camera.after(crate::app::FrameSet::Verification),
            );
        }
        app.insert_resource(Performance {
            path: path.into(),
            start: None,
            frame_start: now,
            previous: now,
            physics_ms: 0.,
            ticks: 0,
            samples: Vec::with_capacity(16384),
            sample_times: Vec::with_capacity(16384),
            render,
        })
        .add_systems(First, begin)
        .add_systems(Last, finish);
    }
}

#[derive(Resource, Default)]
struct MeshBindingTimer(Option<Scope>);
fn mesh_binding_begin(mut timer: ResMut<MeshBindingTimer>) {
    timer.0 = Some(Scope::new("mesh_bind_groups"));
}
fn mesh_binding_end(mut timer: ResMut<MeshBindingTimer>) {
    timer.0.take();
}
fn report_adapter(
    device: Res<bevy::render::renderer::RenderDevice>,
    adapter: Res<bevy::render::renderer::RenderAdapterInfo>,
) {
    eprintln!(
        "SKATE_GPU adapter={:?} features={:?} limits={:?}",
        &**adapter,
        device.features(),
        device.limits()
    );
}

// Rendering-only benchmark: exercise changing visibility without steering the
// skater or feeding synthetic inputs into the simulation camera.
fn sweep_camera(time: Res<Time<Real>>, mut cameras: Query<&mut Transform, With<Camera3d>>) {
    for mut transform in &mut cameras {
        transform.rotate_y(time.elapsed_secs() * std::f32::consts::TAU / 12.);
    }
}
fn begin(mut p: ResMut<Performance>) {
    p.frame_start = Instant::now();
    p.physics_ms = 0.;
    p.ticks = 0;
}
fn finish(mut p: ResMut<Performance>, mut exit: MessageWriter<AppExit>) {
    let now = Instant::now();
    let elapsed = now
        .duration_since(*p.start.get_or_insert(now))
        .as_secs_f64();
    let frame = now.duration_since(p.previous).as_secs_f64() * 1000.;
    p.previous = now;
    // Exclude initialization and shader warmup. Frame interval includes render
    // synchronization; CPU schedule time is First..Last of the main world.
    if elapsed > 10. {
        let sample = [
            frame,
            now.duration_since(p.frame_start).as_secs_f64() * 1000.,
            p.physics_ms,
            f64::from(p.ticks),
        ];
        p.samples.push(sample);
        p.sample_times.push(timestamp(now));
    }
    if elapsed > 25. && !p.samples.is_empty() {
        let mean = |column: usize| {
            p.samples.iter().map(|s| s[column]).sum::<f64>() / p.samples.len() as f64
        };
        let mut frames: Vec<_> = p.samples.iter().map(|s| s[0]).collect();
        frames.sort_by(f64::total_cmp);
        let render = p.render.lock().unwrap();
        let render_mean = |column: usize| {
            render.iter().map(|s| s[column]).sum::<f64>() / render.len().max(1) as f64
        };
        let report = serde_json::json!({
            "frames": frames.len(), "fps": 1000. / mean(0),
            "frame_ms_mean": mean(0), "frame_ms_median": frames[frames.len()/2],
            "frame_ms_p95": frames[(frames.len()-1)*95/100],
            "main_schedule_ms_mean": mean(1), "physics_ms_per_frame": mean(2),
            "physics_ticks_per_frame": mean(3),
            "physics_ms_per_tick": mean(2) / mean(3).max(f64::EPSILON),
            "render_prepare_ms_mean": render_mean(0), "render_submit_ms_mean": render_mean(1),
            "render_assets_ms_mean": render_mean(2), "render_views_ms_mean": render_mean(3), "render_queue_ms_mean": render_mean(4),
            "sections_ms_per_call": SECTIONS.lock().unwrap().iter().map(|(&k, &(ms, n))| (k, ms / n as f64)).collect::<BTreeMap<_,_>>(),
            "samples": p.samples,
            "sample_elapsed_ms": p.sample_times,
            "render_samples": &*render,
            "render_sample_columns": ["prepare_ms", "submit_ms", "assets_ms", "views_ms", "queue_ms", "elapsed_ms", "waiting_pipelines"],
            "slow_sections": &*SLOW_SECTIONS.lock().unwrap(),
            "slow_section_columns": ["section", "start_elapsed_ms", "duration_ms"],
            "frame_ms_max": frames[frames.len()-1],
            "frame_ms_p99": frames[(frames.len()-1)*99/100],
            "frames_over_8ms": frames.iter().filter(|&&ms| ms > 8.).count(),
        });
        match std::fs::write(&p.path, serde_json::to_vec_pretty(&report).unwrap()) {
            Ok(()) => {
                eprintln!("SKATE_PERF_REPORT {}", p.path.display());
                exit.write(AppExit::Success);
            }
            Err(e) => {
                eprintln!("Performance report: {e}");
                exit.write(AppExit::error());
            }
        }
    }
}

#[derive(Resource)]
struct RenderPerformance {
    start: Option<Instant>,
    frame: Instant,
    prepared: Instant,
    mark: Instant,
    phases: [f64; 3],
    samples: Arc<Mutex<Vec<[f64; 7]>>>,
}
fn render_begin(mut p: ResMut<RenderPerformance>) {
    p.frame = Instant::now();
    p.mark = p.frame;
}
fn render_assets_done(mut p: ResMut<RenderPerformance>) {
    let now = Instant::now();
    p.phases[0] = now.duration_since(p.mark).as_secs_f64() * 1000.;
    p.mark = now;
}
fn render_views_done(mut p: ResMut<RenderPerformance>) {
    let now = Instant::now();
    p.phases[1] = now.duration_since(p.mark).as_secs_f64() * 1000.;
    p.mark = now;
}
fn render_queue_done(mut p: ResMut<RenderPerformance>) {
    p.phases[2] = p.mark.elapsed().as_secs_f64() * 1000.;
}
fn render_prepared(mut p: ResMut<RenderPerformance>) {
    p.prepared = Instant::now();
}
fn render_finish(
    mut p: ResMut<RenderPerformance>,
    pipelines: Res<bevy::render::render_resource::PipelineCache>,
) {
    let now = Instant::now();
    if now
        .duration_since(*p.start.get_or_insert(now))
        .as_secs_f64()
        > 10.
    {
        p.samples.lock().unwrap().push([
            p.prepared.duration_since(p.frame).as_secs_f64() * 1000.,
            now.duration_since(p.prepared).as_secs_f64() * 1000.,
            p.phases[0],
            p.phases[1],
            p.phases[2],
            timestamp(now),
            pipelines.waiting_pipelines().count() as f64,
        ]);
    }
}
