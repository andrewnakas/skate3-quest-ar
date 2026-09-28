//! Bounded, local-only CPU timeline. GPU diagnostics are delayed duration counters.
use bevy::{
    log::{
        tracing::{
            self, Subscriber,
            span::{Attributes, Id},
        },
        tracing_subscriber::{
            self as subscriber, Layer,
            layer::{Context, SubscriberExt},
            registry::LookupSpan,
        },
    },
    prelude::*,
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const MAX_BYTES: usize = 128 * 1024 * 1024;
static CAPTURE: OnceLock<Arc<Capture>> = OnceLock::new();
static NEXT_THREAD: AtomicU64 = AtomicU64::new(1);
thread_local! {
    static THREAD: u64 = NEXT_THREAD.fetch_add(1, Ordering::Relaxed);
    static ENTERED: RefCell<Vec<(u64, Instant)>> = const { RefCell::new(Vec::new()) };
}
#[derive(Debug, Default)]
struct Options {
    path: Option<PathBuf>,
    seconds: u64,
    delay: u64,
    wait: bool,
    gpu: bool,
    min_us: u64,
}
impl Options {
    fn parse(args: impl Iterator<Item = std::ffi::OsString>) -> Result<Self, String> {
        let mut result = Self {
            seconds: 30,
            min_us: 25,
            ..Self::default()
        };
        let mut args = args;
        let mut modifiers = false;
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--trace") => {
                    if result.path.is_some() {
                        return Err("Duplicate --trace".into());
                    }
                    result.path = Some(
                        args.next()
                            .ok_or("--trace requires a new output JSON path")?
                            .into(),
                    );
                }
                Some("--trace-min-us") => {
                    modifiers = true;
                    result.min_us = args
                        .next()
                        .and_then(|a| a.to_str().and_then(|v| v.parse().ok()))
                        .filter(|v| *v <= 1000)
                        .ok_or("--trace-min-us requires 0..1000 microseconds")?;
                }
                Some("--trace-seconds" | "--trace-delay") => {
                    modifiers = true;
                    let value = args
                        .next()
                        .and_then(|a| a.to_str().and_then(|v| v.parse::<u64>().ok()))
                        .ok_or("Trace duration must be an integer")?;
                    if value > 600 || (arg == "--trace-seconds" && value == 0) {
                        return Err("Trace seconds: 1..600; delay: 0..600".into());
                    }
                    if arg == "--trace-seconds" {
                        result.seconds = value;
                    } else {
                        result.delay = value;
                    }
                }
                Some("--trace-wait") => {
                    modifiers = true;
                    result.wait = true;
                }
                Some("--trace-gpu") => {
                    modifiers = true;
                    result.gpu = true;
                }
                // Consume values of other CLI options; never interpret them as trace flags.
                Some("--net-local") => {
                    args.next();
                    args.next();
                }
                Some(
                    "--net-host" | "--net-session" | "--spawn-offset" | "--player-title"
                    | "--appearance" | "--controller" | "--assets" | "--map" | "--teleport"
                    | "--difficulty" | "--verify",
                ) => {
                    args.next();
                }
                _ => {}
            }
        }
        if modifiers && result.path.is_none() {
            return Err("Trace modifiers require --trace FILE.json".into());
        }
        if result.wait && result.delay != 0 {
            return Err("Choose --trace-wait or --trace-delay".into());
        }
        Ok(result)
    }
}
struct Capture {
    origin: Instant,
    active: AtomicBool,
    start: AtomicBool,
    stop: AtomicBool,
    dropped: AtomicU64,
    sender: SyncSender<Value>,
    gpu: bool,
    min_us: u64,
    filtered: AtomicU64,
}
impl Capture {
    fn retain_duration(&self, micros: u64) -> bool {
        if micros >= self.min_us {
            return true;
        }
        self.filtered.fetch_add(1, Ordering::Relaxed);
        false
    }
    fn send(&self, value: Value) {
        if self.sender.try_send(value).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn counter(&self, name: &str, args: Value) {
        if self.active.load(Ordering::Relaxed) {
            self.send(json!({"name":name,"ph":"C","pid":1,"tid":0,"ts":self.origin.elapsed().as_micros() as u64,"args":args}));
        }
    }
}
pub(crate) struct Guard(Arc<Capture>, Option<JoinHandle<std::io::Result<()>>>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::Relaxed);
        if let Some(writer) = self.1.take() {
            if !matches!(writer.join(), Ok(Ok(()))) {
                eprintln!("TRACE export failed; capture may be incomplete");
            }
        }
    }
}

pub(crate) fn init() -> Result<Option<Guard>, String> {
    let options = Options::parse(std::env::args_os().skip(1))?;
    init_options(options)
}
fn init_options(options: Options) -> Result<Option<Guard>, String> {
    let mut guard = None;
    let layer = if let Some(path) = options.path {
        // create_new refuses to destroy a previous recording.
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(
                |_| "Cannot create trace: use a new filename in an existing writable directory",
            )?;
        let (sender, receiver) = sync_channel::<Value>(8192);
        let capture = Arc::new(Capture {
            origin: Instant::now(),
            active: AtomicBool::new(!options.wait && options.delay == 0),
            start: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            sender,
            gpu: options.gpu,
            min_us: options.min_us,
            filtered: AtomicU64::new(0),
        });
        CAPTURE
            .set(capture.clone())
            .map_err(|_| "Trace already initialized")?;
        let state = capture.clone();
        let writer = std::thread::Builder::new().name("trace-export".into()).spawn(move || {
            let result = (|| -> std::io::Result<()> {
            let mut writer = BufWriter::new(file);
            writer.write_all(b"{\"traceEvents\":[\n")?;
            let mut bytes = 0;
            let mut started = state.active.load(Ordering::Relaxed).then(Instant::now);
            let mut reason = "stopped";
            loop {
                if state.stop.load(Ordering::Relaxed) { break; }
                if started.is_none() && (state.start.swap(false, Ordering::Relaxed) || (!options.wait && state.origin.elapsed().as_secs() >= options.delay)) {
                    started = Some(Instant::now());
                    state.active.store(true, Ordering::Relaxed);
                }
                if started.is_some_and(|s| s.elapsed() >= Duration::from_secs(options.seconds)) { reason = "duration"; break; }
                if let Ok(event) = receiver.recv_timeout(Duration::from_millis(20)) {
                    let line = serde_json::to_vec(&event)?;
                    if bytes + line.len() + 2 > MAX_BYTES { reason = "size_limit"; break; }
                    writer.write_all(&line)?;
                    writer.write_all(b",\n")?;
                    bytes += line.len() + 2;
                }
            }
            state.active.store(false, Ordering::Relaxed);
            // Drain the bounded queue on normal stop so end-of-frame events survive.
            while let Ok(event) = receiver.try_recv() {
                let line = serde_json::to_vec(&event)?;
                if bytes + line.len() + 2 > MAX_BYTES { reason = "size_limit"; break; }
                writer.write_all(&line)?; writer.write_all(b",\n")?; bytes += line.len() + 2;
            }
            let footer = json!({"name":"capture_complete","ph":"i","s":"g","pid":1,"tid":0,"ts":state.origin.elapsed().as_micros() as u64,"args":{"reason":reason,"dropped_events":state.dropped.load(Ordering::Relaxed),"filtered_short_spans":state.filtered.load(Ordering::Relaxed),"min_span_us":state.min_us}});
            serde_json::to_writer(&mut writer, &footer)?;
            writer.write_all(b"],\"displayTimeUnit\":\"ms\"}\n")?;
            writer.flush()?;
            eprintln!("TRACE complete ({reason}); open the requested JSON in https://ui.perfetto.dev");
            Ok(())
            })();
            state.active.store(false, Ordering::Relaxed);
            if result.is_err() { eprintln!("TRACE export failed; check free disk space and destination access"); }
            result
        }).map_err(|_| "Cannot start trace writer")?;
        capture.send(json!({"name":"build","ph":"i","s":"g","ts":0,"pid":1,"tid":0,"args":{"build_id":env!("SKATE_BUILD_ID"),"version":env!("CARGO_PKG_VERSION"),"revision":env!("SKATE_RELEASE_REVISION"),"bevy":"0.18.1","wgpu":"27.0.1","debug_assertions":cfg!(debug_assertions),"gpu_requested":options.gpu,"min_span_us":options.min_us,"seconds":options.seconds,"delay":options.delay,"wait":options.wait}}));
        eprintln!(
            "TRACE armed; F9 starts waiting capture, F10 stops/exports. Recording does not stop gameplay."
        );
        guard = Some(Guard(capture.clone(), Some(writer)));
        Some(
            Timeline(capture).with_filter(subscriber::filter::FilterFn::new(|m| {
                m.is_span()
                    && (m.target().starts_with("bevy_") || m.target().starts_with("skate3rust"))
            })),
        )
    } else {
        None
    };
    // Per-layer filters keep spans completely disabled in normal play. No environment
    // filter is applied to the recorder; RUST_LOG cannot silently remove its systems.
    let filter = subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| subscriber::EnvFilter::new("info,wgpu=error,naga=warn"));
    let logs = subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(subscriber::filter::FilterFn::new(|m| m.is_event()))
        .with_filter(filter);
    tracing::subscriber::set_global_default(subscriber::registry().with(layer).with(logs))
        .map_err(|_| "Cannot install profiling/log subscriber")?;
    let _ = tracing_log::LogTracer::init();
    Ok(guard)
}

struct Timeline(Arc<Capture>);
#[derive(Default)]
struct SystemName(String);
#[derive(Default)]
struct ScheduleName(String);
impl tracing::field::Visit for ScheduleName {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() != "name" {
            return;
        }
        let name = format!("{value:?}");
        if matches!(
            name.as_str(),
            "Main"
                | "First"
                | "PreUpdate"
                | "StateTransition"
                | "RunFixedMainLoop"
                | "FixedMain"
                | "FixedFirst"
                | "FixedPreUpdate"
                | "FixedUpdate"
                | "FixedPostUpdate"
                | "FixedLast"
                | "Update"
                | "SpawnScene"
                | "PostUpdate"
                | "Last"
                | "ExtractSchedule"
                | "Render"
                | "Startup"
                | "PreStartup"
                | "PostStartup"
        ) {
            self.0 = name;
        }
    }
}
impl tracing::field::Visit for SystemName {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "name" && value.len() <= 512 && !value.contains(['/', '\\', '\n']) {
            self.0 = value.into();
        }
    }
}
impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Timeline {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if attrs.metadata().target().starts_with("bevy_ecs")
            && attrs.metadata().name() == "schedule"
        {
            let mut name = ScheduleName::default();
            attrs.record(&mut name);
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(SystemName(name.0));
            }
        }
        // Only Bevy ECS system labels may supply dynamic text. All other fields,
        // source locations, thread names and log events are deliberately omitted.
        if attrs.metadata().target().starts_with("bevy_ecs")
            && matches!(attrs.metadata().name(), "system" | "system_commands")
        {
            let mut name = SystemName::default();
            attrs.record(&mut name);
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(name);
            }
        }
    }
    fn on_enter(&self, id: &Id, _: Context<'_, S>) {
        if self.0.active.load(Ordering::Relaxed) {
            ENTERED.with(|stack| stack.borrow_mut().push((id.into_u64(), Instant::now())));
        }
    }
    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        let start = ENTERED.with(|stack| {
            let mut stack = stack.borrow_mut();
            let index = stack.iter().rposition(|(key, _)| *key == id.into_u64())?;
            Some(stack.remove(index).1)
        });
        if let Some(start) = start.filter(|_| self.0.active.load(Ordering::Relaxed)) {
            let duration = start.elapsed().as_micros() as u64;
            if !self.0.retain_duration(duration) {
                return;
            }
            if let Some(span) = ctx.span(id) {
                let ext = span.extensions();
                let label = ext
                    .get::<SystemName>()
                    .filter(|s| !s.0.is_empty())
                    .map(|s| s.0.as_str())
                    .unwrap_or(span.metadata().name());
                THREAD.with(|tid| self.0.send(json!({"name":label,"cat":span.metadata().name(),"ph":"X","pid":1,"tid":tid,"ts":start.duration_since(self.0.origin).as_micros() as u64,"dur":duration})));
            }
        }
    }
}

#[derive(Resource, Default)]
struct Frames {
    last: Option<Instant>,
    fixed: u32,
    frame: u64,
    diagnostics: HashMap<String, (usize, bevy::platform::time::Instant)>,
}
pub(crate) fn install(app: &mut App) {
    let Some(capture) = CAPTURE.get() else {
        return;
    };
    if capture.gpu {
        app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
    }
    if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
        render.add_systems(
            bevy::render::Render,
            render_counters.after(bevy::render::RenderSystems::Cleanup),
        );
    }
    app.init_resource::<Frames>()
        .add_systems(First, begin_frame)
        .add_systems(FixedFirst, |mut frames: ResMut<Frames>| {
            frames.fixed += 1;
        })
        .add_systems(Last, end_frame);
}
fn render_counters(
    mut frame: Local<u64>,
    meshes: Res<bevy::render::render_asset::RenderAssets<bevy::render::mesh::RenderMesh>>,
    images: Res<bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>>,
    pipelines: Res<bevy::render::render_resource::PipelineCache>,
) {
    *frame += 1;
    let capture = CAPTURE.get().unwrap();
    if *frame % 60 == 1 && capture.active.load(Ordering::Relaxed) {
        capture.counter("render_resources",json!({"prepared_meshes":meshes.iter().count(),"prepared_images":images.iter().count(),"waiting_pipelines":pipelines.waiting_pipelines().count()}));
    }
}
pub(crate) fn map_metadata(config: &crate::config::Config) {
    let Some(capture) = CAPTURE.get() else {
        return;
    };
    capture.send(json!({"name":"loaded_map","ph":"i","s":"g","pid":1,"tid":0,"ts":capture.origin.elapsed().as_micros() as u64,"args":{"fingerprint":format!("{:016x}",config.map_fingerprint),"difficulty":config.difficulty.key(),"geometry":config.map.as_ref().map(|m|json!({"version":m.version,"triangles":m.geometry.indices.len()/3,"materials":m.materials.len(),"textures":m.textures.len(),"rail_records":m.rails.len()}))}}));
}
fn begin_frame(mut frames: ResMut<Frames>) {
    let capture = CAPTURE.get().unwrap();
    let now = Instant::now();
    if let Some(last) = frames.last.replace(now) {
        capture.counter(
            "frame_interval_ms",
            json!({"value":now.duration_since(last).as_secs_f64()*1000.}),
        );
    }
    frames.fixed = 0;
    frames.frame += 1;
}
fn end_frame(
    mut frames: ResMut<Frames>,
    keys: Res<ButtonInput<KeyCode>>,
    config: Res<crate::config::Config>,
    fixed: Res<Time<Fixed>>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    entities: Query<Entity>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    device: Option<Res<bevy::render::renderer::RenderDevice>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &Msaa)>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    map: Res<crate::map_transition::CurrentMap>,
    transition: Res<crate::map_transition::MapTransition>,
    adapter: Option<Res<bevy::render::renderer::RenderAdapterInfo>>,
) {
    let capture = CAPTURE.get().unwrap();
    if keys.just_pressed(KeyCode::F9) {
        capture.start.store(true, Ordering::Relaxed);
    }
    if keys.just_pressed(KeyCode::F10) {
        capture.stop.store(true, Ordering::Relaxed);
    }
    if !capture.active.load(Ordering::Relaxed) {
        return;
    }
    capture.counter("frame", json!({"number":frames.frame,"fixed_schedule_iterations":frames.fixed,"fixed_period_ms":fixed.timestep().as_secs_f64()*1000.,"cpu_main_until_trace_sample_ms":frames.last.unwrap().elapsed().as_secs_f64()*1000.}));
    if frames.frame % 60 == 1 {
        if let Some(adapter) = adapter {
            capture.send(json!({"name":"adapter","ph":"i","s":"g","pid":1,"tid":0,"ts":capture.origin.elapsed().as_micros() as u64,"args":{"vendor":adapter.vendor,"device":adapter.device,"backend":format!("{:?}",adapter.backend)}}));
        }
        capture.send(json!({"name":"game_configuration","ph":"i","s":"g","pid":1,"tid":0,"ts":capture.origin.elapsed().as_micros() as u64,"args":{"graphics":menu.as_ref().map(|m|m.diagnostic_settings()),"paused":menu.as_ref().is_some_and(|m|m.open),"map_loading":transition.busy(),"map_generation":map.generation}}));
        capture.counter(
            "assets",
            json!({"meshes":meshes.len(),"images":images.len(),"entities":entities.iter().count()}),
        );
        capture.send(json!({"name":"configuration","ph":"i","s":"g","pid":1,"tid":0,"ts":capture.origin.elapsed().as_micros() as u64,"args":{"map_fingerprint":format!("{:016x}",config.map_fingerprint),"difficulty":config.difficulty.key(),"retail_bindless":device.as_ref().map(|d|bevy::pbr::material_uses_bindless_resources::<crate::retail_render::RetailWorldMaterial>(d)),"timestamp_queries":device.as_ref().is_some_and(|d|d.features().contains(bevy::render::settings::WgpuFeatures::TIMESTAMP_QUERY)),"windows":windows.iter().map(|w|json!({"width":w.physical_width(),"height":w.physical_height(),"present_mode":format!("{:?}",w.present_mode)})).collect::<Vec<_>>(),"cameras":cameras.iter().map(|(c,m)|json!({"size":c.physical_target_size().map(|s|[s.x,s.y]),"msaa":m.samples()})).collect::<Vec<_>>()}}));
    }
    for diagnostic in diagnostics.iter() {
        let path = diagnostic.path().as_str();
        // Built-in render paths only. Do not export arbitrary plugin diagnostic labels.
        if !path.starts_with("render/") || path.len() > 1024 || path.contains(['\\', ':']) {
            continue;
        }
        let Some(measurement) = diagnostic.measurement() else {
            continue;
        };
        if frames
            .diagnostics
            .get(path)
            .is_some_and(|(_, time)| *time == measurement.time)
        {
            continue;
        }
        if frames.diagnostics.len() >= 2048 && !frames.diagnostics.contains_key(path) {
            continue;
        }
        let next = frames.diagnostics.len();
        let entry = frames
            .diagnostics
            .entry(path.into())
            .or_insert((next, measurement.time));
        entry.1 = measurement.time;
        // View names may originate in mods. Keep a stable numeric series ID and
        // only allowlisted engine pass/measurement labels in the exported name.
        if let Some(label) = render_label(path, entry.0) {
            capture.counter(&label, json!({"value":measurement.value}));
        }
    }
}

fn render_label(path: &str, id: usize) -> Option<String> {
    let field = path.rsplit('/').next()?;
    if !matches!(
        field,
        "elapsed_cpu"
            | "elapsed_gpu"
            | "vertex_shader_invocations"
            | "clipper_invocations"
            | "clipper_primitives_out"
            | "fragment_shader_invocations"
            | "compute_shader_invocations"
    ) {
        return None;
    }
    let pass = path
        .split('/')
        .find(|s| {
            matches!(
                *s,
                "main_opaque_pass_3d"
                    | "main_transparent_pass_3d"
                    | "main_transmissive_pass_3d"
                    | "early_mesh_preprocessing"
                    | "late_mesh_preprocessing"
                    | "retail_exposure_meter"
                    | "retail_exposed_tone"
                    | "prepass"
                    | "early_prepass"
                    | "late_prepass"
                    | "tonemapping"
                    | "upscaling"
                    | "depth_downsample"
                    | "ui"
            )
        })
        .unwrap_or("other_pass");
    Some(format!("render/{id}/{pass}/{field}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_subscriber_rejects_spans() {
        let logs = subscriber::fmt::layer()
            .with_filter(subscriber::filter::FilterFn::new(|m| m.is_event()));
        tracing::subscriber::with_default(subscriber::registry().with(logs), || {
            assert!(tracing::info_span!("disabled_probe").is_disabled());
        });
    }
    #[test]
    fn full_queue_drops_without_blocking() {
        let (sender, _receiver) = sync_channel(1);
        let capture = Capture {
            origin: Instant::now(),
            active: AtomicBool::new(true),
            start: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            sender,
            gpu: false,
            min_us: 25,
            filtered: AtomicU64::new(0),
        };
        for _ in 0..3 {
            capture.counter("test", json!({"value":1}));
        }
        assert_eq!(capture.dropped.load(Ordering::Relaxed), 2);
        assert!(!capture.retain_duration(24));
        assert!(capture.retain_duration(25));
        assert_eq!(capture.filtered.load(Ordering::Relaxed), 1);
    }
    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(std::ffi::OsString::from))
    }
    #[test]
    fn bounds_and_dependencies() {
        for args in [
            vec!["--trace-seconds", "0"],
            vec!["--trace", "a", "--trace-seconds", "601"],
            vec!["--trace", "a", "--trace-delay", "-1"],
            vec!["--trace-gpu"],
            vec!["--trace", "a", "--trace-wait", "--trace-delay", "1"],
            vec!["--trace", "a", "--trace", "b"],
            vec!["--trace", "a", "--trace-min-us", "1001"],
            vec!["--trace-min-us", "25"],
        ] {
            assert!(parse(&args).is_err());
        }
        let options = parse(&["--trace", "a", "--trace-seconds", "600", "--trace-wait"]).unwrap();
        assert!(options.wait);
        assert_eq!(options.seconds, 600);
        assert_eq!(options.min_us, 25);
        assert!(
            parse(&["--player-title", "--trace"])
                .unwrap()
                .path
                .is_none()
        );
    }
    #[test]
    fn viewer_export_wait_stop_and_privacy() {
        let path =
            std::env::temp_dir().join(format!("skate-trace-test-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let guard = init_options(Options {
            path: Some(path.clone()),
            seconds: 1,
            wait: true,
            ..Options::default()
        })
        .unwrap()
        .unwrap();
        assert!(!guard.0.active.load(Ordering::Relaxed));
        guard.0.start.store(true, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !guard.0.active.load(Ordering::Relaxed) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(guard.0.active.load(Ordering::Relaxed));
        // Headless ECS-only app proves real system labels survive release builds.
        fn named_trace_test_system() {}
        let mut app = App::new();
        app.add_systems(Update, named_trace_test_system);
        app.update();
        tracing::info_span!(
            "safe_test_span",
            secret = "CANARY_SECRET",
            path = "C:\\CANARY_PERSONAL\\asset"
        )
        .in_scope(|| tracing::info!("CANARY_LOG"));
        drop(guard);
        let text = std::fs::read_to_string(&path).unwrap();
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert!(
            parsed["traceEvents"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["name"] == "safe_test_span" && e["ph"] == "X")
        );
        assert_eq!(
            parsed["traceEvents"].as_array().unwrap().last().unwrap()["name"],
            "capture_complete"
        );
        assert!(!text.contains("CANARY"));
        assert!(!text.contains("Enable the debug feature"));
        assert!(parsed["traceEvents"].as_array().unwrap().iter().any(|e| {
            e["cat"] == "system"
                && e["name"]
                    .as_str()
                    .is_some_and(|n| n.ends_with("named_trace_test_system"))
        }));
        assert!(
            parsed["traceEvents"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["cat"] == "schedule" && e["name"] == "Update")
        );
        assert_eq!(
            render_label("render/CANARY_SECRET/main_opaque_pass_3d/elapsed_gpu", 7).unwrap(),
            "render/7/main_opaque_pass_3d/elapsed_gpu"
        );
        std::fs::remove_file(path).unwrap();
    }
}
