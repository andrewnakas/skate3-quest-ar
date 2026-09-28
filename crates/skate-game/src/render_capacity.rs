//! Keep instance-buffer identities stable as views discover more geometry.
use bevy::{
    pbr::{MeshInputUniform, MeshPipeline, MeshUniform},
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        batching::gpu_preprocessing::{BatchedInstanceBuffers, write_batched_instance_buffers},
        renderer::RenderDevice,
    },
};

pub(crate) struct RenderCapacityPlugin;
impl Plugin for RenderCapacityPlugin {
    fn build(&self, app: &mut App) {
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.add_systems(
                Render,
                reserve_instances
                    .in_set(RenderSystems::PrepareResourcesFlush)
                    .before(write_batched_instance_buffers::<MeshPipeline>),
            );
        }
    }
}

fn capacity(required: usize, limit: usize) -> usize {
    if required == 0 {
        return 0;
    }
    required
        .checked_next_power_of_two()
        .unwrap_or(required)
        .min(limit)
        .max(required)
}

fn reserve_instances(
    device: Res<RenderDevice>,
    buffers: Option<ResMut<BatchedInstanceBuffers<MeshUniform, MeshInputUniform>>>,
) {
    let Some(mut buffers) = buffers else {
        return;
    };
    let limits = device.limits();
    let maximum = limits
        .max_buffer_size
        .min(u64::from(limits.max_storage_buffer_binding_size))
        / std::mem::size_of::<MeshUniform>() as u64;
    for phase in buffers.phase_instance_buffers.values_mut() {
        let required = phase.data_buffer.len();
        if required != 0 {
            // Bevy's subsequent write_buffer preserves the logical count and
            // initializes only the actual instances through GPU preprocessing.
            phase
                .data_buffer
                .reserve(capacity(required, maximum as usize), &device);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn growth_is_bounded_and_never_smaller_than_required() {
        assert_eq!(capacity(0, 100), 0);
        assert_eq!(capacity(33, 100), 64);
        assert_eq!(capacity(64, 100), 64);
        assert_eq!(capacity(65, 100), 100);
        // Leave unsupported requests to Bevy's validation; never truncate data.
        assert_eq!(capacity(101, 100), 101);
        for n in 1..10000 {
            assert!(capacity(n, 10000) >= n);
            assert!(capacity(n, 10000) <= 10000);
        }
    }
}
