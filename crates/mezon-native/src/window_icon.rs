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
    windows::core::Error::new(windows::core::HRESULT(E_INVALIDARG), message)
}

#[cfg(target_os = "windows")]
type LoadIconWithScaleDownFn =
    unsafe extern "system" fn(
        windows::Win32::Foundation::HINSTANCE,
        windows::core::PCWSTR,
        i32,
        i32,
        u32,
    ) -> windows::Win32::UI::WindowsAndMessaging::HICON;

#[cfg(target_os = "windows")]
fn load_icon_with_scale_down_fn() -> Option<LoadIconWithScaleDownFn> {
    use std::sync::OnceLock;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::core::w;

    static LOAD_FN: OnceLock<Option<LoadIconWithScaleDownFn>> = OnceLock::new();
    *LOAD_FN.get_or_init(|| unsafe {
        let user32 = GetModuleHandleW(w!("user32.dll")).ok()?;
        let proc = GetProcAddress(user32, windows::core::s!("LoadIconWithScaleDown"))?;
        Some(std::mem::transmute::<_, LoadIconWithScaleDownFn>(proc))
    })
}

#[cfg(target_os = "windows")]
fn load_app_icon(
    module: windows::Win32::Foundation::HINSTANCE,
    cx: i32,
    cy: i32,
) -> windows::core::Result<windows::Win32::UI::WindowsAndMessaging::HICON> {
    use windows::Win32::UI::WindowsAndMessaging::{HICON, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW};

    let flags = LR_DEFAULTCOLOR.0;
    if let Some(load) = load_icon_with_scale_down_fn() {
        let icon = unsafe { load(module, APP_ICON_RESOURCE_ID, cx, cy, flags) };
        if !icon.0.is_null() {
            return Ok(icon);
        }
    }

    unsafe {
        LoadImageW(
            Some(module),
            APP_ICON_RESOURCE_ID,
            IMAGE_ICON,
            cx,
            cy,
            LR_DEFAULTCOLOR,
        )
        .map(|handle| HICON(handle.0))
    }
}

#[cfg(target_os = "windows")]
fn try_apply_dpi_aware_icons(hwnd: isize) -> windows::core::Result<()> {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows::Win32::UI::WindowsAndMessaging::{
        GCLP_HICON, GCLP_HICONSM, ICON_BIG, ICON_SMALL, SM_CXICON, SM_CXSMICON, SM_CYICON,
        SM_CYSMICON, SendMessageW, SetClassLongPtrW, WM_SETICON,
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
    let big_icon = load_app_icon(module.into(), big_w, big_h)?;
    let small_icon = load_app_icon(module.into(), small_w, small_h)?;

    unsafe {
        SetClassLongPtrW(hwnd, GCLP_HICON, big_icon.0 as isize);
        SetClassLongPtrW(hwnd, GCLP_HICONSM, small_icon.0 as isize);
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
