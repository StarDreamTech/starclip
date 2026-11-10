//! 系统剪贴板读写与后台采集循环。
use super::*;

pub(super) enum ClipboardWrite {
    Text(String),
    Html(String, String),
    Image((u32, u32, Vec<u8>), Vec<u8>),
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
        ClipboardWrite::Image((width, height, rgba), _) => {
            CaptureKey::Image(images::fingerprint_rgba(*width, *height, rgba)?)
        }
    };
    let mut reader = lock(&context.reader)?;
    if reader.clipboard.is_none() {
        reader.clipboard = Some(Clipboard::new().map_err(|e| format!("无法访问剪贴板：{e}"))?);
    }
    let own_png = match &content {
        ClipboardWrite::Image(_, png) => Some(png.clone()),
        _ => None,
    };
    let Reader {
        clipboard, capture, ..
    } = &mut *reader;
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
        ClipboardWrite::Image((width, height, rgba), _) => clipboard
            .as_mut()
            .unwrap()
            .set_image(ImageData {
                width: width as usize,
                height: height as usize,
                bytes: rgba.into(),
            })
            .map_err(|e| format!("图片复制失败：{e}")),
    })?;
    reader.copy_image_png = own_png;
    reader.last_image_id = None;
    reader.last_bundle_id = None;
    reader.own_bundle = None;
    Ok(())
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
    Image(images::PreparedImage),
    Bundle(media::IncomingBundle),
    Files(Vec<String>, String),
}

pub(super) enum ReadFailure {
    Invalid(String),
    Transient(String),
}
fn optional_text(clipboard: &mut Clipboard) -> std::result::Result<String, ReadFailure> {
    match clipboard.get_text() {
        Ok(text) => Ok(text),
        Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
        Err(error) => Err(ReadFailure::Transient(error.to_string())),
    }
}
pub(super) fn read_clipboard(
    clipboard: &mut Clipboard,
) -> std::result::Result<ClipboardContent, ReadFailure> {
    // Files first: Finder also offers the file icon as an image and names as text.
    if let Ok(files) = clipboard.get().file_list() {
        let paths: Vec<String> = files
            .iter()
            .filter_map(|path| path.to_str().map(str::to_owned))
            .filter(|path| !path.is_empty() && !path.contains('\n'))
            .collect();
        if !paths.is_empty() && paths.len() == files.len() {
            let text = optional_text(clipboard)?;
            return Ok(ClipboardContent::Files(paths, text));
        }
    }
    let image_error = match clipboard.get_image() {
        Ok(image) => {
            let width = u32::try_from(image.width)
                .map_err(|_| ReadFailure::Invalid("图片尺寸超出支持范围。".into()))?;
            let height = u32::try_from(image.height)
                .map_err(|_| ReadFailure::Invalid("图片尺寸超出支持范围。".into()))?;
            let prepared =
                images::prepare_rgba(width, height, &image.bytes).map_err(ReadFailure::Invalid)?;
            let text = optional_text(clipboard)?;
            if !text.trim().is_empty() {
                if text.len() > crate::store::MAX_CONTENT_BYTES {
                    return Err(ReadFailure::Invalid(
                        "单条文字超过 256 KiB，未记录。".into(),
                    ));
                }
                return Ok(ClipboardContent::Bundle(media::IncomingBundle {
                    text,
                    images: vec![prepared],
                    own: None,
                }));
            }
            return Ok(ClipboardContent::Image(prepared));
        }
        Err(arboard::Error::ContentNotAvailable) => None,
        Err(arboard::Error::ConversionFailure) => Some(arboard::Error::ConversionFailure),
        Err(error) => return Err(ReadFailure::Transient(error.to_string())),
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
            Some(error) => Err(ReadFailure::Invalid(error.to_string())),
            None => Ok(ClipboardContent::Empty),
        },
        Err(error) => Err(ReadFailure::Transient(error.to_string())),
    }
}

pub(super) fn monitor(app: AppHandle, context: Arc<Context>) {
    thread::spawn(move || {
        let mut maintenance_at = Instant::now() + Duration::from_secs(60);
        let mut recovery_cleanup_at = Instant::now();
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
            let Ok(mut storage) = lock(&context.storage) else {
                continue;
            };
            if storage.restart_pending() {
                continue;
            }
            if Instant::now() >= recovery_cleanup_at {
                recovery_cleanup_at = Instant::now() + Duration::from_secs(24 * 60 * 60);
                // Context exists only after Store opened successfully. Never prune on failed
                // migrations or while an update/directory switch is waiting for restart.
                let _ = storage.cleanup_automatic_recovery_points(unix_millis());
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
                    reader.copy_image_png = None;
                    reader.own_bundle = None;
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
                let mut content = match read_clipboard(reader.clipboard.as_mut().unwrap()) {
                    Ok(content) => content,
                    Err(ReadFailure::Invalid(error)) => {
                        reader.capture.skip_invalid(sequence);
                        return Err(format!("剪贴板读取失败：{error}"));
                    }
                    Err(ReadFailure::Transient(error)) => {
                        return Err(format!("剪贴板读取失败：{error}"));
                    }
                };
                if let ClipboardContent::Files(paths, text) = &content {
                    let key = Some(CaptureKey::Files(paths.join("\n")));
                    match media::read_file_bundle(text.clone(), paths) {
                        Ok(Some(bundle)) => {
                            // A file-only write from this app is suppressed once; a new caption
                            // or a later native change must still be examined as a collection.
                            if bundle.text.is_empty() && !reader.capture.needs_capture(&key) {
                                reader
                                    .capture
                                    .accept(sequence, if sequence.is_some() { None } else { key });
                                return Ok(Some(false));
                            }
                            content = ClipboardContent::Bundle(bundle);
                        }
                        Ok(None) => {}
                        Err(error) => {
                            reader.capture.skip_invalid(sequence);
                            return Err(error);
                        }
                    }
                }

                let source = crate::clipboard_source::read();
                // Retry if ownership changed while multiple representations were read.
                if sequence.is_some() && sequence != clipboard_sequence() {
                    return Ok(None);
                }
                let key = match &content {
                    ClipboardContent::Image(image) => {
                        Some(CaptureKey::Image(image.digest().into()))
                    }
                    ClipboardContent::Files(paths, _) => {
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
                let mut capture = reader.capture.needs_capture(&key);
                match content {
                    ClipboardContent::Image(image) => {
                        let own = reader
                            .copy_image_png
                            .as_deref()
                            .map(|png| {
                                if png.len() < 24 {
                                    return Ok(false);
                                }
                                let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
                                let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
                                image.matches_png(width, height, png)
                            })
                            .transpose()?
                            .unwrap_or(false);
                        let store = lock(&context.store)?;
                        let existing = store.image_record_id(&image)?;
                        // A hash alone never suppresses a different image.
                        capture = !own
                            && (capture || existing.is_none() || existing != reader.last_image_id);
                        if capture {
                            reader.last_image_id = Some(store.capture_image(&image, &source)?.id);
                        } else {
                            reader.last_image_id = existing;
                        }
                        reader.last_bundle_id = None;
                    }
                    ClipboardContent::Bundle(bundle) => {
                        let store = lock(&context.store)?;
                        let own = match &bundle.own {
                            Some((token, id)) if reader.own_bundle.as_ref() == Some(token) => {
                                store.bundle_matches(*id, &bundle.text, &bundle.images)?
                            }
                            _ => false,
                        };
                        let same = match reader.last_bundle_id {
                            Some(id) => store
                                .bundle_matches(id, &bundle.text, &bundle.images)
                                .unwrap_or(false),
                            None => false,
                        };
                        capture = !own && !same;
                        if capture {
                            reader.last_bundle_id = Some(
                                store
                                    .add_bundle(
                                        &bundle.text,
                                        &bundle.images,
                                        "",
                                        false,
                                        false,
                                        Some(&source),
                                    )?
                                    .id,
                            );
                        } else if own {
                            reader.last_bundle_id = bundle.own.as_ref().map(|(_, id)| *id);
                        }
                        reader.last_image_id = None;
                    }
                    ClipboardContent::Text(text, html) => {
                        if capture {
                            lock(&context.store)?.capture_rich_text(
                                &text,
                                html.as_deref(),
                                &source,
                            )?;
                        }
                        reader.last_image_id = None;
                        reader.last_bundle_id = None;
                    }
                    ClipboardContent::Files(paths, _) => {
                        if capture {
                            lock(&context.store)?.capture_files(&paths, &source)?;
                        }
                        reader.last_image_id = None;
                        reader.last_bundle_id = None;
                    }
                    ClipboardContent::Empty => {
                        reader.last_image_id = None;
                        reader.last_bundle_id = None;
                    }
                }
                reader.copy_image_png = None;
                // Keep the own collection token until another representation is observed.
                if key.is_some() {
                    reader.own_bundle = None;
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
