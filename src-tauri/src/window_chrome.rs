//! Native title bar colored like the app's toolbar. The window theme is forced to light in
//! `tauri.conf.json` (all platforms); on Windows 11 the caption is additionally painted with the exact
//! toolbar color through DWM. Older Windows versions ignore the attribute and keep the light caption.

/// Top color of the toolbar gradient (`--chrome-top` in tokens.css), as COLORREF (0x00BBGGRR).
#[cfg(windows)]
const CAPTION: u32 = 0x00_FB_FA_F9;
/// Title text color (`--text`).
#[cfg(windows)]
const CAPTION_TEXT: u32 = 0x00_2C_23_1C;
/// Window border color (`--border`).
#[cfg(windows)]
const BORDER: u32 = 0x00_E0_D9_D3;

#[cfg(windows)]
pub fn match_theme(window: &tauri::WebviewWindow) {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DwmSetWindowAttribute};
    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = hwnd.0 as windows_sys::Win32::Foundation::HWND;
    for (attribute, color) in [(DWMWA_CAPTION_COLOR, CAPTION), (DWMWA_TEXT_COLOR, CAPTION_TEXT), (DWMWA_BORDER_COLOR, BORDER)] {
        // SAFETY: `hwnd` is the live window handle and `color` outlives the call.
        unsafe {
            DwmSetWindowAttribute(hwnd, attribute as u32, (&color as *const u32).cast(), size_of::<u32>() as u32);
        }
    }
}

#[cfg(not(windows))]
pub fn match_theme(_window: &tauri::WebviewWindow) {}
