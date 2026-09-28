//! GPU-backed regression test for the local lightmap binding cache.
use super::*;
use bevy_render::{render_resource::*, renderer::initialize_renderer, settings::*};
use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};

fn block_on<T>(future: impl Future<Output = T>) -> T {
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}

#[test]
#[ignore = "requires a Vulkan GPU; run explicitly with --ignored"]
fn cache_tracks_binding_identity_and_texture_replacement() {
    let resources = block_on(initialize_renderer(
        Backends::VULKAN,
        None,
        &WgpuSettings {
            instance_flags: InstanceFlags::empty(),
            ..Default::default()
        },
    ));
    let device = resources.0;
    let queue = resources.1;
    let buffer = || {
        device.create_buffer(&BufferDescriptor {
            label: None,
            size: 1024,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    let a = buffer();
    let b = buffer();
    fn binding(buffer: &Buffer, offset: u64, size: u64) -> BindingResource<'_> {
        BindingResource::Buffer(BufferBinding {
            buffer,
            offset,
            size: BufferSize::new(size),
        })
    }
    let layout = device.create_bind_group_layout(None, &[]);
    let different_layout = device.create_bind_group_layout(None, &[]);
    let group = device.create_bind_group(None, &layout, &[]);
    let mut table = crate::render::mesh::PhaseLightmapCache::default();
    table.remember(&binding(&a, 0, 256), layout.id(), 7);
    assert!(table.matches(&binding(&a.clone(), 0, 256), layout.id(), 7));
    assert!(!table.matches(&binding(&a, 0, 256), layout.id(), 8));
    assert!(!table.matches(&binding(&b, 0, 256), layout.id(), 7));
    assert!(!table.matches(&binding(&a, 256, 256), layout.id(), 7));
    assert!(!table.matches(&binding(&a, 0, 512), layout.id(), 7));
    assert!(!table.matches(&binding(&a, 0, 256), different_layout.id(), 7));
    table.remember(&binding(&b, 0, 256), layout.id(), 7);
    // Bevy exchanges its per-phase buffers every frame. Both allocations must
    // remain reusable, rather than evicting one another on every camera turn.
    for _ in 0..10 {
        assert!(table.select(&binding(&a, 0, 256), layout.id(), 7));
        assert!(table.select(&binding(&b, 0, 256), layout.id(), 7));
    }
    assert!(!table.select(&binding(&a, 0, 256), layout.id(), 8));
    let extent = Extent3d {
        width: 1,
        height: 1,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&TextureDescriptor {
        label: None,
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let image = GpuImage {
        texture_view: texture.create_view(&TextureViewDescriptor::default()),
        texture,
        texture_format: TextureFormat::Rgba8Unorm,
        texture_view_format: None,
        sampler: device.create_sampler(&SamplerDescriptor::default()),
        size: extent,
        mip_level_count: 1,
        had_data: false,
    };
    crate::render::mesh::check_phase_mesh_cache(&device, &a, &b, &image.texture_view);
    let fallback = FallbackImage {
        d1: image.clone(),
        d2: image.clone(),
        d2_array: image.clone(),
        cube: image.clone(),
        cube_array: image.clone(),
        d3: image.clone(),
    };
    // Adding/removing morph targets must refresh the compact mesh list.
    let source = bevy_mesh::Mesh::new(
        PrimitiveTopology::TriangleList,
        bevy_asset::RenderAssetUsages::default(),
    );
    let mut layouts = bevy_mesh::MeshVertexBufferLayouts::default();
    let mesh_layout = source.get_mesh_vertex_buffer_layout(&mut layouts);
    let mesh = |morph| bevy_render::mesh::RenderMesh {
        vertex_count: 0,
        morph_targets: morph,
        buffer_info: bevy_render::mesh::RenderMeshBufferInfo::NonIndexed,
        key_bits: bevy_mesh::BaseMeshPipelineKey::empty(),
        layout: mesh_layout.clone(),
    };
    let static_id = AssetId::Uuid {
        uuid: bevy_asset::uuid::Uuid::from_u128(1),
    };
    let morph_id = AssetId::Uuid {
        uuid: bevy_asset::uuid::Uuid::from_u128(2),
    };
    let mut render_meshes = RenderAssets::default();
    render_meshes.insert(static_id, mesh(None));
    render_meshes.insert(morph_id, mesh(Some(image.texture_view.clone())));
    let mut morph_cache = crate::render::mesh::MorphMeshCache::default();
    assert_eq!(morph_cache.refresh(&render_meshes), &[morph_id]);
    render_meshes.insert(morph_id, mesh(None));
    assert!(morph_cache.refresh(&render_meshes).is_empty());
    render_meshes.insert(static_id, mesh(Some(image.texture_view.clone())));
    assert_eq!(morph_cache.refresh(&render_meshes), &[static_id]);
    render_meshes.remove(static_id);
    assert!(morph_cache.refresh(&render_meshes).is_empty());
    // Exercise both single-texture and binding-array slab storage.
    for bindless in [false, true] {
        let mut slab = LightmapSlab::new(&fallback, bindless);
        slab.cache_bind_group(&binding(&a, 0, 256), layout.id(), group.clone());
        assert_eq!(
            slab.cached_bind_group(&binding(&a.clone(), 0, 256), layout.id())
                .unwrap()
                .id(),
            group.id()
        );
        // Updating buffer contents must keep the binding valid.
        queue.write_buffer(&a, 0, &[0; 16]);
        assert!(
            slab.cached_bind_group(&binding(&a, 0, 256), layout.id())
                .is_some()
        );
        assert!(
            slab.cached_bind_group(&binding(&b, 0, 256), layout.id())
                .is_none()
        );
        assert!(
            slab.cached_bind_group(&binding(&a, 256, 256), layout.id())
                .is_none()
        );
        assert!(
            slab.cached_bind_group(&binding(&a, 0, 512), layout.id())
                .is_none()
        );
        assert!(
            slab.cached_bind_group(&binding(&a, 0, 256), different_layout.id())
                .is_none()
        );
        slab.insert(0u32.into(), image.clone());
        assert!(
            slab.cached_bind_group(&binding(&a, 0, 256), layout.id())
                .is_none()
        );
        slab.cache_bind_group(&binding(&a, 0, 256), layout.id(), group.clone());
        slab.remove(&fallback, 0u32.into());
        assert!(
            slab.cached_bind_group(&binding(&a, 0, 256), layout.id())
                .is_none()
        );
        // Changing buffers repeatedly must not retain an unbounded history.
        for _ in 0..32 {
            slab.cache_bind_group(&binding(&buffer(), 0, 256), layout.id(), group.clone());
        }
        assert_eq!(slab.cached_bind_groups.len(), 8);
    }
    // Exercise the real binding constructor, not only cache lookups.
    let adapter = resources.3;
    let bindless = binding_arrays_are_usable(&device, &adapter);
    let layouts = crate::render::MeshLayouts::new(&device, &adapter);
    let pipeline_cache = PipelineCache::new(device.clone(), adapter, true);
    let mut slab = LightmapSlab::new(&fallback, bindless);
    let first = layouts.lightmapped(
        &device,
        &pipeline_cache,
        &binding(&a, 0, 256),
        &mut slab,
        bindless,
    );
    let reused = layouts.lightmapped(
        &device,
        &pipeline_cache,
        &binding(&a, 0, 256),
        &mut slab,
        bindless,
    );
    assert_eq!(first.id(), reused.id());
    let resized = layouts.lightmapped(
        &device,
        &pipeline_cache,
        &binding(&b, 0, 256),
        &mut slab,
        bindless,
    );
    assert_ne!(first.id(), resized.id());
    slab.insert(0u32.into(), image);
    let replaced = layouts.lightmapped(
        &device,
        &pipeline_cache,
        &binding(&a, 0, 256),
        &mut slab,
        bindless,
    );
    assert_ne!(first.id(), replaced.id());
}
