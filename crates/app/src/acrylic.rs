//! Frosted blur behind the window on Windows.
//!
//! This calls `SetWindowCompositionAttribute` from user32 directly. The documented
//! alternatives do not give see-through glass on Windows 11: the system backdrop
//! (`DWMWA_SYSTEMBACKDROP_TYPE`) is almost opaque and goes solid when the window is
//! inactive, and the plain blur accent renders as a dark opaque layer.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::ffi::c_void;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    /// Tint as 0xAABBGGRR.
    gradient_color: u32,
    animation_id: u32,
}

#[repr(C)]
struct CompositionAttributeData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

const WCA_ACCENT_POLICY: u32 = 19;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

type SetWindowCompositionAttribute = unsafe extern "system" fn(*mut c_void, *mut CompositionAttributeData) -> i32;

/// Turns on the acrylic blur with the given tint. Returns false when Windows does not offer it.
pub fn enable(window: &impl HasWindowHandle, [r, g, b, a]: [u8; 4]) -> bool {
    let Ok(RawWindowHandle::Win32(handle)) = window.window_handle().map(|h| h.as_raw()) else { return false };
    let mut policy = AccentPolicy {
        state: ACCENT_ENABLE_ACRYLICBLURBEHIND,
        flags: 2,
        gradient_color: u32::from_le_bytes([r, g, b, a]),
        animation_id: 0,
    };
    let mut data = CompositionAttributeData {
        attribute: WCA_ACCENT_POLICY,
        data: (&mut policy as *mut AccentPolicy).cast(),
        size: std::mem::size_of::<AccentPolicy>(),
    };
    // SAFETY: user32 is always loaded in a GUI process, and both names are NUL-terminated.
    let function = unsafe { GetProcAddress(GetModuleHandleA(c"user32.dll".as_ptr().cast()), c"SetWindowCompositionAttribute".as_ptr().cast()) };
    let Some(function) = function else { return false };
    // SAFETY: the export has had this signature since Windows 10; `handle` is this process's live
    // window, and `data` points at `policy`, which outlives the call. The function only reads them.
    unsafe {
        let function: SetWindowCompositionAttribute = std::mem::transmute(function);
        function(handle.hwnd.get() as *mut c_void, &mut data) != 0
    }
}
