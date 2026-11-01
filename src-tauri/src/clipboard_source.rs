//! 采集时读取剪贴板附带的上下文：敏感标记、来源应用与浏览器来源页面。
//! 只读系统剪贴板的类型与少量元数据，失败一律当作「没有上下文」，不影响正文采集。
use crate::privacy::CaptureContext;

#[cfg(target_os = "macos")]
pub fn read() -> CaptureContext {
    use crate::capture::{is_concealed_type, MAC_SOURCE_APP_TYPE, MAC_SOURCE_URL_TYPE};
    use objc2_app_kit::{NSPasteboard, NSRunningApplication, NSWorkspace};
    use objc2_foundation::NSString;

    objc2::rc::autoreleasepool(|_| {
        let pasteboard = NSPasteboard::generalPasteboard();
        let types: Vec<String> = pasteboard
            .types()
            .map(|types| types.iter().map(|t| t.to_string()).collect())
            .unwrap_or_default();
        let concealed = types.iter().any(|name| is_concealed_type(name));
        let string_for = |name: &str| {
            types
                .iter()
                .any(|t| t == name)
                .then(|| pasteboard.stringForType(&NSString::from_str(name)))
                .flatten()
                .map(|value| value.to_string())
        };
        let own_bundle = NSRunningApplication::currentApplication()
            .bundleIdentifier()
            .map(|id| id.to_string());
        // 复制方自报的 bundle ID 最准确；否则用当前前台应用近似。
        let declared = string_for(MAC_SOURCE_APP_TYPE).and_then(|bundle| {
            NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(
                &bundle,
            ))
            .iter()
            .next()
        });
        let app = declared
            .or_else(|| NSWorkspace::sharedWorkspace().frontmostApplication())
            .filter(|app| app.bundleIdentifier().map(|id| id.to_string()) != own_bundle)
            .and_then(|app| app.localizedName())
            .map(|name| name.to_string());
        CaptureContext {
            concealed,
            app,
            origin_url: string_for(MAC_SOURCE_URL_TYPE),
        }
    })
}

#[cfg(target_os = "windows")]
pub fn read() -> CaptureContext {
    use crate::capture::{html_source_url, windows_app_name};
    use windows_sys::Win32::Foundation::{CloseHandle, HWND};
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, GetClipboardOwner, IsClipboardFormatAvailable,
        OpenClipboard, RegisterClipboardFormatW,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };

    fn format(name: &str) -> u32 {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        unsafe { RegisterClipboardFormatW(wide.as_ptr()) }
    }
    /// 剪贴板已打开时读取一种格式的原始字节，最多 `limit` 字节。
    unsafe fn data(format: u32, limit: usize) -> Option<Vec<u8>> {
        if format == 0 || IsClipboardFormatAvailable(format) == 0 {
            return None;
        }
        let handle = GetClipboardData(format);
        if handle.is_null() {
            return None;
        }
        let size = GlobalSize(handle).min(limit);
        let pointer = GlobalLock(handle) as *const u8;
        if pointer.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(pointer, size).to_vec();
        GlobalUnlock(handle);
        Some(bytes)
    }
    fn process_name(window: HWND) -> Option<String> {
        if window.is_null() {
            return None;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        if pid == 0 || pid == unsafe { GetCurrentProcessId() } {
            return None;
        }
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut buffer = [0u16; 1024];
            let mut length = buffer.len() as u32;
            let ok = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                buffer.as_mut_ptr(),
                &mut length,
            );
            CloseHandle(process);
            (ok != 0)
                .then(|| String::from_utf16_lossy(&buffer[..length as usize]))
                .and_then(|path| windows_app_name(&path))
        }
    }

    // 「不进入剪贴板历史 / 云剪贴板」的系统约定，密码管理器会设置。
    let exclude = format("ExcludeClipboardContentFromMonitorProcessing");
    let viewer_ignore = format("Clipboard Viewer Ignore");
    let history = format("CanIncludeInClipboardHistory");
    let html = format("HTML Format");
    let mut concealed = [exclude, viewer_ignore]
        .iter()
        .any(|&f| f != 0 && unsafe { IsClipboardFormatAvailable(f) } != 0);
    let mut origin_url = None;
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) != 0 {
            if let Some(value) = data(history, 4) {
                concealed |= value.len() == 4 && value.iter().all(|b| *b == 0);
            }
            // 只需要头部：SourceURL 在 HTML 正文之前。
            if let Some(bytes) = data(html, 4096) {
                origin_url = html_source_url(&String::from_utf8_lossy(&bytes));
            }
            CloseClipboard();
        }
    }
    let app = process_name(unsafe { GetClipboardOwner() })
        .or_else(|| process_name(unsafe { GetForegroundWindow() }));
    CaptureContext {
        concealed,
        app,
        origin_url,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn read() -> CaptureContext {
    CaptureContext::default()
}
