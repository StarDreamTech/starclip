use crate::ai_access::{self, AiAccess, CreateConnection};
use crate::appearance::Appearance;
use crate::capture::{CaptureKey, CaptureState};
use crate::privacy::{PrivacyRescan, PrivacyRules};
use crate::starpet_growth::{self, GrowthChange, GrowthState};
use crate::storage::{DataDirectoryPlan, DataLocation, DirectoryMode, RecoveryPoint, Storage};
use crate::store::{
    starpet_dock_position, starpet_resize_position, CleanupFilter, CleanupPreview, CleanupResult,
    CleanupSnapshot, Clip, Group, HistoryQuery, HistorySummary, Result, Settings,
    StarpetPreferencePatch, StarpetPreferences, Store, UpdatePreferences, STALE_CLEANUP,
};
use crate::telemetry::{self, Telemetry, TelemetryState};
use crate::updater::{self, UpdateStatus, Updates};
use crate::{images, share};
use arboard::{Clipboard, ImageData};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, OnceLock},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_updater::UpdaterExt;

struct Reader {
    clipboard: Option<Clipboard>,
    capture: CaptureState,
}
struct Context {
    storage: Mutex<Storage>,
    ai: Mutex<AiAccess>,
    store: Mutex<Store>,
    telemetry: Mutex<Telemetry>,
    reader: Mutex<Reader>,
    clipboard_error: Mutex<Option<String>>,
    shortcut_error: Mutex<Option<String>>,
    window_error: Mutex<Option<String>>,
    cleanup: Mutex<Option<CleanupSnapshot>>,
    starpet_operation: Mutex<()>,
    startup_operation: Mutex<()>,
    tray_items: OnceLock<TrayItems>,
    native_language: Mutex<String>,
}
struct TrayItems {
    show: MenuItem<tauri::Wry>,
    pause: MenuItem<tauri::Wry>,
    starpet: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}
type AppContext<'a> = State<'a, Arc<Context>>;
fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| "应用内部状态异常，请重新启动星笺。".into())
}
fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
fn write_guard(context: &Context) -> Result<MutexGuard<'_, Storage>> {
    let storage = lock(&context.storage)?;
    storage.writable()?;
    Ok(storage)
}
fn changed(app: &AppHandle) {
    let _ = app.emit("clips-changed", ());
}
fn telemetry_changed(app: &AppHandle, state: &TelemetryState) {
    let _ = app.emit_to("main", "starclip-telemetry-state", state);
}
fn record_telemetry(app: &AppHandle, context: &Context, name: &str) {
    if let Ok(mut telemetry) = lock(&context.telemetry) {
        let before = telemetry.state().revision;
        if telemetry.record(name).is_ok() {
            let state = telemetry.state();
            if state.revision != before {
                telemetry_changed(app, &state);
            }
        }
    }
}
#[tauri::command]
fn get_telemetry_state(context: AppContext<'_>) -> Result<TelemetryState> {
    Ok(lock(&context.telemetry)?.state())
}
#[tauri::command]
fn set_telemetry_enabled(
    app: AppHandle,
    context: AppContext<'_>,
    enabled: bool,
) -> Result<TelemetryState> {
    // Keep the storage → store → telemetry order. No network under these locks.
    let storage = lock(&context.storage)?;
    let store = lock(&context.store)?;
    let mut telemetry = lock(&context.telemetry)?;
    if !enabled {
        telemetry.set_enabled(false);
        telemetry_changed(&app, &telemetry.state());
    } else {
        telemetry.validate_enable()?;
    }
    let saved = storage
        .writable()
        .and_then(|()| store.set_telemetry_enabled(enabled));
    if let Err(error) = saved {
        return Err(if enabled {
            error
        } else {
            "本次运行已停止统计，但关闭偏好未保存；请重试，重启后可能恢复原设置。".into()
        });
    }
    if enabled {
        telemetry.set_enabled(true);
    }
    let state = telemetry.state();
    telemetry_changed(&app, &state);
    Ok(state)
}
#[tauri::command]
fn record_usage_event(
    app: AppHandle,
    context: AppContext<'_>,
    name: String,
) -> Result<TelemetryState> {
    let mut telemetry = lock(&context.telemetry)?;
    telemetry.record_frontend(&name)?;
    let state = telemetry.state();
    telemetry_changed(&app, &state);
    Ok(state)
}
#[tauri::command]
fn reset_telemetry_install(app: AppHandle, context: AppContext<'_>) -> Result<TelemetryState> {
    let mut telemetry = lock(&context.telemetry)?;
    telemetry.reset_install()?;
    let state = telemetry.state();
    telemetry_changed(&app, &state);
    Ok(state)
}
#[tauri::command]
fn clear_telemetry_queue(app: AppHandle, context: AppContext<'_>) -> Result<TelemetryState> {
    let mut telemetry = lock(&context.telemetry)?;
    let cleared = telemetry.clear_queue();
    let state = telemetry.state();
    telemetry_changed(&app, &state);
    cleared?;
    Ok(state)
}
fn flush_telemetry_with_context(app: &AppHandle, context: &Context) -> Result<TelemetryState> {
    let batch = {
        let mut telemetry = lock(&context.telemetry)?;
        let batch = telemetry.take_batch();
        telemetry_changed(app, &telemetry.state());
        batch
    };
    if let Some(batch) = batch {
        let success = telemetry::send(&batch);
        let mut telemetry = lock(&context.telemetry)?;
        telemetry.finish(&batch, success);
        telemetry_changed(app, &telemetry.state());
    }
    Ok(lock(&context.telemetry)?.state())
}
fn schedule_telemetry_flush(app: &AppHandle, context: &Arc<Context>) {
    if lock(&context.telemetry).is_ok_and(|mut telemetry| {
        telemetry.tick();
        telemetry.ready()
    }) {
        let app = app.clone();
        let context = context.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _ = flush_telemetry_with_context(&app, &context);
        });
    }
}
#[tauri::command]
async fn flush_telemetry(app: AppHandle, context: AppContext<'_>) -> Result<TelemetryState> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || flush_telemetry_with_context(&app, &context))
        .await
        .map_err(|_| "统计后台任务暂不可用。".to_string())?
}
fn native_is_english(context: &Context) -> bool {
    lock(&context.native_language)
        .map(|language| language.starts_with("en"))
        .unwrap_or(false)
}
fn native_is_english_app(app: &AppHandle) -> bool {
    app.try_state::<Arc<Context>>()
        .map(|context| native_is_english(context.inner()))
        .unwrap_or(false)
}
fn main_window_title(english: bool) -> &'static str {
    if english {
        "Starclip"
    } else {
        "星笺 Starclip"
    }
}
fn starpet_window_title(english: bool) -> &'static str {
    if english {
        "StarPet · Starclip"
    } else {
        "星伴 · Starclip"
    }
}
fn native_dialog_title(english: bool) -> &'static str {
    if english {
        "Starclip"
    } else {
        "星笺"
    }
}
fn starpet_dialog_title(english: bool) -> &'static str {
    if english {
        "StarPet"
    } else {
        "星伴"
    }
}
fn tray_labels(english: bool, paused: bool) -> (&'static str, &'static str) {
    if english {
        (
            if paused {
                "Resume Recording"
            } else {
                "Pause Recording"
            },
            if paused {
                "Starclip · Paused"
            } else {
                "Starclip · Recording text & images"
            },
        )
    } else {
        (
            if paused {
                "继续记录"
            } else {
                "暂停记录"
            },
            if paused {
                "星笺 · 已暂停"
            } else {
                "星笺 · 正在记录文字与图片"
            },
        )
    }
}
fn apply_window_titles(app: &AppHandle, english: bool) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(main_window_title(english));
    }
    if let Some(window) = app.get_webview_window("starpet") {
        let _ = window.set_title(starpet_window_title(english));
    }
}

#[derive(Default)]
struct MainWindowSession {
    operation: Mutex<()>,
    close_request: Mutex<MainWindowCloseRequest>,
    geometry: Mutex<Option<MainWindowGeometry>>,
}
#[derive(Default)]
struct MainWindowCloseRequest {
    id: u64,
    pending: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MainWindowCloseResult {
    request_id: u64,
    pending: bool,
}
impl MainWindowSession {
    fn next_request(&self, pending: bool) -> Result<u64> {
        let mut request = lock(&self.close_request)?;
        request.id = request.id.wrapping_add(1);
        request.pending = pending;
        Ok(request.id)
    }
    fn pending_request(&self) -> Result<Option<u64>> {
        let request = lock(&self.close_request)?;
        Ok(request.pending.then_some(request.id))
    }
    fn is_current(&self, request: u64) -> Result<bool> {
        Ok(self.pending_request()? == Some(request))
    }
    fn finish_request(&self, id: u64) -> Result<()> {
        let mut request = lock(&self.close_request)?;
        if request.id == id {
            request.pending = false;
        }
        Ok(())
    }
}

struct MainWindowGeometry {
    size: tauri::PhysicalSize<u32>,
    position: tauri::PhysicalPosition<i32>,
    maximized: bool,
}
impl MainWindowGeometry {
    fn capture(window: &WebviewWindow) -> tauri::Result<Self> {
        Ok(Self {
            size: window.inner_size()?,
            position: window.outer_position()?,
            maximized: window.is_maximized()?,
        })
    }
}

// Window creation must run off the event-loop thread, especially on Windows.
fn show_window_inner(app: &AppHandle, reopened: bool) -> Result<()> {
    let session = app.state::<MainWindowSession>();
    let _operation = lock(&session.operation)?;
    let window = if let Some(window) = app.get_webview_window("main") {
        window
    } else {
        let config = app
            .config()
            .app
            .windows
            .iter()
            .find(|window| window.label == "main")
            .ok_or("主窗口配置不存在。")?;
        let mut builder = WebviewWindowBuilder::from_config(app, config)
            .map_err(|error| error.to_string())?
            .visible(false);
        if reopened {
            builder = builder.initialization_script("window.__STARCLIP_REOPENED__ = true;");
        }
        let window = builder
            .build()
            .map_err(|error| format!("无法重新创建主窗口：{error}"))?;
        if let Some(geometry) = lock(&session.geometry)?.as_ref() {
            let _ = window.set_size(geometry.size);
            let _ = window.set_position(geometry.position);
            if geometry.maximized {
                let _ = window.maximize();
            }
        }
        if let Some(context) = app.try_state::<Arc<Context>>() {
            let settings = lock(&context.store)?.settings()?;
            window
                .set_always_on_top(settings.always_on_top)
                .map_err(|error| error.to_string())?;
        }
        window
    };
    let _ = window.set_title(main_window_title(native_is_english_app(app)));
    window
        .show()
        .and_then(|_| window.unminimize())
        .and_then(|_| window.set_focus())
        .map_err(|error| format!("无法打开主窗口：{error}"))
}

fn show_window(app: &AppHandle) {
    // Invalidate a delayed close reply before scheduling the show operation.
    let _ = app.state::<MainWindowSession>().next_request(false);
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = show_window_inner(&app, true) {
            app.dialog()
                .message(error)
                .title(native_dialog_title(native_is_english_app(&app)))
                .show(|_| {});
        }
    });
}

fn request_main_window_close(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        // Conservatively hide first. Only a live frontend that has checked its
        // drafts and pending work may approve releasing this WebView.
        if let Ok(request) = app.state::<MainWindowSession>().next_request(true) {
            let _ = window.hide();
            let _ = app.emit_to(
                "main",
                "main-window-close-requested",
                serde_json::json!({ "requestId": request }),
            );
        }
    }
}
fn toggle_window(app: &AppHandle) {
    if app.get_webview_window("main").is_some_and(|window| {
        window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false)
    }) {
        request_main_window_close(app);
    } else {
        show_window(app);
    }
}

fn destroy_webview_and_wait(app: &AppHandle, window: &WebviewWindow) -> Result<()> {
    let label = window.label();
    window.destroy().map_err(|error| error.to_string())?;
    // destroy() queues a native event. Serialize a rapid reopen until the old
    // label has actually left Tauri's manager; never wait on the UI thread.
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.get_webview_window(label).is_some() {
        if Instant::now() >= deadline {
            return Err("窗口正在释放资源，请稍后再打开。".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn create_starpet(app: &AppHandle, preferences: &StarpetPreferences) -> Result<WebviewWindow> {
    let (width, height) = preferences.window_size();
    let mut builder = WebviewWindowBuilder::new(
        app,
        "starpet",
        WebviewUrl::App("index.html?view=starpet".into()),
    )
    .title(starpet_window_title(native_is_english_app(app)))
    .inner_size(width, height)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    // Initial visibility with focused(false) does not activate the application.
    // Calling show() after a hidden build would steal focus on macOS.
    .visible(true)
    .focused(false);
    if let Ok(Some(monitor)) = app.primary_monitor() {
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let left = f64::from(area.position.x) / scale;
        let top = f64::from(area.position.y) / scale;
        let x = left + (f64::from(area.size.width) / scale - width - 20.0).max(0.0);
        let y = top + (f64::from(area.size.height) / scale - height - 20.0).max(0.0);
        builder = builder.position(x, y);
    }
    builder
        .build()
        .map_err(|e| format!("无法创建星伴窗口，偏好未保存：{e}"))
}

struct StarpetGeometry {
    inner: tauri::PhysicalSize<u32>,
    outer: tauri::PhysicalSize<u32>,
    position: tauri::PhysicalPosition<i32>,
    scale: f64,
}
impl StarpetGeometry {
    fn capture(window: &WebviewWindow) -> Result<Self> {
        (|| -> tauri::Result<Self> {
            Ok(Self {
                inner: window.inner_size()?,
                outer: window.outer_size()?,
                position: window.outer_position()?,
                scale: window.scale_factor()?,
            })
        })()
        .map_err(|e| format!("无法读取星伴窗口大小，偏好未保存：{e}"))
    }
}

fn resize_starpet(
    window: &WebviewWindow,
    previous: &StarpetGeometry,
    preferences: &StarpetPreferences,
) -> Result<()> {
    let (width, height) = preferences.window_size();
    let inner = tauri::LogicalSize::new(width, height).to_physical::<u32>(previous.scale);
    let outer = (
        f64::from(inner.width + previous.outer.width.saturating_sub(previous.inner.width)),
        f64::from(inner.height + previous.outer.height.saturating_sub(previous.inner.height)),
    );
    let monitor = match window.current_monitor().map_err(|e| e.to_string())? {
        Some(monitor) => monitor,
        None => window
            .primary_monitor()
            .map_err(|e| e.to_string())?
            .ok_or("暂时无法确定星伴所在屏幕，大小未更改。")?,
    };
    let area = monitor.work_area();
    if outer.0 > f64::from(area.size.width) || outer.1 > f64::from(area.size.height) {
        return Err("当前屏幕可用空间不足，请选择更小的星伴大小。".into());
    }
    let (x, y) = starpet_resize_position(
        (
            f64::from(previous.position.x),
            f64::from(previous.position.y),
        ),
        (
            f64::from(previous.outer.width),
            f64::from(previous.outer.height),
        ),
        outer,
        (
            f64::from(area.position.x),
            f64::from(area.position.y),
            f64::from(area.size.width),
            f64::from(area.size.height),
        ),
    );
    window
        .set_size(inner)
        .map_err(|e| format!("无法调整星伴大小：{e}"))?;
    window
        .set_position(tauri::PhysicalPosition::new(
            x.round() as i32,
            y.round() as i32,
        ))
        .map_err(|e| format!("无法调整星伴位置：{e}"))
}

fn restore_starpet(
    window: &WebviewWindow,
    visible: bool,
    geometry: Option<&StarpetGeometry>,
) -> Result<()> {
    let mut errors = Vec::new();
    if let Some(geometry) = geometry {
        if let Err(error) = window.set_size(geometry.inner) {
            errors.push(error.to_string());
        }
        if let Err(error) = window.set_position(geometry.position) {
            errors.push(error.to_string());
        }
    }
    if window.is_visible().ok() != Some(visible) {
        let restore = if visible {
            window.show()
        } else {
            window.hide()
        };
        if let Err(error) = restore {
            errors.push(error.to_string());
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

// Every caller runs off the UI thread: window creation may wait for the event loop.
fn update_starpet(
    app: &AppHandle,
    context: &Context,
    update: impl FnOnce(&mut StarpetPreferences),
) -> Result<StarpetPreferences> {
    let _operation = lock(&context.starpet_operation)?;
    let old = lock(&context.store)?.starpet_preferences()?;
    let mut preferences = old.clone();
    update(&mut preferences);
    preferences.validate()?;
    let previous_window = app.get_webview_window("starpet");
    let previously_visible = previous_window
        .as_ref()
        .map(|window| window.is_visible().map_err(|e| e.to_string()))
        .transpose()?
        .unwrap_or(false);
    let geometry = if preferences.size != old.size {
        previous_window
            .as_ref()
            .map(StarpetGeometry::capture)
            .transpose()?
    } else {
        None
    };
    let mut window = previous_window.clone();
    // Do not hold the data write gate while dispatching window operations: a
    // synchronous main-window command may need that gate on the UI thread.
    let save_result = (|| -> Result<()> {
        if let Some(window) = window.as_ref() {
            if let Some(geometry) = geometry.as_ref() {
                resize_starpet(window, geometry, &preferences)?;
            }
            if preferences.enabled != previously_visible {
                if preferences.enabled {
                    window.show()
                } else {
                    window.hide()
                }
                .map_err(|e| format!("无法更改星伴窗口可见性：{e}"))?;
            }
        } else if preferences.enabled {
            window = Some(create_starpet(app, &preferences)?);
        }
        if preferences != old {
            let _write = write_guard(context)?;
            lock(&context.store)?.save_starpet_preferences(&preferences)?;
        }
        Ok(())
    })();
    if let Err(error) = save_result {
        let rollback = if let Some(window) = window {
            if previous_window.is_none() {
                window.destroy().map_err(|e| e.to_string())
            } else {
                restore_starpet(&window, previously_visible, geometry.as_ref())
            }
        } else {
            Ok(())
        };
        return Err(match rollback {
            Ok(()) => format!("星伴偏好未保存，已恢复原窗口状态：{error}"),
            Err(rollback) => {
                format!("星伴偏好未保存：{error}；恢复窗口状态失败：{rollback}。请重启星笺。")
            }
        });
    }
    for label in ["main", "starpet"] {
        let _ = app.emit_to(label, "starpet-changed", &preferences);
    }
    if !preferences.enabled {
        // Do not replay an old Agent message when the user enables the pet again.
        lock(&context.ai)?.clear_pet();
        if let Some(window) = window {
            if let Err(error) = destroy_webview_and_wait(app, &window) {
                starpet_error(app, format!("星伴已停用，窗口资源释放未完成：{error}"));
            }
        }
    }
    Ok(preferences)
}

fn starpet_error(app: &AppHandle, error: String) {
    app.dialog()
        .message(error)
        .title(starpet_dialog_title(native_is_english_app(app)))
        .show(|_| {});
}

#[tauri::command]
async fn get_starpet_preferences(context: AppContext<'_>) -> Result<StarpetPreferences> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // A newly created webview must not read the old value before a save/rollback finishes.
        let _operation = lock(&context.starpet_operation)?;
        lock(&context.store)?.starpet_preferences()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn set_starpet_preferences(
    app: AppHandle,
    context: AppContext<'_>,
    preferences: StarpetPreferencePatch,
) -> Result<StarpetPreferences> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        update_starpet(&app, &context, |value| preferences.apply_to(value))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn show_main_window(app: AppHandle) -> Result<()> {
    app.state::<MainWindowSession>().next_request(false)?;
    tauri::async_runtime::spawn_blocking(move || show_window_inner(&app, true))
        .await
        .map_err(|_| "主窗口创建任务失败，请重试。".to_string())?
}

#[tauri::command]
fn starpet_start_dragging(window: WebviewWindow) -> Result<()> {
    if window.label() != "starpet" {
        return Err("此操作仅用于星伴窗口。".into());
    }
    window
        .start_dragging()
        .map_err(|e| format!("无法拖动星伴：{e}"))
}

#[tauri::command]
fn starpet_dock(window: WebviewWindow) -> Result<Option<&'static str>> {
    if window.label() != "starpet" {
        return Err("此操作仅用于星伴窗口。".into());
    }
    let monitor = match window.current_monitor().map_err(|e| e.to_string())? {
        Some(monitor) => monitor,
        None => return Ok(None),
    };
    let position = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let area = monitor.work_area();
    let docked = starpet_dock_position(
        (f64::from(position.x), f64::from(position.y)),
        (f64::from(size.width), f64::from(size.height)),
        (
            f64::from(area.position.x),
            f64::from(area.position.y),
            f64::from(area.size.width),
            f64::from(area.size.height),
        ),
        // About 28 logical pixels: close enough to feel magnetic, far enough to drop freely.
        28.0 * monitor.scale_factor(),
    );
    let Some((edge, x, y)) = docked else {
        return Ok(None);
    };
    window
        .set_position(tauri::PhysicalPosition::new(
            x.round() as i32,
            y.round() as i32,
        ))
        .map_err(|e| format!("无法让星伴贴边：{e}"))?;
    Ok(Some(edge))
}

fn publish_growth(app: &AppHandle, growth: &GrowthState) {
    for label in ["main", "starpet"] {
        let _ = app.emit_to(label, "starpet-growth-changed", growth);
    }
}

fn change_growth(
    app: &AppHandle,
    context: &Context,
    change: GrowthChange<'_>,
) -> Result<GrowthState> {
    let _write = write_guard(context)?;
    let growth = lock(&context.store)?.change_starpet_growth(change)?;
    publish_growth(app, &growth);
    Ok(growth)
}

// Call only after the primary operation succeeds, while the caller owns its write guard.
// A pet-storage failure must never turn an already-saved code/card into an apparent failure.
fn reward_creation(app: &AppHandle, context: &Context) {
    let result =
        lock(&context.store).and_then(|store| store.change_starpet_growth(GrowthChange::Creation));
    match result {
        Ok(growth) => publish_growth(app, &growth),
        Err(_) => {
            let _ = app.emit_to(
                "main",
                "starpet-growth-error",
                "操作已完成，但星伴成长未能保存。请稍后在星伴页重试互动。",
            );
        }
    }
}

#[tauri::command]
fn get_starpet_growth(context: AppContext<'_>) -> Result<GrowthState> {
    lock(&context.store)?.starpet_growth()
}

#[tauri::command]
fn set_starpet_growth_enabled(
    app: AppHandle,
    context: AppContext<'_>,
    enabled: bool,
) -> Result<GrowthState> {
    change_growth(&app, &context, GrowthChange::Enabled(enabled))
}

#[tauri::command]
fn interact_starpet(
    app: AppHandle,
    context: AppContext<'_>,
    action: String,
    animate: Option<bool>,
) -> Result<GrowthState> {
    let _write = write_guard(&context)?;
    let store = lock(&context.store)?;
    let mut ai = lock(&context.ai)?;
    let (growth, interaction) =
        apply_starpet_interaction(&store, &mut ai, &action, animate.unwrap_or(true))?;
    publish_growth(&app, &growth);
    if let Some(interaction) = interaction {
        // Delivery only: never create/show a disabled or absent companion.
        // An event failure must not report the already-saved interaction as failed.
        if app.get_webview_window("starpet").is_some() {
            if app
                .emit_to("starpet", "starpet-interaction", interaction)
                .is_err()
            {
                let _ = app.emit_to(
                    "main",
                    "starpet-growth-error",
                    "互动已保存，但桌面动作未能播放。请收起并重新开启星伴后重试。",
                );
            }
        }
    }
    Ok(growth)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StarpetInteraction<'a> {
    action: &'a str,
    animate: bool,
    signal_revision: u64,
}

fn apply_starpet_interaction<'a>(
    store: &Store,
    ai: &mut AiAccess,
    action: &'a str,
    animate: bool,
) -> Result<(GrowthState, Option<StarpetInteraction<'a>>)> {
    let enabled = store.starpet_preferences()?.enabled;
    // Validate unlocks and commit exactly once before notifying any animation.
    let growth = store.change_starpet_growth(GrowthChange::Interaction(action))?;
    let interaction = enabled.then(|| {
        // A successful local interaction takes over the current Agent signal.
        // Its revision also prevents a late initial snapshot from restoring it.
        ai.clear_pet();
        StarpetInteraction {
            action,
            animate,
            signal_revision: ai.pet_snapshot().revision,
        }
    });
    Ok((growth, interaction))
}

#[tauri::command]
fn equip_starpet_item(
    app: AppHandle,
    context: AppContext<'_>,
    slot: String,
    item_id: Option<String>,
) -> Result<GrowthState> {
    change_growth(
        &app,
        &context,
        GrowthChange::Equip(&slot, item_id.as_deref()),
    )
}

#[tauri::command]
fn import_starpet_growth(
    app: AppHandle,
    context: AppContext<'_>,
    data: String,
    expected_revision: u64,
) -> Result<GrowthState> {
    change_growth(
        &app,
        &context,
        GrowthChange::Import {
            data: &data,
            expected_revision,
        },
    )
}

#[tauri::command]
fn reset_starpet_growth(
    app: AppHandle,
    context: AppContext<'_>,
    expected_revision: u64,
) -> Result<GrowthState> {
    change_growth(&app, &context, GrowthChange::Reset { expected_revision })
}

#[tauri::command]
async fn export_starpet_growth(app: AppHandle, context: AppContext<'_>) -> Result<Option<String>> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let data = starpet_growth::encode_save(lock(&context.store)?.starpet_growth()?)?;
        let parent = app.get_webview_window("main").ok_or("主窗口不存在。")?;
        let Some(file) = app
            .dialog()
            .file()
            .set_parent(&parent)
            .add_filter("星伴成长存档", &["json"])
            .set_file_name("starclip-starpet.json")
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = file.into_path().map_err(|_| "请选择本地文件路径。")?;
        write_private(&path, data.as_bytes())?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|_| "导出成长存档失败。".to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppState {
    clips: Vec<Clip>,
    history: HistorySummary,
    groups: Vec<Group>,
    settings: Settings,
    onboarding_completed: bool,
    clipboard_error: Option<String>,
    shortcut_error: Option<String>,
    window_error: Option<String>,
}
#[tauri::command]
fn get_state(query: Option<HistoryQuery>, context: AppContext<'_>) -> Result<AppState> {
    let store = lock(&context.store)?;
    let page = store.history_page(&query.unwrap_or_default())?;
    Ok(AppState {
        clips: page.clips,
        history: page.history,
        groups: store.groups()?,
        settings: store.settings()?,
        onboarding_completed: store.onboarding_completed()?,
        clipboard_error: lock(&context.clipboard_error)?.clone(),
        shortcut_error: lock(&context.shortcut_error)?.clone(),
        window_error: lock(&context.window_error)?.clone(),
    })
}
#[tauri::command]
fn get_clips(ids: Vec<i64>, context: AppContext<'_>) -> Result<Vec<Clip>> {
    lock(&context.store)?.clip_details(&ids)
}
#[tauri::command]
fn complete_onboarding(app: AppHandle, context: AppContext<'_>) -> Result<()> {
    let _write = write_guard(&context)?;
    lock(&context.store)?.complete_onboarding()?;
    changed(&app);
    Ok(())
}
#[tauri::command]
fn add_clip(
    app: AppHandle,
    context: AppContext<'_>,
    content: String,
    title: String,
    group_id: Option<i64>,
    keep_forever: Option<bool>,
) -> Result<Clip> {
    let _write = write_guard(&context)?;
    let clip = lock(&context.store)?.add_protected(
        &content,
        &title,
        group_id,
        keep_forever.unwrap_or(false),
    )?;
    changed(&app);
    Ok(clip)
}
#[tauri::command]
fn update_clip(
    app: AppHandle,
    context: AppContext<'_>,
    id: i64,
    title: String,
    group_id: Option<i64>,
    pinned: bool,
) -> Result<()> {
    let _write = write_guard(&context)?;
    lock(&context.store)?.update(id, &title, group_id, pinned)?;
    changed(&app);
    Ok(())
}
#[tauri::command]
fn save_code(
    app: AppHandle,
    context: AppContext<'_>,
    id: Option<i64>,
    title: String,
    content: String,
    language: String,
    group_id: Option<i64>,
) -> Result<Clip> {
    let _write = write_guard(&context)?;
    let clip = lock(&context.store)?.save_code(id, &title, &content, &language, group_id)?;
    reward_creation(&app, &context);
    changed(&app);
    Ok(clip)
}
#[tauri::command]
fn delete_clip(app: AppHandle, context: AppContext<'_>, id: i64) -> Result<()> {
    let _write = write_guard(&context)?;
    lock(&context.store)?.delete(id)?;
    *lock(&context.cleanup)? = None;
    changed(&app);
    Ok(())
}
#[tauri::command]
async fn get_clip_image(context: AppContext<'_>, id: i64, thumbnail: bool) -> Result<String> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let png = lock(&context.store)?.image_png(id, thumbnail)?;
        Ok(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        ))
    })
    .await
    .map_err(|_| "图片读取任务失败。".to_string())?
}
fn product_share_png(app: &AppHandle, image: &share::ShareImage) -> Result<Vec<u8>> {
    let asset = app
        .asset_resolver()
        .get(image.path.clone())
        .ok_or("产品介绍图缺失，请重新安装星笺；开发模式请先构建前端。")?;
    Ok(asset.bytes)
}
#[tauri::command]
async fn copy_product_share_text(
    app: AppHandle,
    context: AppContext<'_>,
    id: String,
) -> Result<()> {
    let text = share::text(&id)?;
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _write = write_guard(&context)?;
        write_clipboard(&context, ClipboardWrite::Text(text))?;
        set_clipboard_error(&app, &context, None);
        Ok(())
    })
    .await
    .map_err(|_| "分享文案复制失败，请重试。".to_string())?
}
#[tauri::command]
async fn copy_product_share_image(
    app: AppHandle,
    context: AppContext<'_>,
    id: String,
) -> Result<()> {
    let image = share::image(&id)?;
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _write = write_guard(&context)?;
        let png = product_share_png(&app, &image)?;
        let pixels = share::decode_image(&image, &png)?;
        write_clipboard(&context, ClipboardWrite::Image(pixels))?;
        set_clipboard_error(&app, &context, None);
        Ok(())
    })
    .await
    .map_err(|_| "产品介绍图复制失败，请重试。".to_string())?
}
#[tauri::command]
async fn save_product_share_image(
    app: AppHandle,
    context: AppContext<'_>,
    id: String,
) -> Result<Option<String>> {
    let image = share::image(&id)?;
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        {
            let _write = write_guard(&context)?;
        }
        let png = product_share_png(&app, &image)?;
        share::decode_image(&image, &png)?;
        let Some(file) = app
            .dialog()
            .file()
            .add_filter("PNG 图片", &["png"])
            .set_file_name(&image.filename)
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = file.into_path().map_err(|_| "请选择本地文件路径。")?;
        let _write = write_guard(&context)?;
        write_private(&path, &png)?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|_| "产品介绍图保存失败，请重试。".to_string())?
}
#[tauri::command]
async fn copy_clip(
    app: AppHandle,
    context: AppContext<'_>,
    id: i64,
    plain: Option<bool>,
) -> Result<()> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _write = write_guard(&context)?;
        let plain = plain.unwrap_or(false);
        let (clip, png, html) = {
            let store = lock(&context.store)?;
            let clip = store.clip(id)?;
            let png = if clip.kind == "image" {
                Some(store.image_png(id, false)?)
            } else {
                None
            };
            let html = if clip.rich && !plain {
                store.clip_html(id)?
            } else {
                None
            };
            (clip, png, html)
        };
        let content = match (png, html) {
            (Some(png), _) => ClipboardWrite::Image(images::decode_png(&png)?),
            (None, Some(html)) => ClipboardWrite::Html(html, clip.content),
            (None, None) if clip.kind == "file" && !plain => {
                let paths: Vec<String> = clip.content.lines().map(str::to_owned).collect();
                if !paths.iter().all(|path| std::path::Path::new(path).exists()) {
                    return Err("部分文件已移动或删除，可选择「复制为纯文本」只复制路径。".into());
                }
                ClipboardWrite::Files(paths)
            }
            (None, None) => ClipboardWrite::Text(clip.content),
        };
        write_clipboard(&context, content)?;
        lock(&context.store)?.mark_copied(id)?;
        *lock(&context.clipboard_error)? = None;
        changed(&app);
        Ok(())
    })
    .await
    .map_err(|_| "复制任务失败，请重试。".to_string())?
}
#[tauri::command]
fn create_group(
    app: AppHandle,
    context: AppContext<'_>,
    name: String,
    color: String,
) -> Result<Group> {
    let _write = write_guard(&context)?;
    let group = lock(&context.store)?.create_group(&name, &color)?;
    changed(&app);
    Ok(group)
}
#[tauri::command]
fn delete_group(app: AppHandle, context: AppContext<'_>, id: i64) -> Result<()> {
    let _write = write_guard(&context)?;
    lock(&context.store)?.delete_group(id)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
fn get_privacy_rules(context: AppContext<'_>) -> Result<PrivacyRules> {
    lock(&context.store)?.privacy_rules()
}
#[tauri::command]
fn save_privacy_rules(
    app: AppHandle,
    context: AppContext<'_>,
    rules: PrivacyRules,
) -> Result<PrivacyRules> {
    let _write = write_guard(&context)?;
    let result = lock(&context.store)?.save_privacy_rules(&rules)?;
    changed(&app);
    Ok(result)
}
#[tauri::command]
fn set_clip_privacy(
    app: AppHandle,
    context: AppContext<'_>,
    id: i64,
    is_private: bool,
    reviewed: bool,
) -> Result<Clip> {
    let _write = write_guard(&context)?;
    let result = lock(&context.store)?.set_clip_privacy(id, is_private, reviewed)?;
    changed(&app);
    Ok(result)
}
#[tauri::command]
fn set_clip_tags(
    app: AppHandle,
    context: AppContext<'_>,
    id: i64,
    tags: Vec<String>,
) -> Result<Clip> {
    let _write = write_guard(&context)?;
    let result = lock(&context.store)?.set_clip_tags(id, &tags)?;
    changed(&app);
    Ok(result)
}
#[tauri::command]
fn preview_privacy_rescan(context: AppContext<'_>) -> Result<PrivacyRescan> {
    lock(&context.store)?.privacy_rescan_preview()
}
#[tauri::command]
fn apply_privacy_rescan(
    app: AppHandle,
    context: AppContext<'_>,
    token: String,
) -> Result<PrivacyRescan> {
    let _write = write_guard(&context)?;
    let result = lock(&context.store)?.privacy_rescan_apply(&token)?;
    changed(&app);
    Ok(result)
}
#[tauri::command]
fn get_ai_state(context: AppContext<'_>) -> Result<serde_json::Value> {
    Ok(lock(&context.ai)?.state())
}
#[tauri::command]
fn get_starpet_signal(context: AppContext<'_>) -> Result<ai_access::PetSnapshot> {
    Ok(lock(&context.ai)?.pet_snapshot())
}
#[tauri::command]
fn set_ai_enabled(
    app: AppHandle,
    context: AppContext<'_>,
    enabled: bool,
) -> Result<serde_json::Value> {
    if enabled {
        let _write = write_guard(&context)?;
        return lock(&context.ai)?.enable(true);
    }
    let mut ai = lock(&context.ai)?;
    let state = ai.enable(false)?;
    let _ = app.emit_to("starpet", "starpet-signal", ai.pet_snapshot());
    Ok(state)
}
#[tauri::command]
fn create_ai_connection(
    context: AppContext<'_>,
    request: CreateConnection,
) -> Result<serde_json::Value> {
    let _write = write_guard(&context)?;
    let store = lock(&context.store)?;
    let executable = std::env::current_exe().map_err(|_| "无法定位星笺程序。")?;
    lock(&context.ai)?.create(&store, request, &executable)
}
#[tauri::command]
fn revoke_ai_connection(
    app: AppHandle,
    context: AppContext<'_>,
    id: String,
) -> Result<serde_json::Value> {
    let mut ai = lock(&context.ai)?;
    let state = ai.revoke(&id);
    let _ = app.emit_to("starpet", "starpet-signal", ai.pet_snapshot());
    Ok(state)
}
#[tauri::command]
fn get_ai_connection_config(context: AppContext<'_>, id: String) -> Result<serde_json::Value> {
    let executable = std::env::current_exe().map_err(|_| "无法定位星笺程序。")?;
    lock(&context.ai)?.config(&id, &executable)
}
#[tauri::command]
fn clear_ai_audit(context: AppContext<'_>) -> Result<serde_json::Value> {
    Ok(lock(&context.ai)?.clear_audit())
}

#[tauri::command]
fn get_appearance(context: AppContext<'_>) -> Result<Appearance> {
    lock(&context.store)?.appearance()
}
#[tauri::command]
fn save_appearance(context: AppContext<'_>, appearance: serde_json::Value) -> Result<Appearance> {
    let _write = write_guard(&context)?;
    lock(&context.store)?.save_appearance(&appearance)
}
#[tauri::command]
async fn import_appearance_sticker(
    context: AppContext<'_>,
    png_base64: String,
    name: String,
) -> Result<Appearance> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _write = write_guard(&context)?;
        lock(&context.store)?.import_appearance_sticker(&png_base64, &name)
    })
    .await
    .map_err(|_| "贴纸导入任务失败，请重试。".to_string())?
}
#[tauri::command]
fn remove_appearance_sticker(context: AppContext<'_>) -> Result<Appearance> {
    let _write = write_guard(&context)?;
    lock(&context.store)?.remove_appearance_sticker()
}
#[tauri::command]
fn set_theme(app: AppHandle, context: AppContext<'_>, theme: String) -> Result<Settings> {
    let _write = write_guard(&context)?;
    let settings = lock(&context.store)?.set_theme(&theme)?;
    changed(&app);
    Ok(settings)
}
#[tauri::command]
fn save_settings(
    app: AppHandle,
    context: AppContext<'_>,
    mut settings: Settings,
    expected_pause_revision: Option<u64>,
) -> Result<()> {
    let _write = write_guard(&context)?;
    settings.validate()?;
    let old = lock(&context.store)?.settings()?;
    let shortcut: Shortcut = settings
        .shortcut
        .parse()
        .map_err(|_| "快捷键格式无效，例如 CommandOrControl+Shift+V。")?;
    let old_shortcut = old.shortcut.parse::<Shortcut>().ok();
    let shortcut_changed = old_shortcut != Some(shortcut);
    // Tray pause remains available while the main WebView has been released.
    let window = app.get_webview_window("main");
    let previous_top = window
        .as_ref()
        .and_then(|window| window.is_always_on_top().ok())
        .unwrap_or(old.always_on_top);
    if shortcut_changed {
        app.global_shortcut()
            .register(shortcut)
            .map_err(|_| "该快捷键无法注册，可能被其他应用占用。")?;
    }
    if let Some(error) = window
        .as_ref()
        .and_then(|window| window.set_always_on_top(settings.always_on_top).err())
    {
        if shortcut_changed {
            let _ = app.global_shortcut().unregister(shortcut);
        }
        let message = format!("无法应用窗口置顶，偏好设置未保存：{error}");
        *lock(&context.window_error)? = Some(message.clone());
        changed(&app);
        return Err(message);
    }
    let save_result = (|| -> Result<()> {
        // Lock order is reader -> store, also used by the monitor. Pause cannot race with capture.
        let mut reader = lock(&context.reader)?;
        settings = lock(&context.store)?
            .save_settings_with_pause_guard(&settings, expected_pause_revision)?;
        if old.paused != settings.paused {
            reader.capture = CaptureState::default();
        }
        Ok(())
    })();
    if let Err(error) = save_result {
        if shortcut_changed {
            let _ = app.global_shortcut().unregister(shortcut);
        }
        let rollback_error = window
            .as_ref()
            .and_then(|window| window.set_always_on_top(previous_top).err())
            .map(|error| format!("恢复原窗口置顶状态失败，请重试保存偏好：{error}"));
        *lock(&context.window_error)? = rollback_error.clone();
        changed(&app);
        return Err(match rollback_error {
            Some(rollback) => format!("{error}；{rollback}"),
            None => error,
        });
    }
    if shortcut_changed {
        if let Some(old_shortcut) = old_shortcut {
            let _ = app.global_shortcut().unregister(old_shortcut);
        }
        *lock(&context.shortcut_error)? = None;
    }
    *lock(&context.window_error)? = None;
    drop(_write);
    refresh_tray(&app, &context, &settings);
    apply_window_titles(&app, native_is_english(&context));
    changed(&app);
    Ok(())
}
#[tauri::command]
fn set_native_language(app: AppHandle, context: AppContext<'_>, language: String) -> Result<()> {
    if !["zh-CN", "en-US"].contains(&language.as_str()) {
        return Err("不支持的界面语言。".into());
    }
    *lock(&context.native_language)? = language;
    let settings = lock(&context.store)?.settings()?;
    refresh_tray(&app, &context, &settings);
    apply_window_titles(&app, native_is_english(&context));
    Ok(())
}
#[tauri::command]
fn get_clip_stats(
    context: AppContext<'_>,
    days: u32,
    utc_offset_minutes: i32,
) -> Result<crate::clip_stats::ClipStats> {
    lock(&context.store)?.clip_stats(days, utc_offset_minutes)
}
#[tauri::command]
fn clear_clip_stats(context: AppContext<'_>) -> Result<()> {
    let _write = write_guard(&context)?;
    lock(&context.store)?.clear_clip_stats()
}
#[tauri::command]
fn clear_history(app: AppHandle, context: AppContext<'_>) -> Result<()> {
    let _write = write_guard(&context)?;
    lock(&context.store)?.clear_history()?;
    *lock(&context.cleanup)? = None;
    changed(&app);
    Ok(())
}
#[tauri::command]
fn set_keep_forever(
    app: AppHandle,
    context: AppContext<'_>,
    id: i64,
    keep_forever: bool,
) -> Result<()> {
    let _write = write_guard(&context)?;
    lock(&context.store)?.set_keep_forever(id, keep_forever)?;
    changed(&app);
    Ok(())
}
#[tauri::command]
async fn preview_cleanup(context: AppContext<'_>, filter: CleanupFilter) -> Result<CleanupPreview> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _write = write_guard(&context)?;
        let mut pending = lock(&context.cleanup)?;
        // A failed replacement preview also invalidates the old confirmation.
        *pending = None;
        let snapshot = lock(&context.store)?.preview_cleanup(&filter)?;
        let preview = snapshot.preview.clone();
        *pending = Some(snapshot);
        Ok(preview)
    })
    .await
    .map_err(|_| "清理预览任务失败。".to_string())?
}
#[tauri::command]
async fn cancel_cleanup(context: AppContext<'_>, token: String) -> Result<()> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut pending = lock(&context.cleanup)?;
        if pending
            .as_ref()
            .is_some_and(|snapshot| snapshot.preview.token == token)
        {
            *pending = None;
        }
        Ok(())
    })
    .await
    .map_err(|_| "无法释放清理预览。".to_string())?
}
#[tauri::command]
async fn execute_cleanup(
    app: AppHandle,
    context: AppContext<'_>,
    token: String,
    compact: bool,
) -> Result<CleanupResult> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _write = write_guard(&context)?;
        let mut pending = lock(&context.cleanup)?;
        if pending
            .as_ref()
            .is_none_or(|snapshot| snapshot.preview.token != token)
        {
            return Err(STALE_CLEANUP.into());
        }
        let snapshot = pending.take().ok_or(STALE_CLEANUP)?;
        let result = lock(&context.store)?.execute_cleanup(snapshot, compact)?;
        changed(&app);
        Ok(result)
    })
    .await
    .map_err(|_| "清理任务失败，请刷新预览确认当前状态。".to_string())?
}
#[tauri::command]
async fn export_backup(app: AppHandle, context: AppContext<'_>) -> Result<String> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let data = lock(&context.store)?.export()?;
        record_telemetry(&app, &context, "backup_exported");
        Ok(data)
    })
    .await
    .map_err(|_| "导出任务失败。".to_string())?
}
#[tauri::command]
async fn import_backup(app: AppHandle, context: AppContext<'_>, data: String) -> Result<usize> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _write = write_guard(&context)?;
        let count = lock(&context.store)?.import(&data)?;
        *lock(&context.cleanup)? = None;
        changed(&app);
        record_telemetry(&app, &context, "backup_imported");
        Ok(count)
    })
    .await
    .map_err(|_| "导入任务失败。".to_string())?
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or("无效的文件路径。")?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(
        ".starclip-export-{}-{nonce}.tmp",
        std::process::id()
    ));
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|_| "无法写入所选目录，请检查权限。")?;
        file.write_all(data)
            .and_then(|_| file.sync_all())
            .map_err(|_| "文件写入失败，请检查磁盘空间。")?;
        drop(file);
        fs::rename(&temporary, path).map_err(|_| "无法保存文件，请检查文件是否被占用。")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
#[tauri::command]
async fn save_backup(app: AppHandle, context: AppContext<'_>) -> Result<Option<String>> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let data = lock(&context.store)?.export()?;
        let window = app
            .get_webview_window("main")
            .ok_or("主窗口不存在，无法选择保存位置。")?;
        let Some(file) = app
            .dialog()
            .file()
            .set_parent(&window)
            .add_filter("Starclip JSON", &["json"])
            .set_file_name("starclip-backup.json")
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = file.into_path().map_err(|_| "请选择本地文件路径。")?;
        let _ = app.emit_to("main", "backup-writing", ());
        write_private(&path, data.as_bytes())?;
        record_telemetry(&app, &context, "backup_exported");
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|_| "导出任务失败。".to_string())?
}
#[tauri::command]
async fn save_export(
    app: AppHandle,
    context: AppContext<'_>,
    data: String,
    format: String,
    filename: Option<String>,
) -> Result<bool> {
    use base64::Engine;
    let context = context.inner().clone();
    let bytes = match format.as_str() {
        "md" => {
            if data.len() > 1024 * 1024 {
                return Err("导出内容过大。".into());
            }
            data.into_bytes()
        }
        "png" => {
            if data.len() > 14 * 1024 * 1024 {
                return Err("图片不能超过 10 MB。".into());
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| "PNG 编码无效。")?;
            if bytes.len() > 10 * 1024 * 1024 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                return Err("PNG 文件无效或过大。".into());
            }
            bytes
        }
        _ => return Err("仅支持 Markdown 与 PNG 导出。".into()),
    };
    let filename = filename
        .and_then(|name| {
            let name = name.rsplit(['/', '\\']).next()?.trim();
            if name.is_empty() || name.chars().count() > 200 || name.chars().any(char::is_control) {
                return None;
            }
            let stem = Path::new(name).file_stem()?.to_str()?;
            if stem.is_empty()
                || stem == "."
                || stem == ".."
                || stem.contains([':', '*', '?', '"', '<', '>', '|'])
            {
                return None;
            }
            let filename = format!("{stem}.{format}");
            (filename.chars().count() <= 200).then_some(filename)
        })
        .unwrap_or_else(|| format!("starclip-card.{format}"));
    tauri::async_runtime::spawn_blocking(move || {
        let Some(file) = app
            .dialog()
            .file()
            .add_filter("Starclip export", &[format.as_str()])
            .set_file_name(filename)
            .blocking_save_file()
        else {
            return Ok(false);
        };
        let path = file.into_path().map_err(|_| "请选择本地文件路径。")?;
        write_private(&path, &bytes)?;
        if format == "png" {
            record_telemetry(&app, &context, "card_exported");
        }
        if let Ok(_write) = write_guard(&context) {
            reward_creation(&app, &context);
        }
        Ok(true)
    })
    .await
    .map_err(|_| "导出任务失败。".to_string())?
}
#[tauri::command]
fn get_main_window_close_request(app: AppHandle) -> Result<Option<u64>> {
    app.state::<MainWindowSession>().pending_request()
}

/// 收起主窗后把焦点交还给上一个应用。macOS 在星伴仍可见时不会自动让出激活状态，
/// 需要显式 deactivate；Windows 隐藏窗口时系统会激活下一个窗口。
fn return_focus_to_previous_app(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let _ = app.run_on_main_thread(|| {
            if let Some(mtm) = objc2::MainThreadMarker::new() {
                objc2_app_kit::NSApplication::sharedApplication(mtm).deactivate();
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

#[tauri::command]
async fn hide_window(
    app: AppHandle,
    release: Option<bool>,
    request_id: Option<u64>,
    return_focus: Option<bool>,
) -> Result<MainWindowCloseResult> {
    let request = match request_id {
        Some(request) => request,
        None => app.state::<MainWindowSession>().next_request(true)?,
    };
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = |pending| MainWindowCloseResult {
            request_id: request,
            pending,
        };
        let session = app.state::<MainWindowSession>();
        let _operation = lock(&session.operation)?;
        if !session.is_current(request)? {
            return Ok(outcome(false));
        }
        let Some(window) = app.get_webview_window("main") else {
            session.finish_request(request)?;
            return Ok(outcome(false));
        };
        window.hide().map_err(|error| error.to_string())?;
        if return_focus.unwrap_or(false) {
            return_focus_to_previous_app(&app);
        }
        if !release.unwrap_or(false) || app.state::<Updates>().ensure_idle().is_err() {
            return Ok(outcome(true));
        }
        let Some(context) = app.try_state::<Arc<Context>>() else {
            return Ok(outcome(true));
        };
        if lock(&context.storage)?.writable().is_err() {
            return Ok(outcome(true));
        }
        *lock(&session.geometry)? = MainWindowGeometry::capture(&window).ok();
        destroy_webview_and_wait(&app, &window)?;
        session.finish_request(request)?;
        Ok(outcome(false))
    })
    .await
    .map_err(|_| "收起主窗口失败，请重试。".to_string())?
}

#[tauri::command]
async fn handwriting_assets(
    context: AppContext<'_>,
) -> Result<crate::handwriting::HandwritingAssets> {
    let directory = lock(&context.storage)?.current.clone();
    tauri::async_runtime::spawn_blocking(move || crate::handwriting::assets(&directory))
        .await
        .map_err(|_| "无法读取手写字体。".to_string())
}
#[tauri::command]
async fn download_handwriting_font(
    app: AppHandle,
    context: AppContext<'_>,
    id: String,
) -> Result<crate::handwriting::HandwritingAssets> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let directory = write_guard(&context)?.current.clone();
        // A reopened panel can still show an old catalog while a previous download finishes.
        // Reuse the verified local file so that retrying also works offline.
        let installed = crate::handwriting::assets(&directory);
        let font = installed
            .fonts
            .iter()
            .find(|font| font.id == id)
            .ok_or("请选择字体列表中的字体。")?;
        if font.downloaded {
            return Ok(installed);
        }
        // Download without holding the data write gate; clipboard capture stays responsive.
        let bytes = crate::handwriting::download_font_bytes(&id)?;
        let storage = write_guard(&context)?;
        if storage.current != directory {
            return Err("数据目录已变化，请重新下载字体。".into());
        }
        let assets = crate::handwriting::install_font(&directory, &id, bytes)?;
        record_telemetry(&app, &context, "font_downloaded");
        Ok(assets)
    })
    .await
    .map_err(|_| "字体下载任务失败，请重试。".to_string())?
}
#[tauri::command]
async fn render_handwriting(
    context: AppContext<'_>,
    options: crate::handwriting::HandwritingOptions,
) -> Result<String> {
    let directory = lock(&context.storage)?.current.clone();
    tauri::async_runtime::spawn_blocking(move || crate::handwriting::render(&directory, options))
        .await
        .map_err(|_| "手写渲染任务失败。".to_string())?
}

#[tauri::command]
async fn get_data_location(context: AppContext<'_>) -> Result<DataLocation> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || lock(&context.storage)?.info())
        .await
        .map_err(|_| "读取数据目录失败。".to_string())?
}
#[tauri::command]
async fn choose_data_directory(
    app: AppHandle,
    context: AppContext<'_>,
    mode: DirectoryMode,
) -> Result<Option<DataDirectoryPlan>> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let title = if mode == DirectoryMode::Move {
            "选择父目录：将在其中新建星笺数据子目录"
        } else {
            "选择含 starclip.sqlite3 的已有星笺数据目录"
        };
        let Some(selected) = app.dialog().file().set_title(title).blocking_pick_folder() else {
            return Ok(None);
        };
        let path = selected.into_path().map_err(|_| "请选择本机目录。")?;
        lock(&context.storage)?.plan(mode, &path).map(Some)
    })
    .await
    .map_err(|_| "选择数据目录失败。".to_string())?
}
#[tauri::command]
async fn apply_data_directory(
    app: AppHandle,
    context: AppContext<'_>,
    mode: DirectoryMode,
    target_path: String,
) -> Result<DataLocation> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut storage = lock(&context.storage)?;
        let mut info = storage.info()?;
        let store = lock(&context.store)?;
        storage.apply(&store, mode, Path::new(&target_path))?;
        info.pending_path = storage.pending_path();
        changed(&app);
        Ok(info)
    })
    .await
    .map_err(|_| "切换数据目录失败。".to_string())?
}
#[tauri::command]
fn restart_app(app: AppHandle, context: AppContext<'_>) -> Result<()> {
    if app.state::<Updates>().installing() {
        return Err("正在备份或安装更新，请等待完成后再重启。".into());
    }
    lock(&context.storage)?.release_for_restart()?;
    // A synchronous command runs on the event loop. `restart()` skips Exit
    // there, so the single-instance socket can intercept the replacement.
    // Request the normal Exit sequence to clean it up before spawning again.
    app.request_restart();
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartupPreferences {
    auto_launch: bool,
    start_hidden: bool,
    show_tray_icon: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartupPreferencePatch {
    auto_launch: Option<bool>,
    start_hidden: Option<bool>,
    show_tray_icon: Option<bool>,
}
fn read_startup_preferences(app: &AppHandle, context: &Context) -> Result<StartupPreferences> {
    let local = {
        let _storage = lock(&context.storage)?;
        lock(&context.store)?.window_startup_preferences()?
    };
    Ok(StartupPreferences {
        auto_launch: crate::startup::read_enabled(app)?,
        start_hidden: local.start_hidden,
        show_tray_icon: local.show_tray_icon,
    })
}
#[tauri::command]
async fn get_startup_preferences(
    app: AppHandle,
    context: AppContext<'_>,
) -> Result<StartupPreferences> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = lock(&context.startup_operation)?;
        read_startup_preferences(&app, &context)
    })
    .await
    .map_err(|_| "无法读取启动设置。".to_string())?
}
fn apply_startup_preferences(
    context: &Context,
    patch: StartupPreferencePatch,
    read_auto_launch: impl FnOnce() -> Result<bool>,
    mut set_auto_launch: impl FnMut(bool) -> Result<()>,
    mut set_tray_visible: impl FnMut(bool) -> Result<()>,
) -> Result<StartupPreferences> {
    let _operation = lock(&context.startup_operation)?;
    let local = {
        let _storage = write_guard(context)?;
        lock(&context.store)?.window_startup_preferences()?
    };
    let previous = StartupPreferences {
        auto_launch: read_auto_launch()?,
        start_hidden: local.start_hidden,
        show_tray_icon: local.show_tray_icon,
    };
    let next = crate::store::WindowStartupPreferences {
        start_hidden: patch.start_hidden.unwrap_or(previous.start_hidden),
        show_tray_icon: patch.show_tray_icon.unwrap_or(previous.show_tray_icon),
    };
    let auto_launch = patch.auto_launch.unwrap_or(previous.auto_launch);
    let result = (|| -> Result<()> {
        if auto_launch != previous.auto_launch {
            set_auto_launch(auto_launch)?;
        }
        if next.show_tray_icon != previous.show_tray_icon {
            set_tray_visible(next.show_tray_icon)?;
        }
        // Native tray calls synchronously wait for the event loop. Never
        // hold storage there: a queued tray pause event also needs storage.
        // Recheck the write gate after external work in case an update or
        // migration began; rollback below runs outside this storage guard.
        let _storage = write_guard(context)?;
        lock(&context.store)?.save_window_startup_preferences(&next)
    })();
    if let Err(mut error) = result {
        // Restore external side effects if persistence failed. Never silently
        // claim the UI choice was saved while the OS has a different state.
        if auto_launch != previous.auto_launch {
            if let Err(rollback) = set_auto_launch(previous.auto_launch) {
                error.push_str(&format!("；恢复系统登录项失败：{rollback}"));
            }
        }
        if next.show_tray_icon != previous.show_tray_icon {
            if let Err(rollback) = set_tray_visible(previous.show_tray_icon) {
                error.push_str(&format!("；恢复图标显示失败：{rollback}"));
            }
        }
        return Err(error);
    }
    Ok(StartupPreferences {
        auto_launch,
        start_hidden: next.start_hidden,
        show_tray_icon: next.show_tray_icon,
    })
}
#[tauri::command]
async fn set_startup_preferences(
    app: AppHandle,
    context: AppContext<'_>,
    patch: StartupPreferencePatch,
) -> Result<StartupPreferences> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let tray = app
            .tray_by_id("main-tray")
            .ok_or("菜单栏 / 托盘图标不可用，请重启星笺后重试。")?;
        apply_startup_preferences(
            &context,
            patch,
            || crate::startup::read_enabled(&app),
            |enabled| crate::startup::set_enabled(&app, enabled).map(|_| ()),
            |visible| {
                tray.set_visible(visible)
                    .map_err(|error| format!("无法更改菜单栏 / 托盘图标：{error}"))
            },
        )
    })
    .await
    .map_err(|_| "无法保存启动设置。".to_string())?
}
#[tauri::command]
async fn quit_starclip(app: AppHandle, context: AppContext<'_>) -> Result<()> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Never block the native event loop while a writer waits on tray/window UI.
        let _storage = lock(&context.storage)?;
        if app.state::<Updates>().installing() {
            return Err("正在备份或安装更新，请等待完成后再退出。".into());
        }
        app.exit(0);
        Ok(())
    })
    .await
    .map_err(|_| "无法退出星笺，请稍后重试。".to_string())?
}
#[tauri::command]
async fn open_data_directory(context: AppContext<'_>) -> Result<()> {
    let directory = lock(&context.storage)?.current.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if !directory.is_dir() {
            return Err("当前数据目录无法访问，请检查磁盘连接。".into());
        }
        system_open(&directory).map_err(|error| format!("无法打开数据目录：{error}"))
    })
    .await
    .map_err(|_| "打开数据目录失败。".to_string())?
}

fn system_open(target: impl AsRef<std::ffi::OsStr>) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer.exe");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = std::process::Command::new("xdg-open");
    command.arg(target).spawn().map(|_| ())
}

fn project_link(target: &str) -> Result<&'static str> {
    match target {
        "repository" => Ok("https://github.com/cpython666/starclip"),
        "issues" => Ok("https://github.com/cpython666/starclip/issues/new"),
        _ => Err("不支持的项目链接。".into()),
    }
}

#[tauri::command]
async fn open_project_link(target: String) -> Result<()> {
    let url = project_link(&target)?;
    tauri::async_runtime::spawn_blocking(move || {
        system_open(url).map_err(|error| format!("无法打开项目链接：{error}"))
    })
    .await
    .map_err(|_| "打开项目链接失败。".to_string())?
}

fn offer_storage_recovery(app: &AppHandle, default: PathBuf, error: String) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    let handle = app.clone();
    app.dialog().message(format!("{error}\n\n为防止写入分叉，星笺没有新建或切回数据库。可以重新连接磁盘后退出重试，或选择已有数据目录恢复。"))
        .title("星笺无法打开数据目录")
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom("选择已有目录".into(), "退出".into()))
        .show(move |choose| {
            if !choose { handle.exit(0); return; }
            let app = handle.clone();
            handle.dialog().file().set_title("选择要恢复使用的已有星笺数据目录").pick_folder(move |selected| {
                let Some(selected) = selected else { app.exit(0); return; };
                thread::spawn(move || {
                    let result = selected.into_path().map_err(|_| "请选择本机目录。".to_string()).and_then(|path| Storage::recover(&default, &path));
                    match result {
                        Ok(()) => app.restart(),
                        Err(error) => offer_storage_recovery(&app, default, error),
                    }
                });
            });
        });
}

fn offer_upgrade_recovery(app: &AppHandle, default: PathBuf, error: String) {
    let latest = Storage::recovery_points_at_startup(&default)
        .ok()
        .and_then(|points| points.into_iter().find(|point| point.compatible));
    let Some(point) = latest else {
        offer_storage_recovery(app, default, error);
        return;
    };
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    let handle = app.clone();
    app.dialog().message(format!("{error}\n\n可恢复到最近的恢复点 {}（应用版本 {}）。恢复将在新目录中完成并重启，当前目录和恢复点均保留；恢复点之后的变动不会出现在还原结果中。", point.id, point.app_version))
        .title("星笺数据升级与恢复")
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom("恢复并重启".into(), "其他方式".into()))
        .show(move |restore| {
            if !restore { offer_storage_recovery(&handle, default, error); return; }
            thread::spawn(move || match Storage::restore_recovery_point_at_startup(&default, &point.id) {
                Ok(_) => handle.restart(),
                Err(error) => offer_storage_recovery(&handle, default, error),
            });
        });
}

#[tauri::command]
fn get_update_status(app: AppHandle) -> Result<UpdateStatus> {
    app.state::<Updates>().status()
}

async fn check_update_with_context(
    app: AppHandle,
    context: Arc<Context>,
    automatic: bool,
) -> Result<UpdateStatus> {
    {
        let _storage = write_guard(&context)?;
        let store = lock(&context.store)?;
        let started = app.state::<Updates>().begin_check(&app, automatic, || {
            let previous = store.update_preferences()?;
            let (preferences, started) = store.record_update_check(unix_millis(), automatic)?;
            if preferences.revision != previous.revision {
                let _ = app.emit_to("main", "starclip-update-preferences", &preferences);
            }
            Ok(started)
        })?;
        if !started {
            return app.state::<Updates>().status();
        }
    }
    let before_exit = app.clone();
    let cleanup_started = app.state::<Updates>().cleanup_flag()?;
    let result = async {
        let update = app
            .updater_builder()
            .timeout(Duration::from_secs(30))
            .restart_after_install(true)
            .on_before_exit(move || {
                cleanup_started.store(true, std::sync::atomic::Ordering::SeqCst);
                before_exit.cleanup_before_exit();
            })
            .build()
            .map_err(|error| format!("更新配置无法读取：{error}"))?
            .check()
            .await
            .map_err(updater::check_error_message)?;
        app.state::<Updates>().checked(&app, update)
    }
    .await;
    match result {
        Ok(()) => {
            record_telemetry(&app, &context, "update_check_completed");
            app.state::<Updates>().status()
        }
        Err(error) => app.state::<Updates>().failed_check(&app, error, automatic),
    }
}

#[tauri::command]
async fn check_starclip_update(app: AppHandle, context: AppContext<'_>) -> Result<UpdateStatus> {
    check_update_with_context(app, context.inner().clone(), false).await
}

#[tauri::command]
fn get_update_preferences(context: AppContext<'_>) -> Result<UpdatePreferences> {
    lock(&context.store)?.update_preferences()
}

#[tauri::command]
fn set_auto_update_check(
    app: AppHandle,
    context: AppContext<'_>,
    enabled: bool,
) -> Result<UpdatePreferences> {
    let _storage = write_guard(&context)?;
    let preferences = lock(&context.store)?.set_auto_update_check(enabled)?;
    let _ = app.emit_to("main", "starclip-update-preferences", &preferences);
    Ok(preferences)
}

#[tauri::command]
fn skip_update_version(
    app: AppHandle,
    context: AppContext<'_>,
    version: String,
) -> Result<UpdatePreferences> {
    let _storage = write_guard(&context)?;
    let store = lock(&context.store)?;
    let preferences = app
        .state::<Updates>()
        .for_available_version(&version, || store.skip_update_version(&version))?;
    let _ = app.emit_to("main", "starclip-update-preferences", &preferences);
    Ok(preferences)
}

#[tauri::command]
async fn download_starclip_update(
    app: AppHandle,
    context: AppContext<'_>,
    expected_version: String,
) -> Result<UpdateStatus> {
    lock(&context.storage)?.writable()?;
    let update = app
        .state::<Updates>()
        .begin_download(&app, &expected_version)?;
    let result: Result<()> = async {
        let mut downloaded = 0_u64;
        let mut last_progress = Instant::now();
        let bytes = update
            .download(
                |chunk, total| {
                    downloaded += chunk as u64;
                    if last_progress.elapsed() >= Duration::from_millis(100)
                        || total == Some(downloaded)
                    {
                        app.state::<Updates>().progress(&app, downloaded, total);
                        last_progress = Instant::now();
                    }
                },
                || {},
            )
            .await
            .map_err(|error| format!("下载或签名校验失败，未开始安装：{error}"))?;
        // download() verifies both the package signature and its bound version.
        // Keep it only in this session until the user confirms installation.
        app.state::<Updates>()
            .downloaded(&app, &expected_version, bytes)
    }
    .await;
    if let Err(error) = result {
        return app.state::<Updates>().failed(&app, error);
    }
    record_telemetry(&app, &context, "update_download_completed");
    schedule_telemetry_flush(&app, context.inner());
    app.state::<Updates>().status()
}

#[tauri::command]
async fn install_starclip_update(
    app: AppHandle,
    context: AppContext<'_>,
    expected_version: String,
) -> Result<UpdateStatus> {
    lock(&context.storage)?.writable()?;
    let (update, bytes, cleanup_started) = app
        .state::<Updates>()
        .begin_install(&app, &expected_version)?;
    let context = context.inner().clone();
    let result: Result<()> = async {
        let worker_app = app.clone();
        let worker_context = context.clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<()> {
            let mut storage = write_guard(&worker_context)?;
            let point =
                storage.create_recovery_point(&*lock(&worker_context.store)?, "before-update")?;
            worker_app
                .state::<Updates>()
                .backed_up(&worker_app, point.id.clone())?;
            updater::preserve_application(storage.default_directory(), &point.id)?;
            storage.freeze_for_update()?;
            drop(storage);
            worker_app
                .state::<Updates>()
                .phase(&worker_app, "installing")?;
            // Count the installation attempt after all prerequisites succeed.
            // Windows may exit inside install(); never wait on telemetry here.
            record_telemetry(&worker_app, &worker_context, "update_install_started");
            schedule_telemetry_flush(&worker_app, &worker_context);
            update
                .install(bytes)
                .map_err(|error| format!("安装未完成，旧版应用和数据恢复点已保留：{error}"))?;
            worker_app
                .state::<Updates>()
                .phase(&worker_app, "installed")?;
            Ok(())
        })
        .await
        .map_err(|_| "更新任务异常终止；已生成的恢复点仍保留。".to_string())?
    }
    .await;
    if let Err(error) = result {
        if cleanup_started.load(std::sync::atomic::Ordering::SeqCst) {
            return app.state::<Updates>().restart_required(
                &app,
                format!("安装器未能成功启动，当前会话已停止写入。请重启星笺后再试。{error}"),
            );
        }
        lock(&context.storage)?.cancel_update_freeze();
        return app.state::<Updates>().failed(&app, error);
    }
    app.state::<Updates>().status()
}

#[tauri::command]
async fn list_recovery_points(context: AppContext<'_>) -> Result<Vec<RecoveryPoint>> {
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || lock(&context.storage)?.recovery_points())
        .await
        .map_err(|_| "读取恢复点失败。".to_string())?
}

#[tauri::command]
async fn create_recovery_point(app: AppHandle, context: AppContext<'_>) -> Result<RecoveryPoint> {
    app.state::<Updates>().ensure_idle()?;
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let storage = write_guard(&context)?;
        storage.create_recovery_point(&*lock(&context.store)?, "manual")
    })
    .await
    .map_err(|_| "创建恢复点失败。".to_string())?
}

#[tauri::command]
async fn restore_recovery_point(
    app: AppHandle,
    context: AppContext<'_>,
    id: String,
) -> Result<RecoveryPoint> {
    app.state::<Updates>().ensure_idle()?;
    let context = context.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut storage = write_guard(&context)?;
        let point = storage.restore_recovery_point(&*lock(&context.store)?, &id)?;
        changed(&app);
        Ok(point)
    })
    .await
    .map_err(|_| "恢复任务异常，请检查当前目录状态。".to_string())?
}

fn refresh_tray(app: &AppHandle, context: &Context, settings: &Settings) {
    let english = native_is_english(context);
    if let Some(items) = context.tray_items.get() {
        let _ = items.show.set_text(if english {
            "Open Starclip"
        } else {
            "打开星笺"
        });
        let _ = items.starpet.set_text(if english {
            "Show / Hide StarPet"
        } else {
            "显示 / 隐藏星伴"
        });
        let _ = items.quit.set_text(if english {
            "Quit Starclip"
        } else {
            "退出星笺"
        });
        let (pause_label, _) = tray_labels(english, settings.paused);
        let _ = items.pause.set_text(pause_label);
    }
    if let Some(tray) = app.tray_by_id("main-tray") {
        let (_, tooltip) = tray_labels(english, settings.paused);
        let _ = tray.set_tooltip(Some(tooltip));
    }
}
#[cfg(target_os = "macos")]
fn status_star_icon() -> tauri::image::Image<'static> {
    // A 36px transparent template becomes an 18pt, system-colored menu-bar star.
    let points = [
        (18.0, 2.0),
        (22.0, 13.0),
        (34.0, 18.0),
        (22.0, 22.0),
        (18.0, 34.0),
        (13.0, 22.0),
        (2.0, 18.0),
        (13.0, 13.0),
    ];
    let mut rgba = vec![0; 36 * 36 * 4];
    for y in 0..36 {
        for x in 0..36 {
            let mut samples = 0;
            for dy in [0.25, 0.75] {
                for dx in [0.25, 0.75] {
                    let (px, py) = (x as f64 + dx, y as f64 + dy);
                    let mut inside = false;
                    for i in 0..points.len() {
                        let (a, b) = (points[i], points[(i + 1) % points.len()]);
                        if (a.1 > py) != (b.1 > py)
                            && px < (b.0 - a.0) * (py - a.1) / (b.1 - a.1) + a.0
                        {
                            inside = !inside;
                        }
                    }
                    if inside {
                        samples += 1;
                    }
                }
            }
            rgba[(y * 36 + x) * 4 + 3] = (samples * 255 / 4) as u8;
        }
    }
    tauri::image::Image::new_owned(rgba, 36, 36)
}

fn schedule_auto_update_check(app: &AppHandle, context: &Arc<Context>) {
    let app = app.clone();
    let context = context.clone();
    tauri::async_runtime::spawn(async move {
        // Re-check the persisted switch and throttle when the task actually
        // starts. Closing the main WebView neither cancels nor triggers checks.
        let _ = check_update_with_context(app, context, true).await;
    });
}

pub fn run() {
    let app_context = tauri::generate_context!();
    let startup_plugin = crate::startup::init(&app_context.config().identifier);
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_window(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(startup_plugin)
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Updates::new())
        .manage(MainWindowSession::default())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_window(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let default = app.path().app_data_dir()?;
            let storage = match Storage::open(&default, &app.path().app_config_dir()?) {
                Ok(storage) => storage,
                Err(error) => {
                    offer_upgrade_recovery(app.handle(), default, error);
                    return Ok(());
                }
            };
            let db_path = storage.current.join("starclip.sqlite3");
            let store = match storage.open_store() {
                Ok(store) => store,
                Err(error) => {
                    offer_upgrade_recovery(app.handle(), default, error);
                    return Ok(());
                }
            };
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&db_path, fs::Permissions::from_mode(0o600))?;
            }
            let shortcut_error = store
                .settings()
                .and_then(|settings| {
                    let shortcut: Shortcut = settings
                        .shortcut
                        .parse()
                        .map_err(|_| "已保存的快捷键无效。".to_string())?;
                    app.global_shortcut()
                        .register(shortcut)
                        .map_err(|_| "快捷键注册失败，可在偏好设置中更换。".to_string())
                })
                .err();
            let (window_startup, window_error) = match store.window_startup_preferences() {
                Ok(preferences) => (preferences, None),
                Err(error) => (Default::default(), Some(error)),
            };
            // A broken shortcut or preference must never leave an invisible app
            // with no obvious way to recover its controls.
            let show_at_startup =
                !window_startup.start_hidden || shortcut_error.is_some() || window_error.is_some();
            let saved_language = store.settings()?.ui_language;
            let native_language = if saved_language == "system" {
                if sys_locale::get_locale().is_some_and(|locale| locale.starts_with("zh")) {
                    "zh-CN".to_string()
                } else {
                    "en".to_string()
                }
            } else {
                saved_language
            };
            let context = Arc::new(Context {
                telemetry: Mutex::new(
                    Telemetry::new(store.telemetry_preferences())
                        .with_cache(default.join("usage-counts.json"))
                        .with_install(default.join("usage-install.json")),
                ),
                storage: Mutex::new(storage),
                ai: Mutex::new(AiAccess::default()),
                store: Mutex::new(store),
                reader: Mutex::new(Reader {
                    clipboard: None,
                    capture: CaptureState::default(),
                }),
                clipboard_error: Mutex::new(None),
                shortcut_error: Mutex::new(shortcut_error),
                window_error: Mutex::new(window_error),
                cleanup: Mutex::new(None),
                starpet_operation: Mutex::new(()),
                startup_operation: Mutex::new(()),
                tray_items: OnceLock::new(),
                native_language: Mutex::new(native_language),
            });
            let pet_app = app.handle().clone();
            start_ai_broker(&context, move |snapshot| {
                // No show/focus or synchronous native-window queries under the
                // broker locks. A disabled/destroyed pet cannot be awakened by AI.
                if pet_app.get_webview_window("starpet").is_none() {
                    return Err("星伴窗口不可用。".into());
                }
                pet_app
                    .emit_to("starpet", "starpet-signal", snapshot)
                    .map_err(|_| "星伴消息未能发送。".into())
            });
            app.manage(context.clone());
            let show = MenuItem::with_id(app, "show", "打开星笺", true, None::<&str>)?;
            let pause = MenuItem::with_id(app, "pause", "暂停记录", true, None::<&str>)?;
            let starpet = MenuItem::with_id(app, "starpet", "显示 / 隐藏星伴", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出星笺", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &pause, &starpet, &quit])?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .tooltip("星笺 Starclip")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_window(app),
                    "pause" => {
                        let context = app.state::<Arc<Context>>();
                        let result = (|| -> Result<()> {
                            let mut settings = lock(&context.store)?.settings()?;
                            let expected_pause_revision = settings.pause_revision;
                            settings.paused = !settings.paused;
                            save_settings(
                                app.clone(),
                                context,
                                settings,
                                Some(expected_pause_revision),
                            )
                        })();
                        if let Err(error) = result {
                            let context = app.state::<Arc<Context>>();
                            set_clipboard_error(app, &context, Some(error));
                        }
                    }
                    "quit" => app.exit(0),
                    "starpet" => {
                        let app = app.clone();
                        let context = app.state::<Arc<Context>>().inner().clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            if let Err(error) = update_starpet(&app, &context, |value| {
                                value.enabled = !value.enabled
                            }) {
                                starpet_error(&app, error);
                            }
                        });
                    }
                    _ => {}
                });
            #[cfg(target_os = "macos")]
            {
                tray = tray.icon(status_star_icon()).icon_as_template(true);
            }
            #[cfg(not(target_os = "macos"))]
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            let _ = context.tray_items.set(TrayItems {
                show,
                pause,
                starpet,
                quit,
            });
            let tray = tray.build(app)?;
            if !window_startup.show_tray_icon {
                tray.set_visible(false)?;
            }
            let startup_settings = lock(&context.store)?.settings()?;
            refresh_tray(app.handle(), &context, &startup_settings);
            apply_window_titles(app.handle(), native_is_english(&context));
            let pet_app = app.handle().clone();
            let pet_context = context.clone();
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(error) = update_starpet(&pet_app, &pet_context, |_| {}) {
                    starpet_error(&pet_app, error);
                }
            });
            record_telemetry(app.handle(), &context, "app_started");
            monitor(app.handle().clone(), context);
            if show_at_startup {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(error) = show_window_inner(&handle, false) {
                        handle
                            .dialog()
                            .message(error)
                            .title(native_dialog_title(native_is_english_app(&handle)))
                            .show(|_| {});
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                match window.label() {
                    "main" => {
                        api.prevent_close();
                        request_main_window_close(window.app_handle());
                    }
                    "starpet" => {
                        api.prevent_close();
                        let app = window.app_handle().clone();
                        let context = app.state::<Arc<Context>>().inner().clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            if let Err(error) =
                                update_starpet(&app, &context, |value| value.enabled = false)
                            {
                                starpet_error(&app, error);
                            }
                        });
                    }
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_telemetry_state,
            set_telemetry_enabled,
            get_clip_stats,
            clear_clip_stats,
            record_usage_event,
            flush_telemetry,
            clear_telemetry_queue,
            reset_telemetry_install,
            get_state,
            get_startup_preferences,
            set_startup_preferences,
            quit_starclip,
            get_clips,
            get_starpet_preferences,
            get_starpet_signal,
            set_starpet_preferences,
            show_main_window,
            starpet_start_dragging,
            starpet_dock,
            get_starpet_growth,
            set_starpet_growth_enabled,
            interact_starpet,
            equip_starpet_item,
            export_starpet_growth,
            import_starpet_growth,
            reset_starpet_growth,
            complete_onboarding,
            add_clip,
            update_clip,
            save_code,
            delete_clip,
            copy_clip,
            copy_product_share_text,
            copy_product_share_image,
            save_product_share_image,
            get_clip_image,
            create_group,
            delete_group,
            save_settings,
            set_native_language,
            set_theme,
            get_privacy_rules,
            save_privacy_rules,
            set_clip_privacy,
            set_clip_tags,
            preview_privacy_rescan,
            apply_privacy_rescan,
            get_ai_state,
            set_ai_enabled,
            create_ai_connection,
            revoke_ai_connection,
            get_ai_connection_config,
            clear_ai_audit,
            get_appearance,
            save_appearance,
            import_appearance_sticker,
            remove_appearance_sticker,
            clear_history,
            set_keep_forever,
            preview_cleanup,
            cancel_cleanup,
            execute_cleanup,
            export_backup,
            import_backup,
            save_backup,
            save_export,
            hide_window,
            get_main_window_close_request,
            handwriting_assets,
            download_handwriting_font,
            render_handwriting,
            get_data_location,
            choose_data_directory,
            apply_data_directory,
            open_data_directory,
            open_project_link,
            get_update_status,
            get_update_preferences,
            set_auto_update_check,
            skip_update_version,
            check_starclip_update,
            download_starclip_update,
            install_starclip_update,
            list_recovery_points,
            create_recovery_point,
            restore_recovery_point,
            restart_app
        ])
        .build(app_context)
        .expect("星笺启动失败，请检查本地数据目录权限或磁盘空间。")
        .run(|app, event| match event {
            // A visible starpet does not mean the main window is open.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_window(app),
            tauri::RunEvent::ExitRequested { api, .. } if app.state::<Updates>().installing() => {
                api.prevent_exit();
                show_window(app);
            }
            tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } if app.try_state::<Arc<Context>>().is_some() => {
                // Releasing the last WebView must not stop clipboard capture,
                // the tray, shortcuts or the local AI broker. Explicit quit /
                // restart requests carry an exit code and remain permitted.
                api.prevent_exit();
            }
            tauri::RunEvent::Exit => {
                if let Some(context) = app.try_state::<Arc<Context>>() {
                    if let Ok(mut telemetry) = lock(&context.telemetry) {
                        telemetry.checkpoint();
                    }
                }
            }
            _ => {}
        });
}

mod ai_broker;
mod clipboard;
use ai_broker::*;
use clipboard::*;

#[cfg(test)]
mod tests;
