//! The headset's depth (`XR_META_environment_depth`), fused into the park scan while "Scan the
//! park" is on.
//!
//! The runtime renders a depth image of the surroundings (about 320 x 320 a view) several times
//! a second. A few times a second one of them is copied to a host-visible buffer on wgpu's own
//! Vulkan queue (this runs as an exclusive system, so nothing else submits meanwhile; pipelined
//! rendering is off), and when the copy's fence has signalled, every fourth pixel becomes a
//! depth ray from the eye: it carves the cells it passes over and adds its hit to
//! [`ParkScan::depth`].
//!
//! Which way up the image's rows go is found from the scan itself: the first frames are
//! unprojected both ways, and the way that puts points on the floor under a headset looking
//! down wins.
use super::ar_settings::ArSettings;
use super::depth_project::{pose_position, pose_rotation, unproject, InFlight};
use super::scan::{ParkScan, ScanStatus, MAX_PARK_HEIGHT_M};
use ash::vk;
use ash::vk::Handle as _;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use bevy_mod_openxr::{
    exts::OxrEnabledExtensions,
    openxr_session_running,
    resources::{OxrFrameState, OxrInstance},
    session::OxrSession,
};
use bevy_mod_xr::spaces::XrPrimaryReferenceSpace;
use openxr::sys::{self, Handle as _};
use std::ptr;

/// Seconds between depth frames.
const PERIOD_S: f32 = 0.2;
/// Points this close to the head (round it, along the ground) are the player's own body.
const BODY_RADIUS_M: f32 = 0.45;
/// Frames (looking down at the ground) used to decide which way up the rows go.
const ORIENTATION_FRAMES: u32 = 6;
/// Looking down at least this much (the view's forward y) counts for that.
const LOOKING_DOWN: f32 = -0.15;
/// Seconds before trying again after the provider couldn't be made or started (the Scene
/// permission may still be on its way).
const RETRY_S: f32 = 3.0;

pub(super) fn build(app: &mut App) {
    app.add_systems(Last, capture.run_if(openxr_session_running));
}

struct Gpu {
    device: ash::Device,
    queue: vk::Queue,
    pool: vk::CommandPool,
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: *const u16,
}

struct Provider {
    provider: sys::EnvironmentDepthProviderMETA,
    swapchain: sys::EnvironmentDepthSwapchainMETA,
    images: Vec<vk::Image>,
    size: (u32, u32),
    running: bool,
}


#[derive(Default)]
struct DepthState {
    provider: Option<Provider>,
    gpu: Option<Gpu>,
    failed: bool,
    /// Not before this time: the provider failed to come up, try again then.
    retry_at: f32,
    in_flight: Option<InFlight>,
    last: f32,
    /// Floor hits with the rows top-down and bottom-up, until one wins.
    votes: [u32; 2],
    frames_voted: u32,
    flip: Option<bool>,
}

// SAFETY: the raw handles and the mapped pointer are only used from the exclusive system that
// owns this state.
unsafe impl Send for DepthState {}
unsafe impl Sync for DepthState {}

#[derive(Resource, Default)]
struct Depth(DepthState);

fn set_state(world: &mut World, state: impl Into<String>) {
    let state = state.into();
    let mut status = world.resource_mut::<ScanStatus>();
    if status.state != state {
        info!("DEPTH state: {state}");
        status.state = state;
    }
}

fn capture(world: &mut World) {
    if !world.contains_resource::<Depth>() {
        world.init_resource::<Depth>();
    }
    let wanted = world.resource::<ArSettings>().scan_capture;
    let floor_y = {
        let s = world.resource::<ArSettings>();
        if s.free_roam() { s.floor_offset } else { 0.0 }
    };
    let enabled = world.get_resource::<OxrEnabledExtensions>().is_some_and(|e| e.meta_environment_depth);
    let now = world.resource::<Time<Real>>().elapsed_secs();
    world.resource_scope(|world, mut depth: Mut<Depth>| {
        let st = &mut depth.0;
        if !enabled {
            set_state(world, "not available (this headset's runtime has no depth API)");
            return;
        }
        if st.failed {
            return;
        }
        let instance = world.resource::<OxrInstance>().clone();
        let session = world.resource::<OxrSession>().clone();
        let Some(fp) = instance.exts().meta_environment_depth.as_ref() else { return };
        if !wanted {
            if let Some(p) = st.provider.as_mut().filter(|p| p.running) {
                unsafe { (fp.stop_environment_depth_provider)(p.provider) };
                p.running = false;
                info!("DEPTH provider stopped");
            }
            set_state(world, "off (turn on Scan the park)");
            return;
        }
        if now < st.retry_at {
            return;
        }
        if st.provider.is_none() {
            match create_provider(fp, session.as_raw()) {
                Ok(p) => st.provider = Some(p),
                Err(e) => {
                    warn!("DEPTH no depth provider ({e}): trying again");
                    st.retry_at = now + RETRY_S;
                    set_state(world, format!("can't start ({e}); allow Spatial data in the app's permissions"));
                    return;
                }
            }
        }
        if st.gpu.is_none() {
            let (w, h) = st.provider.as_ref().unwrap().size;
            match create_gpu(world.resource::<RenderDevice>(), (w * h) as u64 * 2) {
                Ok(g) => st.gpu = Some(g),
                Err(e) => {
                    warn!("DEPTH no readback ({e}): the park scan can't capture");
                    st.failed = true;
                    set_state(world, format!("failed ({e})"));
                    return;
                }
            }
        }
        let provider = st.provider.as_mut().unwrap();
        if !provider.running {
            let r = unsafe { (fp.start_environment_depth_provider)(provider.provider) };
            if r.into_raw() < 0 {
                warn!("DEPTH start failed: {r:?}: trying again");
                st.retry_at = now + RETRY_S;
                set_state(world, format!("can't start ({r:?}); trying again"));
                return;
            }
            provider.running = true;
            info!("DEPTH provider started ({}x{})", provider.size.0, provider.size.1);
        }
        let gpu = st.gpu.as_ref().unwrap();
        // A finished copy: unproject it into the scan.
        if let Some(job) = st.in_flight.as_ref() {
            let done = unsafe { gpu.device.get_fence_status(gpu.fence) }.unwrap_or(false);
            if !done {
                return;
            }
            let (w, h) = provider.size;
            let pixels = unsafe { std::slice::from_raw_parts(gpu.mapped, (w * h) as usize) };
            let eye = pose_position(job.view.pose);
            let vote = st.flip.is_none();
            let mut scan = world.resource_mut::<ParkScan>();
            let mut rays = 0u64;
            let mut rejected = 0u64;
            for flip in [false, true] {
                if st.flip.is_some_and(|f| f != flip) {
                    continue;
                }
                let points = unproject(pixels, (w, h), job, flip);
                if vote {
                    // Looking down at the ground, the right way up puts the near points on it.
                    let down = (pose_rotation(job.view.pose) * Vec3::NEG_Z).y < LOOKING_DOWN;
                    if down {
                        st.votes[flip as usize] += points.iter().filter(|p| (p.y - floor_y).abs() < 0.12).count() as u32;
                    }
                    continue;
                }
                for p in points {
                    let ground = Vec2::new(p.x - eye.x, p.z - eye.z).length();
                    if ground < BODY_RADIUS_M || p.y > floor_y + MAX_PARK_HEIGHT_M || p.y < floor_y - 2.0 {
                        rejected += 1;
                        continue;
                    }
                    let range = p.distance(eye);
                    let weight = 1.0 / (1.0 + (range / 2.5).powi(2));
                    scan.depth.add_ray(eye, p, weight);
                    rays += 1;
                }
            }
            drop(scan);
            let voting = vote;
            if vote {
                let down = (pose_rotation(job.view.pose) * Vec3::NEG_Z).y < LOOKING_DOWN;
                st.frames_voted += down as u32;
                if st.frames_voted >= ORIENTATION_FRAMES {
                    st.flip = Some(st.votes[1] > st.votes[0]);
                    info!("DEPTH rows {} (floor votes {:?})", if st.votes[1] > st.votes[0] { "bottom-up" } else { "top-down" }, st.votes);
                }
            } else if rays > 0 {
                let mut scan = world.resource_mut::<ParkScan>();
                scan.depth_frames += 1;
                scan.depth_rays += rays;
                scan.revision += 1;
            }
            {
                let mut status = world.resource_mut::<ScanStatus>();
                status.frames += 1;
                status.points += rays;
                status.rejected += rejected;
            }
            if voting && st.flip.is_none() {
                set_state(world, "starting: look down at the ground for a moment");
            } else {
                set_state(world, if rays > 0 { "scanning" } else { "scanning (nothing in range: look at the ground within 4 m)" });
            }
            unsafe { gpu.device.reset_fences(&[gpu.fence]).ok() };
            st.in_flight = None;
        }
        if now - st.last < PERIOD_S {
            return;
        }
        let Some(frame) = world.get_resource::<OxrFrameState>() else { return };
        let space = sys::Space::from_raw(world.resource::<XrPrimaryReferenceSpace>().as_raw());
        let info = sys::EnvironmentDepthImageAcquireInfoMETA {
            ty: sys::EnvironmentDepthImageAcquireInfoMETA::TYPE,
            next: ptr::null(),
            space,
            display_time: frame.predicted_display_time,
        };
        let mut image = sys::EnvironmentDepthImageMETA {
            ty: sys::EnvironmentDepthImageMETA::TYPE,
            next: ptr::null(),
            swapchain_index: 0,
            near_z: 0.0,
            far_z: 0.0,
            views: [view_out(), view_out()],
        };
        let r = unsafe { (fp.acquire_environment_depth_image)(provider.provider, &info, &mut image) };
        if r != sys::Result::SUCCESS {
            // Not ready yet (it takes a moment after starting), or lost tracking.
            if world.resource::<ScanStatus>().frames == 0 {
                set_state(world, format!("waiting for the first depth image ({r:?})"));
            }
            return;
        }
        st.last = now;
        let Some(&src) = provider.images.get(image.swapchain_index as usize) else { return };
        if let Err(e) = unsafe { copy(gpu, src, provider.size) } {
            warn!("DEPTH copy failed: {e:?}");
            st.failed = true;
            return;
        }
        st.in_flight = Some(InFlight { near: image.near_z, far: image.far_z, view: image.views[0] });
    });
}

fn view_out() -> sys::EnvironmentDepthImageViewMETA {
    sys::EnvironmentDepthImageViewMETA {
        ty: sys::EnvironmentDepthImageViewMETA::TYPE,
        next: ptr::null(),
        fov: sys::Fovf { angle_left: 0.0, angle_right: 0.0, angle_up: 0.0, angle_down: 0.0 },
        pose: sys::Posef::IDENTITY,
    }
}

fn create_provider(fp: &openxr::raw::EnvironmentDepthMETA, session: sys::Session) -> Result<Provider, String> {
    let check = |what: &str, r: sys::Result| if r.into_raw() < 0 { Err(format!("{what}: {r:?}")) } else { Ok(()) };
    unsafe {
        let info = sys::EnvironmentDepthProviderCreateInfoMETA {
            ty: sys::EnvironmentDepthProviderCreateInfoMETA::TYPE,
            next: ptr::null(),
            create_flags: sys::EnvironmentDepthProviderCreateFlagsMETA::EMPTY,
        };
        let mut provider = sys::EnvironmentDepthProviderMETA::NULL;
        check("create provider", (fp.create_environment_depth_provider)(session, &info, &mut provider))?;
        // Hands out of the depth where the runtime can (else they become lumps in the scan;
        // the body radius filter takes most of them anyway).
        let hands = sys::EnvironmentDepthHandRemovalSetInfoMETA {
            ty: sys::EnvironmentDepthHandRemovalSetInfoMETA::TYPE,
            next: ptr::null(),
            enabled: sys::TRUE,
        };
        let _ = (fp.set_environment_depth_hand_removal)(provider, &hands);
        let sc_info = sys::EnvironmentDepthSwapchainCreateInfoMETA {
            ty: sys::EnvironmentDepthSwapchainCreateInfoMETA::TYPE,
            next: ptr::null(),
            create_flags: sys::EnvironmentDepthSwapchainCreateFlagsMETA::EMPTY,
        };
        let mut swapchain = sys::EnvironmentDepthSwapchainMETA::NULL;
        check("create swapchain", (fp.create_environment_depth_swapchain)(provider, &sc_info, &mut swapchain))?;
        let mut state = sys::EnvironmentDepthSwapchainStateMETA {
            ty: sys::EnvironmentDepthSwapchainStateMETA::TYPE,
            next: ptr::null_mut(),
            width: 0,
            height: 0,
        };
        check("swapchain state", (fp.get_environment_depth_swapchain_state)(swapchain, &mut state))?;
        let mut count = 0;
        check(
            "count images",
            (fp.enumerate_environment_depth_swapchain_images)(swapchain, 0, &mut count, ptr::null_mut()),
        )?;
        let mut images = vec![
            sys::SwapchainImageVulkanKHR {
                ty: sys::SwapchainImageVulkanKHR::TYPE,
                next: ptr::null_mut(),
                image: 0,
            };
            count as usize
        ];
        check(
            "images",
            (fp.enumerate_environment_depth_swapchain_images)(
                swapchain,
                count,
                &mut count,
                images.as_mut_ptr() as *mut sys::SwapchainImageBaseHeader,
            ),
        )?;
        info!("DEPTH swapchain {}x{}, {count} images", state.width, state.height);
        Ok(Provider {
            provider,
            swapchain,
            images: images.iter().map(|i| vk::Image::from_raw(i.image)).collect(),
            size: (state.width, state.height),
            running: false,
        })
    }
}

fn create_gpu(render: &RenderDevice, bytes: u64) -> Result<Gpu, String> {
    unsafe {
        let hal = render
            .wgpu_device()
            .as_hal::<wgpu_hal::api::Vulkan>()
            .ok_or("not a Vulkan device")?;
        let device = hal.raw_device().clone();
        let queue = hal.raw_queue();
        let family = hal.queue_family_index();
        let instance = hal.shared_instance().raw_instance();
        let props = instance.get_physical_device_memory_properties(hal.raw_physical_device());
        let e = |what: &'static str| move |r: vk::Result| format!("{what}: {r:?}");
        let pool = device
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
            .map_err(e("command pool"))?;
        let cmd = device
            .allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
            .map_err(e("command buffer"))?[0];
        let fence = device.create_fence(&vk::FenceCreateInfo::default(), None).map_err(e("fence"))?;
        let buffer = device
            .create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(bytes)
                    .usage(vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
            .map_err(e("buffer"))?;
        let req = device.get_buffer_memory_requirements(buffer);
        let want = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
        let kind = (0..props.memory_type_count)
            .find(|&i| req.memory_type_bits & (1 << i) != 0 && props.memory_types[i as usize].property_flags.contains(want))
            .ok_or("no host-visible memory")?;
        let memory = device
            .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(kind), None)
            .map_err(e("memory"))?;
        device.bind_buffer_memory(buffer, memory, 0).map_err(e("bind"))?;
        let mapped = device.map_memory(memory, 0, bytes, vk::MemoryMapFlags::empty()).map_err(e("map"))? as *const u16;
        Ok(Gpu { device, queue, pool, cmd, fence, buffer, memory, mapped })
    }
}

/// Copies the left view (layer 0) of a depth image into the readback buffer. The runtime hands
/// its depth images over ready to sample, so they go back the way they came.
unsafe fn copy(gpu: &Gpu, image: vk::Image, (w, h): (u32, u32)) -> Result<(), vk::Result> {
    let d = &gpu.device;
    let range = vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::DEPTH)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1);
    let barrier = |from: vk::ImageLayout, to: vk::ImageLayout, src: vk::AccessFlags, dst: vk::AccessFlags| {
        vk::ImageMemoryBarrier::default()
            .old_layout(from)
            .new_layout(to)
            .src_access_mask(src)
            .dst_access_mask(dst)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(range)
    };
    unsafe {
        d.reset_command_buffer(gpu.cmd, vk::CommandBufferResetFlags::empty())?;
        d.begin_command_buffer(
            gpu.cmd,
            &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )?;
        d.cmd_pipeline_barrier(
            gpu.cmd,
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier(
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::AccessFlags::MEMORY_WRITE,
                vk::AccessFlags::TRANSFER_READ,
            )],
        );
        d.cmd_copy_image_to_buffer(
            gpu.cmd,
            image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            gpu.buffer,
            &[vk::BufferImageCopy::default()
                .image_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::DEPTH)
                        .mip_level(0)
                        .base_array_layer(0)
                        .layer_count(1),
                )
                .image_extent(vk::Extent3D { width: w, height: h, depth: 1 })],
        );
        d.cmd_pipeline_barrier(
            gpu.cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier(
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::AccessFlags::TRANSFER_READ,
                vk::AccessFlags::SHADER_READ,
            )],
        );
        d.end_command_buffer(gpu.cmd)?;
        let cmds = [gpu.cmd];
        d.queue_submit(gpu.queue, &[vk::SubmitInfo::default().command_buffers(&cmds)], gpu.fence)
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.wait_for_fences(&[self.fence], true, 1_000_000_000);
            self.device.unmap_memory(self.memory);
            self.device.destroy_buffer(self.buffer, None);
            self.device.free_memory(self.memory, None);
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_command_pool(self.pool, None);
        }
    }
}
