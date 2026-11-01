//! 系统剪贴板读写与后台采集循环。
use super::*;

pub(super) enum ClipboardWrite {
    Text(String),
    Html(String, String),
    Image((u32, u32, Vec<u8>)),
    Files(Vec<String>),
}
pub(super) fn write_and_suppress(
    capture: &mut CaptureState,
    key: CaptureKey,
    write: impl FnOnce() -> Result<()>,
) -> Result<()> {
    write()?;
    capture.copied(key);
    Ok(())
}
pub(super) fn write_clipboard(context: &Context, content: ClipboardWrite) -> Result<()> {
    let key = match &content {
        ClipboardWrite::Text(text) | ClipboardWrite::Html(_, text) => {
            CaptureKey::Text(text.clone())
        }
        ClipboardWrite::Files(paths) => CaptureKey::Files(paths.join("\n")),
        ClipboardWrite::Image((width, height, rgba)) => {
            CaptureKey::Image(images::fingerprint_rgba(*width, *height, rgba)?)
        }
    };
    let mut reader = lock(&context.reader)?;
    if reader.clipboard.is_none() {
        reader.clipboard = Some(Clipboard::new().map_err(|e| format!("无法访问剪贴板：{e}"))?);
    }
    let Reader { clipboard, capture } = &mut *reader;
    write_and_suppress(capture, key, || match content {
        ClipboardWrite::Text(text) => clipboard
            .as_mut()
            .unwrap()
            .set_text(text)
            .map_err(|e| format!("复制失败：{e}")),
        ClipboardWrite::Html(html, text) => clipboard
            .as_mut()
            .unwrap()
            .set_html(html, Some(text))
            .map_err(|e| format!("复制失败：{e}")),
        ClipboardWrite::Files(paths) => clipboard
            .as_mut()
            .unwrap()
            .set()
            .file_list(&paths)
            .map_err(|e| format!("文件复制失败：{e}")),
        ClipboardWrite::Image((width, height, rgba)) => clipboard
            .as_mut()
            .unwrap()
            .set_image(ImageData {
                width: width as usize,
                height: height as usize,
                bytes: rgba.into(),
            })
            .map_err(|e| format!("图片复制失败：{e}")),
    })
}

pub(super) fn set_clipboard_error(app: &AppHandle, context: &Context, error: Option<String>) {
    if let Ok(mut current) = context.clipboard_error.lock() {
        if *current != error {
            *current = error;
            changed(app);
        }
    }
}
// Native change tokens avoid repeatedly decoding unchanged images in the background.
pub(super) fn clipboard_sequence() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        return objc2::rc::autoreleasepool(|_| {
            Some(objc2_app_kit::NSPasteboard::generalPasteboard().changeCount() as u64)
        });
    }
    #[cfg(target_os = "windows")]
    {
        let sequence =
            unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() };
        return (sequence != 0).then_some(sequence as u64);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}
pub(super) enum ClipboardContent {
    Empty,
    Text(String, Option<String>),
    Image(ImageData<'static>),
    Files(Vec<String>),
}
pub(super) fn read_clipboard(
    clipboard: &mut Clipboard,
) -> std::result::Result<ClipboardContent, arboard::Error> {
    // Files first: Finder also offers the file icon as an image and names as text.
    if let Ok(files) = clipboard.get().file_list() {
        let paths: Vec<String> = files
            .iter()
            .filter_map(|path| path.to_str().map(str::to_owned))
            .filter(|path| !path.is_empty() && !path.contains('\n'))
            .collect();
        if !paths.is_empty() && paths.len() == files.len() {
            return Ok(ClipboardContent::Files(paths));
        }
    }
    let image_error = match clipboard.get_image() {
        Ok(image) => return Ok(ClipboardContent::Image(image)),
        Err(arboard::Error::ContentNotAvailable) => None,
        Err(arboard::Error::ConversionFailure) => Some(arboard::Error::ConversionFailure),
        Err(error) => return Err(error),
    };
    match clipboard.get_text() {
        Ok(text) => {
            // Formatting is optional: any failure keeps the plain text.
            let html = clipboard
                .get()
                .html()
                .ok()
                .filter(|html| html.len() <= crate::store::MAX_RICH_BYTES);
            Ok(ClipboardContent::Text(text, html))
        }
        Err(arboard::Error::ContentNotAvailable) => match image_error {
            Some(error) => Err(error),
            None => Ok(ClipboardContent::Empty),
        },
        Err(error) => Err(error),
    }
}

pub(super) fn monitor(app: AppHandle, context: Arc<Context>) {
    thread::spawn(move || {
        let mut maintenance_at = Instant::now() + Duration::from_secs(60);
        let mut update_check_at = Instant::now() + Duration::from_secs(20);
        let mut telemetry_at = Instant::now() + Duration::from_secs(60);
        loop {
            thread::sleep(Duration::from_millis(700));
            if Instant::now() >= telemetry_at {
                telemetry_at = Instant::now() + Duration::from_secs(60);
                schedule_telemetry_flush(&app, &context);
            }
            if Instant::now() >= update_check_at {
                update_check_at = Instant::now() + Duration::from_secs(60);
                schedule_auto_update_check(&app, &context);
            }
            // Every mutation takes this gate before reader/store locks. A completed
            // migration leaves it paused until restart, including scheduled cleanup.
            let Ok(storage) = lock(&context.storage) else {
                continue;
            };
            if storage.restart_pending() {
                continue;
            }
            if Instant::now() >= maintenance_at {
                maintenance_at = Instant::now() + Duration::from_secs(60);
                let removed = lock(&context.store).and_then(|store| {
                    if store.settings()?.retention_days == 0 {
                        return Ok(0);
                    }
                    store.prune_history()
                });
                match removed {
                    Ok(count) if count > 0 => changed(&app),
                    Err(error) => set_clipboard_error(&app, &context, Some(error)),
                    _ => {}
                }
            }
            let result: Result<Option<bool>> = (|| {
                // The storage gate serializes mutations. Read-only UI queries remain available
                // while clipboard pixels are decoded and compressed outside the store lock.
                let mut reader = lock(&context.reader)?;
                if lock(&context.store)?.settings()?.paused {
                    return Ok(None);
                }
                let sequence = clipboard_sequence();
                if !reader.capture.should_read(sequence) {
                    return Ok(None);
                }
                if reader.clipboard.is_none() {
                    reader.clipboard =
                        Some(Clipboard::new().map_err(|e| format!("无法访问剪贴板：{e}"))?);
                }
                let content = match read_clipboard(reader.clipboard.as_mut().unwrap()) {
                    Ok(content) => content,
                    Err(error) => {
                        if matches!(error, arboard::Error::ConversionFailure) {
                            reader.capture.skip_invalid(sequence);
                        }
                        return Err(format!("剪贴板读取失败：{error}"));
                    }
                };
                let source = crate::clipboard_source::read();
                // A copy during the read may mix formats from two owners. Retry next poll.
                if sequence.is_some() && sequence != clipboard_sequence() {
                    return Ok(None);
                }
                let key = match &content {
                    ClipboardContent::Image(image) => {
                        let dimensions = u32::try_from(image.width)
                            .ok()
                            .zip(u32::try_from(image.height).ok());
                        let digest = dimensions
                            .ok_or_else(|| "图片尺寸超出支持范围。".to_string())
                            .and_then(|(width, height)| {
                                images::fingerprint_rgba(width, height, &image.bytes)
                            });
                        match digest {
                            Ok(digest) => Some(CaptureKey::Image(digest)),
                            Err(error) => {
                                reader.capture.skip_invalid(sequence);
                                return Err(error);
                            }
                        }
                    }
                    ClipboardContent::Files(paths) => {
                        if paths.len() > crate::store::MAX_FILES {
                            reader.capture.skip_invalid(sequence);
                            return Err(format!(
                                "一次复制超过 {} 个文件，未记录。",
                                crate::store::MAX_FILES
                            ));
                        }
                        Some(CaptureKey::Files(paths.join("\n")))
                    }
                    ClipboardContent::Text(text, _) if !text.trim().is_empty() => {
                        if text.len() > crate::store::MAX_CONTENT_BYTES {
                            reader.capture.skip_invalid(sequence);
                            return Err("单条文字超过 256 KiB，未记录。".into());
                        }
                        Some(CaptureKey::Text(text.clone()))
                    }
                    _ => None,
                };
                let capture = reader.capture.needs_capture(&key);
                if capture {
                    match content {
                        ClipboardContent::Image(image) => {
                            let prepared = match images::prepare_rgba(
                                image.width as u32,
                                image.height as u32,
                                &image.bytes,
                            ) {
                                Ok(image) => image,
                                Err(error) => {
                                    reader.capture.skip_invalid(sequence);
                                    return Err(error);
                                }
                            };
                            lock(&context.store)?.capture_image(&prepared, &source)?;
                        }
                        ClipboardContent::Text(text, html) => {
                            lock(&context.store)?.capture_rich_text(
                                &text,
                                html.as_deref(),
                                &source,
                            )?;
                        }
                        ClipboardContent::Files(paths) => {
                            lock(&context.store)?.capture_files(&paths, &source)?;
                        }
                        ClipboardContent::Empty => {}
                    }
                }
                reader.capture.accept(sequence, key);
                Ok(Some(capture))
            })();
            match result {
                Ok(Some(captured)) => {
                    set_clipboard_error(&app, &context, None);
                    if captured {
                        changed(&app);
                    }
                }
                Ok(None) => {}
                Err(error) => set_clipboard_error(&app, &context, Some(error)),
            }
        }
    });
}
