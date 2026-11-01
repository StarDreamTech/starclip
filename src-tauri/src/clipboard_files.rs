//! Native clipboard file lists. Only references are handled: no file contents
//! are read, folders are never scanned, and no shell command is ever built.
//!
//! macOS reads/writes `public.file-url` items on `NSPasteboard`; Windows uses
//! `CF_HDROP`. Other platforms keep the existing text/image behaviour and
//! report that file references are unsupported instead of faking success.
use crate::store::{FileEntry, FileEntryKind, Result};
use std::collections::HashSet;
use std::path::Path;

/// A single copy can reference at most this many items.
pub const MAX_ITEMS: usize = 100;
/// Serialized budget for one file list, matching the text record limit.
pub const MAX_BYTES: usize = 256 * 1024;

/// Joined paths are the record identity: same order and same items is the same
/// record. Nothing is derived from file contents or from resolving symlinks.
pub fn content(entries: &[FileEntry]) -> String {
    entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn tidy(path: &str) -> String {
    let mut value = path.to_owned();
    while value.len() > 1 && (value.ends_with('/') || value.ends_with('\\')) {
        let trimmed = &value[..value.len() - 1];
        // Keep Windows drive roots such as "C:\" intact.
        if trimmed.ends_with(':') {
            break;
        }
        value = trimmed.to_owned();
    }
    value
}

fn entry_kind(path: &str) -> FileEntryKind {
    // Metadata only, and never followed through a symlink. A symlink or a
    // missing path stays "unknown" instead of being guessed from the suffix.
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_dir() => FileEntryKind::Directory,
        Ok(meta) if meta.file_type().is_file() => FileEntryKind::File,
        _ => FileEntryKind::Unknown,
    }
}

/// Validates, de-duplicates and bounds a native list. The whole batch is
/// rejected when any path is unusable, so nothing is dropped silently.
pub fn normalize(paths: impl IntoIterator<Item = String>) -> Result<Vec<FileEntry>> {
    let mut entries: Vec<FileEntry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut bytes = 0usize;
    for raw in paths {
        let path = tidy(&raw);
        if path.is_empty()
            || path.contains('\0')
            || path.contains('\n')
            || path.contains('\r')
            || !Path::new(&path).is_absolute()
        {
            return Err("文件路径格式不支持，本次未记录。".into());
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        if entries.len() == MAX_ITEMS {
            return Err("一次最多记录 100 个文件或文件夹，本次未记录。".into());
        }
        bytes = bytes.saturating_add(path.len() + 1);
        if bytes > MAX_BYTES {
            return Err("文件列表超过 256 KiB，本次未记录。".into());
        }
        let kind = entry_kind(&path);
        entries.push(FileEntry {
            path,
            entry_kind: kind,
        });
    }
    if entries.is_empty() {
        return Err("没有可记录的文件。".into());
    }
    Ok(entries)
}

/// `None` means the clipboard holds no native file list at all.
pub fn read() -> Result<Option<Vec<String>>> {
    platform::read()
}

/// Replaces the clipboard with a native file list; never touches the files.
pub fn write(paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Err("没有可复制的文件。".into());
    }
    platform::write(paths)
}

/// Shows the item in the system file manager without building a shell command.
pub fn reveal(path: &str) -> Result<()> {
    platform::reveal(path)
}

/// Paths that no longer exist, checked only on an explicit user action.
pub fn missing(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|path| std::fs::symlink_metadata(path.as_str()).is_err())
        .cloned()
        .collect()
}

#[cfg(all(target_os = "macos", feature = "desktop"))]
mod platform {
    use super::Result;
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeFileURL, NSPasteboardWriting, NSWorkspace};
    use objc2_foundation::{NSArray, NSString, NSURL};

    pub fn read() -> Result<Option<Vec<String>>> {
        let pasteboard = NSPasteboard::generalPasteboard();
        let Some(items) = pasteboard.pasteboardItems() else {
            return Ok(None);
        };
        // SAFETY: the type constant is a static NSString provided by AppKit.
        let file_url = unsafe { NSPasteboardTypeFileURL };
        let mut paths = Vec::new();
        for item in items.iter() {
            let Some(value) = item.stringForType(file_url) else {
                continue;
            };
            // Remote URLs are not local files and are ignored.
            let Some(url) = NSURL::URLWithString(&value) else {
                continue;
            };
            if !url.isFileURL() {
                continue;
            }
            if let Some(path) = url.path() {
                paths.push(path.to_string());
            }
        }
        Ok((!paths.is_empty()).then_some(paths))
    }

    fn urls(paths: &[String]) -> Vec<Retained<NSURL>> {
        paths
            .iter()
            .map(|path| NSURL::fileURLWithPath(&NSString::from_str(path)))
            .collect()
    }

    pub fn write(paths: &[String]) -> Result<()> {
        let urls = urls(paths);
        let objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = urls
            .into_iter()
            .map(ProtocolObject::from_retained)
            .collect();
        let array = NSArray::from_retained_slice(&objects);
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        if !pasteboard.writeObjects(&array) {
            return Err("复制文件失败，请重试。".into());
        }
        Ok(())
    }

    pub fn reveal(path: &str) -> Result<()> {
        let urls = urls(&[path.to_owned()]);
        let array = NSArray::from_retained_slice(&urls);
        NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&array);
        Ok(())
    }
}

#[cfg(all(target_os = "windows", feature = "desktop"))]
mod platform {
    use super::Result;
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData, CF_HDROP,
    };
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalFree, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows_sys::Win32::UI::Shell::{DragQueryFileW, ShellExecuteW, DROPFILES};

    /// Owns the clipboard and always releases it, including on early return.
    struct Clipboard;
    impl Clipboard {
        fn open() -> Result<Self> {
            // A slow owner can hold the clipboard briefly; retry a few times.
            for _ in 0..10 {
                if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                    return Ok(Self);
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err("剪贴板正被其他程序占用，请稍后重试。".into())
        }
    }
    impl Drop for Clipboard {
        fn drop(&mut self) {
            unsafe { CloseClipboard() };
        }
    }

    /// Owns the global memory block until the clipboard takes ownership of it.
    struct Global(Option<*mut c_void>);
    impl Drop for Global {
        fn drop(&mut self) {
            if let Some(handle) = self.0.take() {
                unsafe { GlobalFree(handle) };
            }
        }
    }

    fn hglobal(bytes: &[u8]) -> Result<Global> {
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
        if handle.is_null() {
            return Err("无法为文件列表分配内存，请重试。".into());
        }
        let mut global = Global(Some(handle));
        let pointer = unsafe { GlobalLock(handle) };
        if pointer.is_null() {
            return Err("无法写入文件列表，请重试。".into());
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer as *mut u8, bytes.len());
            GlobalUnlock(handle);
        }
        global.0 = Some(handle);
        Ok(global)
    }

    pub fn read() -> Result<Option<Vec<String>>> {
        let _clipboard = Clipboard::open()?;
        let handle = unsafe { GetClipboardData(CF_HDROP) };
        if handle.is_null() {
            return Ok(None);
        }
        let count = unsafe { DragQueryFileW(handle, u32::MAX, std::ptr::null_mut(), 0) };
        if count == 0 {
            return Ok(None);
        }
        let mut paths = Vec::with_capacity(count as usize);
        for index in 0..count {
            let length = unsafe { DragQueryFileW(handle, index, std::ptr::null_mut(), 0) };
            if length == 0 {
                continue;
            }
            let mut buffer = vec![0u16; length as usize + 1];
            let written =
                unsafe { DragQueryFileW(handle, index, buffer.as_mut_ptr(), buffer.len() as u32) };
            if written == 0 {
                continue;
            }
            paths.push(String::from_utf16_lossy(&buffer[..written as usize]));
        }
        Ok((!paths.is_empty()).then_some(paths))
    }

    pub fn write(paths: &[String]) -> Result<()> {
        let header = std::mem::size_of::<DROPFILES>();
        let mut list: Vec<u16> = Vec::new();
        for path in paths {
            list.extend(path.encode_utf16());
            list.push(0);
        }
        list.push(0);
        let mut bytes = vec![0u8; header + list.len() * 2];
        let drop = DROPFILES {
            pFiles: header as u32,
            pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
            fNC: 0,
            fWide: 1,
        };
        unsafe {
            std::ptr::write(bytes.as_mut_ptr() as *mut DROPFILES, drop);
        }
        for (index, unit) in list.iter().enumerate() {
            let offset = header + index * 2;
            bytes[offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
        }
        let memory = hglobal(&bytes)?;
        let _clipboard = Clipboard::open()?;
        if unsafe { EmptyClipboard() } == 0 {
            return Err("清空剪贴板失败，请重试。".into());
        }
        let handle = memory.0.ok_or("文件列表句柄无效。")?;
        if unsafe { SetClipboardData(CF_HDROP, handle) }.is_null() {
            return Err("复制文件失败，请重试。".into());
        }
        // The clipboard owns the block now; dropping must not free it.
        std::mem::forget(memory);
        Ok(())
    }

    pub fn reveal(path: &str) -> Result<()> {
        let verb: Vec<u16> = "open\0".encode_utf16().collect();
        let file: Vec<u16> = format!("{path}\0").encode_utf16().collect();
        let arguments: Vec<u16> = "/select,\0".encode_utf16().collect();
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut::<c_void>() as HWND,
                verb.as_ptr(),
                file.as_ptr(),
                arguments.as_ptr(),
                std::ptr::null(),
                1,
            )
        };
        if result as isize <= 32 {
            return Err("无法在文件夹中显示该项目。".into());
        }
        Ok(())
    }
}

#[cfg(not(all(any(target_os = "macos", target_os = "windows"), feature = "desktop")))]
mod platform {
    use super::Result;

    pub fn read() -> Result<Option<Vec<String>>> {
        Ok(None)
    }
    pub fn write(_paths: &[String]) -> Result<()> {
        Err("当前平台不支持复制文件引用。".into())
    }
    pub fn reveal(_path: &str) -> Result<()> {
        Err("当前平台不支持在文件夹中显示。".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let directory =
            std::env::temp_dir().join(format!("starclip-files-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        directory.to_string_lossy().into_owned()
    }

    #[test]
    fn normalizes_order_deduplicates_and_keeps_kinds() {
        let directory = fixture("basic");
        let file = format!("{directory}/原创样例.txt");
        std::fs::write(&file, b"original fixture").unwrap();
        let entries = normalize(vec![format!("{directory}/"), file.clone(), file.clone()]).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].entry_kind, FileEntryKind::Directory);
        assert_eq!(entries[0].path, directory);
        assert_eq!(entries[1].entry_kind, FileEntryKind::File);
        assert_eq!(content(&entries), format!("{directory}\n{file}"));
        assert_eq!(std::fs::read(&file).unwrap(), b"original fixture");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn rejects_relative_empty_and_unserializable_paths() {
        for paths in [
            vec!["relative/path".to_string()],
            vec![String::new()],
            vec!["/original/line\nbreak".to_string()],
            vec!["/original/nul\0byte".to_string()],
        ] {
            assert!(normalize(paths).is_err());
        }
        assert!(normalize(Vec::new()).is_err());
    }

    #[test]
    fn enforces_item_and_byte_limits_instead_of_truncating() {
        let mut many: Vec<String> = (0..MAX_ITEMS + 1)
            .map(|index| format!("/original/{index}.txt"))
            .collect();
        assert!(normalize(many.clone()).is_err());
        many.truncate(MAX_ITEMS);
        assert_eq!(normalize(many).unwrap().len(), MAX_ITEMS);

        let long = format!("/original/{}.txt", "长".repeat(MAX_BYTES));
        assert!(normalize(vec![long]).is_err());
    }

    #[test]
    fn missing_paths_are_reported_only_on_demand() {
        let existing = fixture("missing");
        let absent = format!("{existing}/不存在.txt");
        let reported = missing(&[existing.clone(), absent.clone()]);
        assert_eq!(reported, vec![absent]);
        let _ = std::fs::remove_dir_all(&existing);
    }
}
