use super::*;
use crate::image_transfer::{self, Batch};

fn stage_cache(app: &AppHandle, store: &Store, ids: &[i64], allow_private: bool) -> Result<Batch> {
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|_| "无法定位图片临时目录。")?;
    fs::create_dir_all(&cache).map_err(|_| "无法创建图片临时目录。")?;
    let root = cache.join("image-transfers");
    image_transfer::cache_root(&root)?;
    store.stage_images(&root, ids, allow_private)
}

#[tauri::command]
pub(super) async fn prepare_image_transfer(
    app: AppHandle,
    context: AppContext<'_>,
    ids: Vec<i64>,
    allow_private: bool,
) -> Result<String> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _write = write_guard(&context)?;
        if let Some(previous) = lock(&context.image_transfer)?.take() {
            if !previous.retained {
                if let Some(parent) = previous.paths[0].parent() {
                    let _ = fs::remove_dir_all(parent);
                }
            }
        }
        let batch = stage_cache(&app, &*lock(&context.store)?, &ids, allow_private)?;
        let token = batch.token.clone();
        *lock(&context.image_transfer)? = Some(batch);
        Ok(token)
    })
    .await
    .map_err(|_| "图片准备失败，请重试。".to_string())?
}

#[tauri::command]
pub(super) async fn copy_image_files(
    app: AppHandle,
    context: AppContext<'_>,
    ids: Vec<i64>,
    allow_private: bool,
) -> Result<()> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _write = write_guard(&context)?;
        let store = lock(&context.store)?;
        let batch = stage_cache(&app, &store, &ids, allow_private)?;
        drop(store);
        let paths = batch
            .paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        if let Err(error) = write_clipboard(&context, ClipboardWrite::Files(paths)) {
            if let Some(parent) = batch.paths[0].parent() {
                let _ = fs::remove_dir_all(parent);
            }
            return Err(error);
        }
        let store = lock(&context.store)?;
        for id in ids {
            store.mark_copied(id)?;
        }
        *lock(&context.clipboard_error)? = None;
        changed(&app);
        Ok(())
    })
    .await
    .map_err(|_| "图片文件复制失败，请重试。".to_string())?
}

#[tauri::command]
pub(super) async fn export_image_files(
    app: AppHandle,
    context: AppContext<'_>,
    ids: Vec<i64>,
    allow_private: bool,
) -> Result<Option<String>> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        {
            image_transfer::validate(&*lock(&context.store)?, &ids, allow_private)?;
        }
        let Some(folder) = app.dialog().file().blocking_pick_folder() else {
            return Ok(None);
        };
        let root = folder.into_path().map_err(|_| "请选择本地文件夹。")?;
        let _write = write_guard(&context)?;
        let batch = lock(&context.store)?.stage_images(&root, &ids, allow_private)?;
        Ok(Some(
            batch.paths[0]
                .parent()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        ))
    })
    .await
    .map_err(|_| "图片导出失败，请重试。".to_string())?
}

#[tauri::command]
pub(super) async fn drag_image_files(
    app: AppHandle,
    window: WebviewWindow,
    context: AppContext<'_>,
    token: String,
) -> Result<bool> {
    let context = context.inner().clone();
    let (paths, preview) = {
        let _write = write_guard(&context)?;
        let mut prepared = lock(&context.image_transfer)?;
        let batch = prepared
            .as_mut()
            .filter(|b| b.token == token)
            .ok_or("请先准备所选图片，再按住拖出。")?;
        batch.revalidate(&*lock(&context.store)?)?;
        batch.retained = true;
        (batch.paths.clone(), batch.preview.clone())
    };
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        // An IPC can arrive after a quick mouse release. AppKit would otherwise
        // start a session with no button held and wait for an unrelated click.
        #[cfg(target_os = "macos")]
        if objc2_app_kit::NSEvent::pressedMouseButtons() & 1 == 0 {
            let _ = sender.try_send(Err("请按住「拖出图片」稍候，再移到目标文件夹。".into()));
            return;
        }
        let callback = sender.clone();
        #[cfg(target_os = "linux")]
        let native_window = window.gtk_window();
        #[cfg(target_os = "linux")]
        let native_window = match native_window {
            Ok(ref w) => w,
            Err(_) => {
                let _ = sender.try_send(Err("无法开始拖出，请使用「导出到文件夹」。".into()));
                return;
            }
        };
        #[cfg(not(target_os = "linux"))]
        let native_window = &window;
        if drag::start_drag(
            native_window,
            drag::DragItem::Files(paths),
            drag::Image::Raw(preview),
            move |result, _| {
                let _ = callback.try_send(Ok(matches!(result, drag::DragResult::Dropped)));
            },
            drag::Options {
                mode: drag::DragMode::Copy,
                ..Default::default()
            },
        )
        .is_err()
        {
            let _ = sender.try_send(Err("无法开始拖出，请使用「导出到文件夹」。".into()));
        }
    })
    .map_err(|_| "无法开始拖出，请使用「导出到文件夹」。")?;
    tauri::async_runtime::spawn_blocking(move || {
        receiver
            .recv_timeout(Duration::from_secs(300))
            .map_err(|_| "拖出任务已中断。".to_string())?
    })
    .await
    .map_err(|_| "拖出任务已中断。".to_string())?
}
