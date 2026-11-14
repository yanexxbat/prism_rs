//! Windows-only window effects: the optional acrylic blur behind the window, and clipping
//! the window to the glass panel so its corners and margin are really transparent.
//!
//! The acrylic call is `SetWindowCompositionAttribute` from user32, used directly. The
//! documented alternatives do not give see-through glass on Windows 11: the system backdrop
//! (`DWMWA_SYSTEMBACKDROP_TYPE`) is almost opaque and goes solid when the window is
//! inactive, and the plain blur accent renders as a dark opaque layer.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::ffi::c_void;
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, CombineRgn, CreateEllipticRgn, CreateRoundRectRgn, DeleteObject, SetWindowRgn, RGN_OR};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;

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
const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

type SetWindowCompositionAttribute = unsafe extern "system" fn(*mut c_void, *mut CompositionAttributeData) -> i32;

fn hwnd(window: &impl HasWindowHandle) -> Option<HWND> {
    match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => Some(handle.hwnd.get() as HWND),
        _ => None,
    }
}

/// Turns the acrylic blur on with the given RGBA tint, or off with `None`.
/// Returns false when Windows does not offer it.
pub fn set(window: &impl HasWindowHandle, tint: Option<[u8; 4]>) -> bool {
    let Some(hwnd) = hwnd(window) else { return false };
    let mut policy = AccentPolicy {
        state: if tint.is_some() { ACCENT_ENABLE_ACRYLICBLURBEHIND } else { ACCENT_DISABLED },
        flags: 2,
        gradient_color: u32::from_le_bytes(tint.unwrap_or_default()),
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
    // SAFETY: the export has had this signature since Windows 10; `hwnd` is this process's live
    // window, and `data` points at `policy`, which outlives the call. The function only reads them.
    unsafe {
        let function: SetWindowCompositionAttribute = std::mem::transmute(function);
        function(hwnd, &mut data) != 0
    }
}

/// Clips the window to a rounded panel and circles, so clicks outside them reach whatever is
/// behind. `shape` is client pixels: the panel's left, top, right, bottom and corner radius,
/// then left, top, right, bottom of each circle's box. Empty removes the clip (maximised).
pub fn clip(window: &impl HasWindowHandle, shape: &[i32]) {
    let Some(hwnd) = hwnd(window) else { return };
    // SAFETY: plain GDI calls on this process's live window. Every region created here is
    // either combined and deleted, or handed to SetWindowRgn, which then owns it.
    unsafe {
        if shape.len() < 5 {
            SetWindowRgn(hwnd, std::ptr::null_mut(), 1);
            return;
        }
        // Regions are in window coordinates; the client area may start a little inside.
        let mut window_rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let mut origin = POINT { x: 0, y: 0 };
        GetWindowRect(hwnd, &mut window_rect);
        ClientToScreen(hwnd, &mut origin);
        let (dx, dy) = (origin.x - window_rect.left, origin.y - window_rect.top);
        let [l, t, r, b, radius] = [shape[0] + dx, shape[1] + dy, shape[2] + dx, shape[3] + dy, shape[4]];
        let region = CreateRoundRectRgn(l, t, r + 1, b + 1, 2 * radius, 2 * radius);
        for circle in shape[5..].as_chunks::<4>().0 {
            let part = CreateEllipticRgn(circle[0] + dx, circle[1] + dy, circle[2] + dx + 1, circle[3] + dy + 1);
            CombineRgn(region, region, part, RGN_OR);
            DeleteObject(part);
        }
        if SetWindowRgn(hwnd, region, 1) == 0 {
            DeleteObject(region);
        }
    }
}
