//! GPU-only exposure adaptation. Native evaluator, portable luminance sampling.
use bevy::{
    asset::embedded_asset,
    core_pipeline::{
        FullscreenShader,
        core_3d::graph::{Core3d, Node3d},
    },
    ecs::query::QueryItem,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_component::ExtractComponentPlugin,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_graph::{
            NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel, ViewNode, ViewNodeRunner,
        },
        render_resource::{binding_types::*, *},
        renderer::{RenderContext, RenderDevice, RenderQueue},
        view::ViewTarget,
    },
};
use std::{collections::VecDeque, sync::Mutex};

#[derive(Resource, Clone, ExtractResource)]
struct Settings {
    tuning: Vec4,
    timing: Vec4,
}
impl Default for Settings {
    fn default() -> Self {
        // Missing metadata retains the existing fixed exposure.
        Self {
            tuning: Vec4::new(0., 2.5, 2.5, 0.),
            timing: Vec4::ZERO,
        }
    }
}
#[derive(serde::Deserialize)]
struct Authored {
    target_luminance: f32,
    min: f32,
    max: f32,
    damping: f32,
}

pub(super) fn install(app: &mut App) {
    embedded_asset!(app, "retail_exposure.wgsl");
    app.init_resource::<Settings>()
        .add_plugins((
            ExtractResourcePlugin::<Settings>::default(),
            ExtractComponentPlugin::<super::RetailTone>::default(),
        ))
        .add_systems(Startup, load)
        .add_systems(
            PreUpdate,
            load.after(crate::map_transition::MapTransitionSet)
                .run_if(super::world_changed),
        )
        .add_systems(Update, advance);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .add_systems(RenderStartup, initialize)
            .add_systems(Render, upload.in_set(RenderSystems::PrepareResources))
            .add_systems(
                Render,
                prune_bindings.in_set(RenderSystems::PrepareBindGroups),
            )
            .add_render_graph_node::<ViewNodeRunner<ExposureNode>>(Core3d, ExposureLabel)
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::Tonemapping,
                    ExposureLabel,
                    Node3d::EndMainPassPostProcessing,
                ),
            );
    }
}

fn load(
    config: Res<crate::config::Config>,
    retail: Res<super::RetailScene>,
    mut settings: ResMut<Settings>,
) {
    let generation = settings.timing.y + 1.;
    *settings = Settings::default();
    settings.timing.y = generation;
    if !retail.0 {
        return;
    }
    if std::env::var_os("SKATE_FIXED_EXPOSURE").is_some_and(|v| v == "1") {
        info!("RETAIL_EXPOSURE: fixed 2.5 comparison mode");
        return;
    }
    let profile = config
        .map_path
        .as_ref()
        .and_then(|p| p.file_stem())
        .and_then(|name| {
            let bytes =
                std::fs::read(config.asset_root.join("private/exposure-profiles.json")).ok()?;
            let mut profiles: std::collections::BTreeMap<String, Authored> =
                serde_json::from_slice(&bytes).ok()?;
            profiles.remove(&name.to_string_lossy().to_lowercase())
        });
    let fallback = || {
        std::fs::read(config.asset_root.join("private/exposure.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Authored>(&b).ok())
    };
    if let Some(a) = profile.or_else(fallback) {
        if [a.target_luminance, a.min, a.max, a.damping]
            .iter()
            .all(|x| x.is_finite())
            && a.min > 0.
            && a.max >= a.min
            && a.target_luminance > 0.
            && a.damping >= 0.
        {
            settings.tuning = Vec4::new(a.target_luminance, a.min, a.max, a.damping);
            info!(
                "RETAIL_EXPOSURE: authored target={} range={}..{} damping={}; portable GPU meter",
                a.target_luminance, a.min, a.max, a.damping
            );
        }
    }
}
fn advance(time: Res<Time>, mut settings: ResMut<Settings>) {
    settings.timing.x = time.delta_secs().clamp(0., 0.05);
}

#[derive(Resource)]
struct Pipeline {
    compute_layout: BindGroupLayoutDescriptor,
    tone_layout: BindGroupLayoutDescriptor,
    compute: CachedComputePipelineId,
    tone: CachedRenderPipelineId,
    settings: Buffer,
    state: Buffer,
    sampler: Sampler,
    // The buffers, sampler and layouts are immutable for this Pipeline's life.
    // Source views can alternate, resize or belong to different cameras. Keep a
    // bounded cache so retired targets cannot accumulate across map/size changes.
    bindings: Mutex<VecDeque<(TextureViewId, BindGroup, BindGroup)>>,
}

fn initialize(
    mut commands: Commands,
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
    assets: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
) {
    let compute_layout = BindGroupLayoutDescriptor::new(
        "retail exposure meter",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<[Vec4; 2]>(false),
                storage_buffer::<Vec4>(false),
            ),
        ),
    );
    let tone_layout = BindGroupLayoutDescriptor::new(
        "retail exposed tone",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                storage_buffer_read_only::<Vec4>(false),
            ),
        ),
    );
    let compute = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("retail exposure".into()),
        layout: vec![compute_layout.clone()],
        shader: assets.load(
            bevy::asset::AssetPath::from(bevy::asset::embedded_path!("retail_exposure.wgsl"))
                .with_source("embedded"),
        ),
        entry_point: Some("meter".into()),
        ..default()
    });
    let tone = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("retail exposed tone".into()),
        layout: vec![tone_layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: assets.load(
                bevy::asset::AssetPath::from(bevy::asset::embedded_path!("retail_tone.wgsl"))
                    .with_source("embedded"),
            ),
            targets: vec![Some(ColorTargetState {
                format: ViewTarget::TEXTURE_FORMAT_HDR,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    });
    let settings = device.create_buffer(&BufferDescriptor {
        label: Some("retail exposure settings"),
        size: 32,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let state = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("retail exposure state"),
        contents: &bytes([Vec4::new(2.5, 0., 0., 0.)]),
        usage: BufferUsages::STORAGE,
    });
    commands.insert_resource(Pipeline {
        compute_layout,
        tone_layout,
        compute,
        tone,
        settings,
        state,
        sampler: device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..default()
        }),
        bindings: default(),
    });
}
fn bytes<const N: usize>(values: [Vec4; N]) -> Vec<u8> {
    values
        .into_iter()
        .flat_map(|v| v.to_array())
        .flat_map(f32::to_le_bytes)
        .collect()
}
fn upload(settings: Res<Settings>, pipeline: Res<Pipeline>, queue: Res<RenderQueue>) {
    let mut data = [0u8; 32];
    for (chunk, value) in data.chunks_exact_mut(4).zip(
        [settings.tuning, settings.timing]
            .into_iter()
            .flat_map(|v| v.to_array()),
    ) {
        chunk.copy_from_slice(&value.to_le_bytes());
    }
    queue.write_buffer(&pipeline.settings, 0, &data);
}

fn prune_bindings(pipeline: Res<Pipeline>, views: Query<&ViewTarget, With<super::RetailTone>>) {
    // Bind groups retain their textures. Release retired camera/resize targets
    // before rendering, including when there are no exposure views left.
    pipeline.bindings.lock().unwrap().retain(|(id, _, _)| {
        views.iter().any(|view| {
            *id == view.main_texture_view().id() || *id == view.main_texture_other_view().id()
        })
    });
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct ExposureLabel;
#[derive(Default)]
struct ExposureNode;
impl ViewNode for ExposureNode {
    type ViewQuery = (&'static ViewTarget, &'static super::RetailTone);
    fn run<'w>(
        &self,
        _: &mut RenderGraphContext,
        context: &mut RenderContext,
        (view, _): QueryItem<Self::ViewQuery>,
        world: &'w World,
    ) -> Result<(), NodeRunError> {
        let p = world.resource::<Pipeline>();
        let cache = world.resource::<PipelineCache>();
        let (Some(compute), Some(tone)) = (
            cache.get_compute_pipeline(p.compute),
            cache.get_render_pipeline(p.tone),
        ) else {
            return Ok(());
        };
        let post = view.post_process_write();
        use bevy::render::diagnostic::RecordDiagnostics;
        let diagnostics = context.diagnostic_recorder();
        let mut bindings = p.bindings.lock().unwrap();
        let index = if let Some(index) = bindings
            .iter()
            .position(|(id, _, _)| *id == post.source.id())
        {
            index
        } else {
            let meter = context.render_device().create_bind_group(
                "retail meter",
                &cache.get_bind_group_layout(&p.compute_layout),
                &BindGroupEntries::sequential((
                    post.source,
                    &p.sampler,
                    p.settings.as_entire_binding(),
                    p.state.as_entire_binding(),
                )),
            );
            let output = context.render_device().create_bind_group(
                "retail exposed tone",
                &cache.get_bind_group_layout(&p.tone_layout),
                &BindGroupEntries::sequential((
                    post.source,
                    &p.sampler,
                    p.state.as_entire_binding(),
                )),
            );
            if bindings.len() == 8 {
                bindings.pop_front();
            }
            bindings.push_back((post.source.id(), meter, output));
            bindings.len() - 1
        };
        let (_, meter, output) = &bindings[index];
        {
            let mut pass = context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor {
                    label: Some("retail exposure meter"),
                    timestamp_writes: None,
                });
            pass.set_pipeline(compute);
            let span = diagnostics.pass_span(&mut pass, "retail_exposure_meter");
            pass.set_bind_group(0, meter, &[]);
            pass.dispatch_workgroups(1, 1, 1);
            span.end(&mut pass);
        }
        let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("retail exposed tone"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post.destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_render_pipeline(tone);
        let span = diagnostics.pass_span(&mut pass, "retail_exposed_tone");
        pass.set_bind_group(0, output, &[]);
        pass.draw(0..3, 0..1);
        span.end(&mut pass);
        Ok(())
    }
}
