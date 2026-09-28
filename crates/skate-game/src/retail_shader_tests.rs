//! Shader composition tests plus opt-in, headless Vulkan regression probes.
//! Bevy vertex interfaces are real; unrelated lighting functions/resources use
//! interface fixtures. These probes do not launch gameplay or the game app.
use naga_oil::compose::{
    ComposableModuleDescriptor, Composer, NagaModuleDescriptor, ShaderDefValue,
};
use std::collections::HashMap;

#[test]
fn material_index_table_matches_shader_resources() {
    use bevy::render::render_resource::{AsBindGroup, BindlessResourceType as R};
    let descriptor = super::RetailWorldMaterial::bindless_descriptor().unwrap();
    assert_eq!(
        descriptor.resources.as_ref(),
        &[
            R::DataBuffer,
            R::Texture2d,
            R::SamplerFiltering,
            R::Texture2d,
            R::SamplerFiltering,
            R::Texture2d,
            R::SamplerFiltering,
            R::Texture2d,
            R::SamplerFiltering,
            R::Texture2d,
            R::SamplerFiltering,
            R::Texture2d,
            R::SamplerFiltering,
            R::Texture2d,
            R::SamplerFiltering,
            R::TextureCube,
            R::Buffer,
        ]
    );
    assert_eq!(descriptor.index_tables.len(), 1);
    let table = &descriptor.index_tables[0];
    assert_eq!(
        (
            table.binding_number.0,
            table.indices.start.0,
            table.indices.end.0
        ),
        (0, 0, 17)
    );
    let mut buffers: Vec<_> = descriptor
        .buffers
        .iter()
        .map(|b| (b.bindless_index.0, b.binding_number.0))
        .collect();
    buffers.sort_unstable();
    assert_eq!(buffers, [(0, 17), (16, 18)]);
}

fn validate(bindless: bool, prepass: bool, extras: &[&str]) -> naga::Module {
    let mut defs = HashMap::from([("MATERIAL_BIND_GROUP".into(), ShaderDefValue::UInt(3))]);
    for &name in [
        "VERTEX_UVS_A",
        "VERTEX_UVS_B",
        "VERTEX_TANGENTS",
        "VERTEX_COLORS",
        "VERTEX_NORMALS",
        "VERTEX_OUTPUT_INSTANCE_INDEX",
    ]
    .iter()
    .chain(extras)
    {
        defs.insert(name.into(), ShaderDefValue::Bool(true));
    }
    if bindless {
        defs.insert("BINDLESS".into(), ShaderDefValue::Bool(true));
    }
    let mut composer = Composer::default().with_capabilities(naga::valid::Capabilities::all());
    let fixtures = [
        (
            "forward",
            include_str!("../../../vendor/bevy_pbr/src/render/forward_io.wgsl"),
        ),
        (
            "prepass",
            include_str!("../../../vendor/bevy_pbr/src/prepass/prepass_io.wgsl"),
        ),
        (
            "mesh",
            "#define_import_path bevy_pbr::mesh_bindings\nstruct Mesh { material_and_lightmap_bind_group_slot: u32 }\n@group(2) @binding(0) var<storage> mesh: array<Mesh>;",
        ),
        (
            "frame",
            "#define_import_path bevy_pbr::mesh_view_bindings\nstruct View {view_from_world:mat4x4<f32>,viewport:vec4<f32>,world_position:vec3<f32>,padding:f32}\nstruct Light {flags:u32}\nstruct Lights {n_directional_lights:u32,directional_lights:array<Light,10>}\n@group(0) @binding(0) var<uniform> view:View;\n@group(0) @binding(1) var<storage> lights:Lights;",
        ),
        (
            "shadows",
            "#define_import_path bevy_pbr::shadows\nfn fetch_directional_shadow(id:u32,p:vec4<f32>,n:vec3<f32>,z:f32)->f32 {return 1.0;}",
        ),
        (
            "motion",
            "#define_import_path bevy_pbr::pbr_prepass_functions\nfn calculate_motion_vector(p:vec4<f32>,q:vec4<f32>)->vec2<f32> {return vec2<f32>(0.0);}",
        ),
    ];
    for (path, source) in fixtures {
        if let Err(error) = composer.add_composable_module(ComposableModuleDescriptor {
            source,
            file_path: path,
            shader_defs: defs.clone(),
            ..Default::default()
        }) {
            panic!("{}", error.emit_to_string(&composer));
        }
    }
    let experiment = std::env::var("SKATE_SHADER_PROBE_SOURCE").ok();
    let probe_source;
    if let Some(ref directory) = experiment {
        let binding_source =
            std::fs::read_to_string(format!("{directory}/v6-retail_material_bindings.wgsl"))
                .unwrap();
        composer
            .add_composable_module(ComposableModuleDescriptor {
                source: &binding_source,
                file_path: "bindings",
                shader_defs: defs.clone(),
                ..Default::default()
            })
            .unwrap();
        probe_source =
            std::fs::read_to_string(format!("{directory}/v6-retail_world.wgsl")).unwrap();
    } else {
        probe_source = String::new();
    }
    if experiment.is_none() {
        composer
            .add_composable_module(ComposableModuleDescriptor {
                source: include_str!("retail_material_bindings.wgsl"),
                file_path: "bindings",
                shader_defs: defs.clone(),
                ..Default::default()
            })
            .unwrap();
    }
    let source = if prepass {
        include_str!("retail_depth.wgsl")
    } else {
        include_str!("retail_world.wgsl")
    };
    let source = if experiment.is_some() {
        &probe_source
    } else {
        source
    };
    let module = composer
        .make_naga_module(NagaModuleDescriptor {
            source,
            file_path: "retail",
            shader_defs: defs,
            ..Default::default()
        })
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&composer)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    module
}

#[test]
fn material_shaders_validate() {
    for bindless in [false, true] {
        validate(bindless, false, &[]);
        validate(bindless, true, &[]);
        validate(
            bindless,
            true,
            &[
                "PREPASS_FRAGMENT",
                "NORMAL_PREPASS",
                "NORMAL_PREPASS_OR_DEFERRED_PREPASS",
                "MOTION_VECTOR_PREPASS",
                "UNCLIPPED_DEPTH_ORTHO_EMULATION",
            ],
        );
    }
}

#[test]
#[ignore = "Explicit headless GPU compiler probe; no game or window"]
fn vulkan_material_pipeline_probe() {
    gpu_probe(false);
}

#[test]
#[ignore = "Explicit headless GPU shadow compiler probe; no game or window"]
fn vulkan_shadow_pipeline_probe() {
    gpu_probe(true);
}

fn gpu_probe(prepass: bool) {
    bevy::tasks::block_on(async {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        eprintln!("PROBE adapter={:?}", adapter.get_info());
        let features = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
            | wgpu::Features::BUFFER_BINDING_ARRAY
            | wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::STORAGE_RESOURCE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
            | wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: features,
                required_limits: adapter.limits(),
                ..Default::default()
            })
            .await
            .unwrap();
        allocation_reuses_resident_textures(&device);
        let module = validate(
            std::env::var_os("SKATE_SHADER_PROBE_FALLBACK").is_none(),
            prepass,
            &[],
        );
        let mut groups: Vec<Vec<wgpu::BindGroupLayoutEntry>> = vec![vec![]; 4];
        for (_, variable) in module.global_variables.iter() {
            let Some(binding) = variable.binding else {
                continue;
            };
            let (ty, count) = match module.types[variable.ty].inner {
                naga::TypeInner::BindingArray { base, .. } => (base, std::num::NonZeroU32::new(64)),
                _ => (variable.ty, None),
            };
            let resource = match module.types[ty].inner {
                naga::TypeInner::Sampler { comparison: false } => {
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
                }
                naga::TypeInner::Image {
                    dim,
                    arrayed: false,
                    ..
                } => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: match dim {
                        naga::ImageDimension::D2 => wgpu::TextureViewDimension::D2,
                        naga::ImageDimension::Cube => wgpu::TextureViewDimension::Cube,
                        _ => panic!("image"),
                    },
                    multisampled: false,
                },
                _ => wgpu::BindingType::Buffer {
                    ty: if variable.space == naga::AddressSpace::Uniform {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage { read_only: true }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            };
            groups[binding.group as usize].push(wgpu::BindGroupLayoutEntry {
                binding: binding.binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: resource,
                count,
            });
        }
        let layouts: Vec<_> = groups
            .iter()
            .map(|entries| {
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: None,
                    entries,
                })
            })
            .collect();
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &layouts.iter().collect::<Vec<_>>(),
            push_constant_ranges: &[],
        });
        let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("probe vertex"),source:wgpu::ShaderSource::Wgsl((if prepass { r#"
struct Out { @builtin(position) position:vec4<f32>, @location(0) uv:vec2<f32>, @location(1) uv_b:vec2<f32>, @location(4) world:vec4<f32>, @location(7) @interpolate(flat) instance:u32, @location(8) color:vec4<f32> }
@vertex fn vertex()->Out {var o:Out; o.position=vec4<f32>(0,0,0,1); return o;}
"# } else { r#"
struct Out { @builtin(position) position:vec4<f32>, @location(0) world:vec4<f32>, @location(1) normal:vec3<f32>, @location(2) uv:vec2<f32>, @location(3) uv_b:vec2<f32>, @location(4) tangent:vec4<f32>, @location(5) color:vec4<f32>, @location(6) @interpolate(flat) instance:u32 }
@vertex fn vertex(@builtin(vertex_index) index:u32,@builtin(instance_index) instance:u32)->Out {var o:Out; let xy=vec2<f32>(f32((index<<1u)&2u),f32(index&2u))*2.0-1.0; o.position=vec4<f32>(xy,0,1); o.world=vec4<f32>(xy,0,1); o.normal=vec3<f32>(0,1,0); o.tangent=vec4<f32>(1,0,0,1); o.uv=(xy+1.0)*0.5; o.uv_b=o.uv; o.instance=instance; return o;}
"# }).into())});
        eprintln!("PROBE create shader");
        let fragment = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("retail probe"),
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
        });
        eprintln!("PROBE create pipeline");
        let color_targets = [Some(wgpu::TextureFormat::Rgba8Unorm.into())];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("retail compiler probe"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: prepass.then_some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: if prepass { 1 } else { 8 },
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: if prepass { &[] } else { &color_targets },
            }),
            multiview: None,
            cache: None,
        });
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        eprintln!("PROBE pipeline compiled");
        if prepass {
            return;
        }
        // Read back two distinct material slots. This exercises indexed buffer
        // and texture resources, queue submission and 8x-MSAA resolve on Vulkan.
        let bindless = std::env::var_os("SKATE_SHADER_PROBE_FALLBACK").is_none();
        let buffer = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 16384,
                usage: wgpu::BufferUsages::UNIFORM
                    | wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let view_buffer = buffer("view");
        let zero_buffer = buffer("zero");
        let mesh_buffer = buffer("mesh");
        let table_buffer = buffer("table");
        let params_buffer = buffer("params");
        let bytes = |values: Vec<f32>| {
            values
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>()
        };
        let mut view = vec![0.0; 24];
        view[19] = 640.0;
        view[22] = 2.0;
        queue.write_buffer(&view_buffer, 0, &bytes(view));
        let mut params = vec![0.0; 52 * 64];
        for slot in 0..64 {
            let row = &mut params[slot * 52..(slot + 1) * 52];
            row[0] = 14.0;
            row[2] = -1.0;
            row[3] = if slot == 0 { 0.25 } else { 0.75 };
            row[18] = 1.0;
            row[37] = 1.0;
        }
        queue.write_buffer(&params_buffer, 0, &bytes(params));
        queue.write_buffer(
            &mesh_buffer,
            0,
            &(0u32..64).flat_map(u32::to_le_bytes).collect::<Vec<_>>(),
        );
        let mut table = vec![0u32; 17 * 64];
        for slot in 0..64 {
            table[slot * 17] = slot as u32;
        }
        queue.write_buffer(
            &table_buffer,
            0,
            &table
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let make_texture = |layers| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let texture = make_texture(1);
        let cube = make_texture(6);
        for (tex, layers) in [(&texture, 1), (&cube, 6)] {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &vec![255u8; 4 * layers as usize],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: layers,
                },
            );
        }
        let texture_view = texture.create_view(&Default::default());
        let cube_view = cube.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        let sampler = device.create_sampler(&Default::default());
        let samplers = vec![&sampler; 64];
        let textures = vec![&texture_view; 64];
        let cubes = vec![&cube_view; 64];
        let frame_buffers = vec![zero_buffer.as_entire_buffer_binding(); 64];
        let mut bind_groups = vec![];
        for (group, entries) in groups.iter().enumerate() {
            let entries: Vec<_> = entries
                .iter()
                .map(|entry| {
                    let resource = match entry.ty {
                        wgpu::BindingType::Sampler(_) if entry.count.is_some() => {
                            wgpu::BindingResource::SamplerArray(&samplers)
                        }
                        wgpu::BindingType::Sampler(_) => wgpu::BindingResource::Sampler(&sampler),
                        wgpu::BindingType::Texture {
                            view_dimension: wgpu::TextureViewDimension::Cube,
                            ..
                        } if entry.count.is_some() => {
                            wgpu::BindingResource::TextureViewArray(&cubes)
                        }
                        wgpu::BindingType::Texture {
                            view_dimension: wgpu::TextureViewDimension::Cube,
                            ..
                        } => wgpu::BindingResource::TextureView(&cube_view),
                        wgpu::BindingType::Texture { .. } if entry.count.is_some() => {
                            wgpu::BindingResource::TextureViewArray(&textures)
                        }
                        wgpu::BindingType::Texture { .. } => {
                            wgpu::BindingResource::TextureView(&texture_view)
                        }
                        _ if entry.count.is_some() => {
                            wgpu::BindingResource::BufferArray(&frame_buffers)
                        }
                        _ => match (group, entry.binding) {
                            (0, 0) => &view_buffer,
                            (2, 0) => &mesh_buffer,
                            (3, 0) if bindless => &table_buffer,
                            (3, 0) | (3, 17) => &params_buffer,
                            _ => &zero_buffer,
                        }
                        .as_entire_binding(),
                    };
                    wgpu::BindGroupEntry {
                        binding: entry.binding,
                        resource,
                    }
                })
                .collect();
            bind_groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &layouts[group],
                entries: &entries,
            }));
        }
        let target = |samples| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | if samples == 1 {
                        wgpu::TextureUsages::COPY_SRC
                    } else {
                        wgpu::TextureUsages::empty()
                    },
                view_formats: &[],
            })
        };
        let msaa = target(8);
        let output = target(1);
        let msaa_view = msaa.create_view(&Default::default());
        let output_view = output.create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 4096,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        for slot in 0..if bindless { 2 } else { 1 } {
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &msaa_view,
                        depth_slice: None,
                        resolve_target: Some(&output_view),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Discard,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&pipeline);
                for (index, group) in bind_groups.iter().enumerate() {
                    pass.set_bind_group(index as u32, group, &[]);
                }
                pass.draw(0..3, slot..slot + 1);
            }
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &output,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(16),
                    },
                },
                wgpu::Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            let slice = readback.slice(..);
            let (send, recv) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap()
            });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            recv.recv().unwrap().unwrap();
            let data = slice.get_mapped_range();
            let pixel = &data[8 * 256 + 8 * 4..8 * 256 + 8 * 4 + 4];
            let expected = if slot == 0 { 64i16 } else { 191 };
            assert!(
                (i16::from(pixel[0]) - expected).abs() <= 1,
                "slot {slot}: {pixel:?}"
            );
            assert_eq!(pixel[3], 255);
            eprintln!("PROBE slot={slot} pixel={pixel:?}");
            drop(data);
            readback.unmap();
        }
    });
}

// Exercise the real allocator against GPU resource identities and retirement.
fn allocation_reuses_resident_textures(device: &wgpu::Device) {
    use bevy::render::render_resource::{
        BindGroupLayoutDescriptor, BindingNumber, BindingResources, BindlessDescriptor,
        BindlessIndex, BindlessIndexTableDescriptor, BindlessResourceType,
        BindlessSlabResourceLimit, OwnedBindingResource, UnpreparedBindGroup,
    };
    let render_device = bevy::render::renderer::RenderDevice::from(device.clone());
    let layout = BindGroupLayoutDescriptor::new("allocation regression", &[]);
    let descriptor = BindlessDescriptor {
        resources: vec![BindlessResourceType::Texture2d; 2].into(),
        buffers: vec![].into(),
        index_tables: vec![BindlessIndexTableDescriptor {
            indices: BindlessIndex(0)..BindlessIndex(2),
            binding_number: BindingNumber(0),
        }]
        .into(),
    };
    let mut allocator = bevy::pbr::MaterialBindGroupAllocator::new(
        &render_device,
        "allocation regression",
        Some(descriptor),
        layout.clone(),
        Some(BindlessSlabResourceLimit::Custom(2)),
    );
    let views: Vec<bevy::render::render_resource::TextureView> = (0..4)
        .map(|_| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
                .into()
        })
        .collect();
    let group = |a: usize, b: usize| UnpreparedBindGroup {
        bindings: BindingResources(vec![
            (
                0,
                OwnedBindingResource::TextureView(wgpu::TextureViewDimension::D2, views[a].clone()),
            ),
            (
                1,
                OwnedBindingResource::TextureView(wgpu::TextureViewDimension::D2, views[b].clone()),
            ),
        ]),
    };
    let first = allocator.allocate_unprepared(group(0, 1), &layout);
    let second = allocator.allocate_unprepared(group(2, 3), &layout);
    assert_ne!(first.group, second.group);
    allocator.free(first);
    let shared = allocator.allocate_unprepared(group(2, 3), &layout);
    assert_eq!(
        shared.group, second.group,
        "reuse resident textures instead of duplicating into an empty earlier slab"
    );
    allocator.free(second);
    allocator.free(shared);
    let fresh = allocator.allocate_unprepared(group(0, 3), &layout);
    assert_eq!(fresh.group, first.group);
    eprintln!("PROBE allocator resource reuse and retirement passed");
}
