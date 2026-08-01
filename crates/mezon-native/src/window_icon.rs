#[cfg(target_os = "windows")]
pub fn apply_dpi_aware_icons(hwnd: isize) {
    if hwnd == 0 {
        return;
    }
    if let Err(e) = try_apply_dpi_aware_icons(hwnd) {
        tracing::warn!("Failed to apply DPI-aware window icons: {e}");
    }
}

#[cfg(not(target_os = "windows"))]
pub fn apply_dpi_aware_icons(_hwnd: isize) {}

#[cfg(target_os = "windows")]
const APP_ICON_RESOURCE_ID: windows::core::PCWSTR = windows::core::PCWSTR(1 as _);

#[cfg(target_os = "windows")]
fn invalid_arg(message: &str) -> windows::core::Error {
    const E_INVALIDARG: i32 = 0x80070057_u32 as i32;
    windows::core::Error::new(windows::core::HRESULT(E_INVALIDARG), message.into())
}

#[cfg(target_os = "windows")]
fn try_apply_dpi_aware_icons(hwnd: isize) -> windows::core::Result<()> {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows::Win32::UI::WindowsAndMessaging::{
        GCLP_HICON, GCLP_HICONSM, ICON_BIG, ICON_SMALL, LR_DEFAULTCOLOR, LoadIconWithScaleDown,
        SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, SendMessageW, SetClassLongPtrW, WM_SETICON,
    };

    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        return Err(invalid_arg("GetDpiForWindow returned 0"));
    }

    let big_w = unsafe { GetSystemMetricsForDpi(SM_CXICON, dpi) };
    let big_h = unsafe { GetSystemMetricsForDpi(SM_CYICON, dpi) };
    let small_w = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) };
    let small_h = unsafe { GetSystemMetricsForDpi(SM_CYSMICON, dpi) };
    if big_w <= 0 || big_h <= 0 || small_w <= 0 || small_h <= 0 {
        return Err(invalid_arg(
            "GetSystemMetricsForDpi returned non-positive icon size",
        ));
    }

    let module = unsafe { GetModuleHandleW(None)? };
    let load = |cx: i32, cy: i32| -> windows::core::Result<_> {
        unsafe {
            LoadIconWithScaleDown(module.into(), APP_ICON_RESOURCE_ID, cx, cy, LR_DEFAULTCOLOR)
        }
    };

    let big_icon = load(big_w, big_h)?;
    let small_icon = load(small_w, small_h)?;

    unsafe {
        SetClassLongPtrW(hwnd, GCLP_HICON, big_icon.0 as isize)?;
        SetClassLongPtrW(hwnd, GCLP_HICONSM, small_icon.0 as isize)?;
        SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(big_icon.0 as isize)),
        );
        SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_SMALL as usize)),
            Some(LPARAM(small_icon.0 as isize)),
        );
    }

    Ok(())
}
