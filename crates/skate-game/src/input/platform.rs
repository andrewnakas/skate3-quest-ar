//! Windows device transport. Raw signed axes/trigger bytes reach the TU3
//! converter without Bevy/gilrs deadzones or normalized-axis reconstruction.
use skate_core::input::xbox::XboxState;

pub(crate) struct DevicePacket {
    pub number: u32,
    pub state: XboxState,
    pub subtype: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceError {
    Disconnected,
    State(u32),
    Capabilities(u32),
    #[cfg(not(windows))]
    UnsupportedPlatform,
}

/// Device identity is metadata; raw input is still sampled every host frame.
/// Refresh periodically as well as after errors, so hot swaps cannot leave a
/// subtype cached indefinitely even if Windows never exposes a disconnect.
#[derive(Default)]
pub(crate) struct CapabilityCache {
    value: Option<(u8, std::time::Instant)>,
}
impl CapabilityCache {
    pub(crate) fn invalidate(&mut self) {
        self.value = None;
    }
    fn get(
        &mut self,
        now: std::time::Instant,
        read: impl FnOnce() -> Result<u8, DeviceError>,
    ) -> Result<u8, DeviceError> {
        if let Some((subtype, expires)) = self.value {
            if now < expires {
                return Ok(subtype);
            }
        }
        self.value = None;
        let subtype = read()?;
        self.value = Some((subtype, now + std::time::Duration::from_secs(1)));
        Ok(subtype)
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::mem::MaybeUninit;

    // ABI from the installed Windows SDK Xinput.h. No OS-owned pointers are
    // retained and only successful calls permit reading output storage.
    #[repr(C)]
    struct Gamepad {
        buttons: u16,
        left_trigger: u8,
        right_trigger: u8,
        left_x: i16,
        left_y: i16,
        right_x: i16,
        right_y: i16,
    }
    #[repr(C)]
    struct State {
        number: u32,
        gamepad: Gamepad,
    }
    #[repr(C)]
    struct Vibration {
        left: u16,
        right: u16,
    }
    #[repr(C)]
    struct Capabilities {
        device_type: u8,
        subtype: u8,
        flags: u16,
        gamepad: Gamepad,
        vibration: Vibration,
    }
    const _: () = assert!(size_of::<Gamepad>() == 12);
    const _: () = assert!(size_of::<State>() == 16);
    const _: () = assert!(size_of::<Capabilities>() == 20);

    #[link(name = "xinput")]
    unsafe extern "system" {
        fn XInputGetState(index: u32, state: *mut State) -> u32;
        fn XInputGetCapabilities(index: u32, flags: u32, capabilities: *mut Capabilities) -> u32;
    }

    pub(super) fn poll(
        index: u32,
        cache: &mut CapabilityCache,
    ) -> Result<DevicePacket, DeviceError> {
        let mut state = MaybeUninit::<State>::uninit();
        // SAFETY: properly aligned writable storage with the SDK's exact C ABI.
        let result = unsafe { XInputGetState(index, state.as_mut_ptr()) };
        if result != 0 {
            cache.invalidate();
        }
        if result == 1167 {
            return Err(DeviceError::Disconnected);
        }
        if result != 0 {
            return Err(DeviceError::State(result));
        }
        let subtype = cache.get(std::time::Instant::now(), || {
            let mut capabilities = MaybeUninit::<Capabilities>::uninit();
            // SAFETY: writable storage with the SDK ABI; read only on success.
            let result = unsafe { XInputGetCapabilities(index, 1, capabilities.as_mut_ptr()) };
            if result != 0 {
                return Err(DeviceError::Capabilities(result));
            }
            Ok(unsafe { capabilities.assume_init() }.subtype)
        })?;
        // SAFETY: successful XInputGetState initialized the complete structure.
        let state = unsafe { state.assume_init() };
        Ok(DevicePacket {
            number: state.number,
            state: XboxState {
                buttons: state.gamepad.buttons,
                triggers: [state.gamepad.left_trigger, state.gamepad.right_trigger],
                left: [state.gamepad.left_x, state.gamepad.left_y],
                right: [state.gamepad.right_x, state.gamepad.right_y],
            },
            subtype,
        })
    }
}

pub(crate) fn poll_cached(
    index: usize,
    cache: &mut CapabilityCache,
) -> Result<DevicePacket, DeviceError> {
    assert!(index < 4);
    #[cfg(windows)]
    return windows::poll(index as u32, cache);
    #[cfg(not(windows))]
    Err(DeviceError::UnsupportedPlatform)
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn capability_cache_refreshes_and_never_caches_errors() {
        let start = std::time::Instant::now();
        let mut cache = CapabilityCache::default();
        assert_eq!(cache.get(start, || Ok(1)), Ok(1));
        assert_eq!(
            cache.get(start + std::time::Duration::from_millis(999), || panic!(
                "redundant capability query"
            )),
            Ok(1)
        );
        assert_eq!(
            cache.get(start + std::time::Duration::from_secs(1), || Ok(2)),
            Ok(2)
        );
        cache.invalidate();
        assert_eq!(
            cache.get(start, || Err(DeviceError::Capabilities(5))),
            Err(DeviceError::Capabilities(5))
        );
        assert_eq!(cache.get(start, || Ok(3)), Ok(3));
        cache.invalidate();
        assert_eq!(cache.get(start, || Ok(4)), Ok(4));
    }
}

// Preserve the uncached API for menu-only polling.
pub(crate) fn poll(index: usize) -> Result<DevicePacket, DeviceError> {
    poll_cached(index, &mut CapabilityCache::default())
}
