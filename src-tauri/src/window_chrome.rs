//! Window frame. On Windows the native caption is removed and the UI draws its own title bar (`TitleBar.tsx`),
//! whose height and insets do not shrink when the window is maximized; Windows 11 paints the remaining frame
//! border in the app's border color through DWM (older versions ignore the attribute). Other platforms keep the
//! native title bar, with the window theme forced to light in `tauri.conf.json`.

/// Window border color (`--border` in tokens.css), as COLORREF (0x00BBGGRR).
#[cfg(windows)]
const BORDER: u32 = 0x00_E0_D9_D3;

#[cfg(windows)]
pub fn apply(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DwmSetWindowAttribute};
    window.set_decorations(false)?;
    let hwnd = window.hwnd()?.0 as windows_sys::Win32::Foundation::HWND;
    // SAFETY: `hwnd` is the live window handle and `BORDER` outlives the call.
    unsafe {
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, (&BORDER as *const u32).cast(), size_of::<u32>() as u32);
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn apply(_window: &tauri::WebviewWindow) -> tauri::Result<()> {
    Ok(())
}
