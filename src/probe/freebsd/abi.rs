//! Bindings to existing read-only FreeBSD ACPI/DRM ioctls. A native-header
//! regression test keeps the layouts and request sizes honest.

use std::io;
use std::os::fd::RawFd;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BatteryInfo {
    pub cap: i32,
    pub minutes: i32,
    pub state: i32,
    pub rate: i32,
}

#[repr(C)]
pub union BatteryArgument {
    pub unit: i32,
    pub info: BatteryInfo,
    // acpi_battery_ioctl_arg includes the 256-byte acpi_bix, even when
    // GET_BATTINFO only returns its 16-byte battinfo member.
    pub storage: [u32; 64],
}

#[repr(C)]
#[derive(Default)]
pub struct CardResources {
    pub fb_ids: u64,
    pub crtc_ids: u64,
    pub connector_ids: u64,
    pub encoder_ids: u64,
    pub count_fbs: u32,
    pub count_crtcs: u32,
    pub count_connectors: u32,
    pub count_encoders: u32,
    pub min_width: u32,
    pub max_width: u32,
    pub min_height: u32,
    pub max_height: u32,
}

#[repr(C)]
#[derive(Default)]
#[allow(
    clippy::struct_field_names,
    reason = "mirrors struct drm_mode_get_connector"
)]
pub struct Connector {
    pub encoders: u64,
    pub modes: u64,
    pub properties: u64,
    pub property_values: u64,
    pub count_modes: u32,
    pub count_properties: u32,
    pub count_encoders: u32,
    pub encoder_id: u32,
    pub connector_id: u32,
    pub connector_type: u32,
    pub connector_type_id: u32,
    pub connection: u32,
    pub mm_width: u32,
    pub mm_height: u32,
    pub subpixel: u32,
    pub pad: u32,
}

/// `struct drm_mode_modeinfo` is 68 bytes; only its storage is needed.
pub type ModeInfo = [u32; 17];

const OUT: libc::c_ulong = 0x4000_0000;
const IN_OUT: libc::c_ulong = 0xc000_0000;

#[allow(clippy::cast_lossless, clippy::cast_possible_truncation)]
const fn request<T>(direction: libc::c_ulong, group: u8, number: u8) -> libc::c_ulong {
    direction
        | ((size_of::<T>() as libc::c_ulong) << 16)
        | ((group as libc::c_ulong) << 8)
        | number as libc::c_ulong
}

pub const BATTERY_UNITS: libc::c_ulong = request::<i32>(OUT, b'B', 1);
pub const BATTERY_INFO: libc::c_ulong = request::<BatteryArgument>(IN_OUT, b'B', 3);
pub const CARD_RESOURCES: libc::c_ulong = request::<CardResources>(IN_OUT, b'd', 0xa0);
pub const CONNECTOR: libc::c_ulong = request::<Connector>(IN_OUT, b'd', 0xa7);

/// # Safety
/// `value` must be the exact ABI structure for `request`. Any embedded output
/// pointers must remain valid, with their declared capacities, during the call.
pub unsafe fn query<T>(fd: RawFd, request: libc::c_ulong, value: &mut T) -> io::Result<()> {
    // SAFETY: the caller establishes the request-specific ABI and pointer bounds.
    if unsafe { libc::ioctl(fd, request, std::ptr::from_mut(value)) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
