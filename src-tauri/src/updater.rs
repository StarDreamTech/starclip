use crate::store::Result;
use serde::Serialize;
use std::{
    fs,
    path::Path,
    sync::{atomic::AtomicBool, Arc, Mutex},
};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::Update;

pub fn check_error_message(error: tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error;
    match error {
        // The plugin discards the HTTP status for unsuccessful responses, so
        // this alone cannot distinguish an unavailable asset from access errors.
        Error::ReleaseNotFound => {
            "无法从 GitHub 获取更新信息。可能是更新链接访问失败，或更新文件尚未发布，请稍后重试。"
        }
        Error::Reqwest(error) if error.is_timeout() => {
            "连接 GitHub 更新服务超时，请检查网络后重试。"
        }
        Error::Reqwest(error) if error.is_decode() => {
            "GitHub 更新链接返回的内容无法识别，暂时无法检查更新，请稍后重试。"
        }
        Error::Reqwest(_) | Error::Network(_) => "GitHub 更新链接访问失败，请检查网络连接后重试。",
        Error::Serialization(_) | Error::Semver(_) => {
            "GitHub 上的更新信息格式有误，暂时无法检查更新，请稍后重试。"
        }
        Error::TargetNotFound(_) | Error::TargetsNotFound(_) => {
            "GitHub 上的新版本暂未提供适用于当前系统和架构的更新包。"
        }
        Error::UnsupportedArch | Error::UnsupportedOs => "当前系统或架构暂不支持应用内更新。",
        _ => "检查更新未能完成，请稍后重试。",
    }
    .into()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    version: String,
    date: Option<String>,
    notes: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    revision: u64,
    current_version: String,
    configured: bool,
    phase: &'static str,
    available: Option<AvailableUpdate>,
    downloaded: u64,
    total: Option<u64>,
    error: Option<String>,
    backup_id: Option<String>,
}

struct Pending {
    status: UpdateStatus,
    update: Option<Update>,
    downloaded_package: Option<Vec<u8>>,
    cleanup_started: Arc<AtomicBool>,
}

impl Pending {
    fn begin_check(&mut self, automatic: bool) -> Result<()> {
        self.ensure_checkable(automatic)?;
        if !automatic {
            self.update = None;
            self.status.available = None;
        }
        self.downloaded_package = None;
        self.cleanup_started = Arc::new(AtomicBool::new(false));
        self.status.phase = "checking";
        self.status.error = None;
        self.status.downloaded = 0;
        self.status.total = None;
        self.status.backup_id = None;
        Ok(())
    }

    fn ensure_checkable(&self, automatic: bool) -> Result<()> {
        ensure_idle(&self.status)?;
        if automatic && self.status.phase == "ready" {
            return Err("已下载的更新等待安装，暂不自动检查。".into());
        }
        Ok(())
    }

    fn failed_check(&mut self, error: String, automatic: bool) {
        self.status.error = Some(error);
        self.status.phase = if automatic && self.status.available.is_some() {
            "available"
        } else {
            "error"
        };
    }

    fn downloaded(&mut self, expected: &str, bytes: Vec<u8>) -> Result<()> {
        if self.status.phase != "downloading" {
            return Err("下载状态已变化，请重新检查更新。".into());
        }
        let available = self.status.available.as_ref().ok_or("请先检查更新。")?;
        validate_expected_version(&available.version, expected)?;
        if bytes.is_empty() {
            return Err("更新安装包为空，未开始安装。".into());
        }
        self.status.downloaded = bytes.len() as u64;
        self.status.total = Some(bytes.len() as u64);
        self.downloaded_package = Some(bytes);
        self.status.phase = "ready";
        Ok(())
    }

    fn take_downloaded(&mut self, expected: &str) -> Result<Vec<u8>> {
        ensure_idle(&self.status)?;
        if self.status.phase != "ready" {
            return Err("请先下载并验证更新，再确认安装。".into());
        }
        let available = self.status.available.as_ref().ok_or("请先检查更新。")?;
        validate_expected_version(&available.version, expected)?;
        let bytes = self
            .downloaded_package
            .take()
            .ok_or("下载的更新已失效，请重新检查并下载。")?;
        self.status.phase = "backing-up";
        self.status.error = None;
        self.status.backup_id = None;
        Ok(bytes)
    }

    fn failed(&mut self, error: String) {
        self.downloaded_package = None;
        self.status.phase = "error";
        self.status.error = Some(error);
    }
}

pub struct Updates(Mutex<Pending>);

impl Updates {
    pub fn new() -> Self {
        Self(Mutex::new(Pending {
            status: UpdateStatus {
                revision: 0,
                current_version: env!("CARGO_PKG_VERSION").into(),
                configured: true,
                phase: "idle",
                available: None,
                downloaded: 0,
                total: None,
                error: None,
                backup_id: None,
            },
            update: None,
            downloaded_package: None,
            cleanup_started: Arc::new(AtomicBool::new(false)),
        }))
    }
    fn change<T>(
        &self,
        app: &AppHandle,
        action: impl FnOnce(&mut Pending) -> Result<T>,
    ) -> Result<T> {
        let mut pending = self.0.lock().map_err(|_| "更新状态异常，请重启星笺。")?;
        let result = action(&mut pending)?;
        pending.status.revision += 1;
        let _ = app.emit_to("main", "starclip-update-status", &pending.status);
        Ok(result)
    }
    pub fn status(&self) -> Result<UpdateStatus> {
        Ok(self
            .0
            .lock()
            .map_err(|_| "更新状态异常，请重启星笺。")?
            .status
            .clone())
    }
    pub fn ensure_idle(&self) -> Result<()> {
        ensure_idle(
            &self
                .0
                .lock()
                .map_err(|_| "更新状态异常，请重启星笺。")?
                .status,
        )
    }
    pub fn installing(&self) -> bool {
        self.0
            .lock()
            .map(|pending| matches!(pending.status.phase, "backing-up" | "installing"))
            .unwrap_or(true)
    }
    pub fn begin_check(
        &self,
        app: &AppHandle,
        automatic: bool,
        record_attempt: impl FnOnce() -> Result<bool>,
    ) -> Result<bool> {
        let mut pending = self.0.lock().map_err(|_| "更新状态异常，请重启星笺。")?;
        pending.ensure_checkable(automatic)?;
        // Persist the attempt only once the updater has reserved this check.
        // A competing download/install cannot slip in and lose its package.
        if !record_attempt()? {
            return Ok(false);
        }
        pending.begin_check(automatic)?;
        pending.status.revision += 1;
        let _ = app.emit_to("main", "starclip-update-status", &pending.status);
        Ok(true)
    }
    pub fn for_available_version<T>(
        &self,
        expected: &str,
        save: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let pending = self.0.lock().map_err(|_| "更新状态异常，请重启星笺。")?;
        let available = pending.status.available.as_ref().ok_or("请先检查更新。")?;
        validate_expected_version(&available.version, expected)?;
        save()
    }
    pub fn checked(&self, app: &AppHandle, mut update: Option<Update>) -> Result<()> {
        if update
            .as_ref()
            .is_some_and(|candidate| !is_stable_release(&candidate.version, &candidate.raw_json))
        {
            update = None;
        }
        if let Some(update) = &mut update {
            validate_download_url(update.download_url.as_str())?;
            update.timeout = Some(std::time::Duration::from_secs(600));
        }
        self.change(app, |pending| {
            pending.status.available = update.as_ref().map(|update| AvailableUpdate {
                version: update.version.clone(),
                date: update
                    .raw_json
                    .get("pub_date")
                    .and_then(|date| date.as_str())
                    .map(str::to_owned),
                notes: update.body.clone(),
            });
            pending.status.phase = if update.is_some() {
                "available"
            } else {
                "up-to-date"
            };
            pending.update = update;
            Ok(())
        })
    }
    pub fn cleanup_flag(&self) -> Result<Arc<AtomicBool>> {
        Ok(self
            .0
            .lock()
            .map_err(|_| "更新状态异常，请重启星笺。")?
            .cleanup_started
            .clone())
    }
    pub fn begin_download(&self, app: &AppHandle, expected: &str) -> Result<Update> {
        self.change(app, |pending| {
            ensure_idle(&pending.status)?;
            if !matches!(pending.status.phase, "available" | "error") {
                return Err("请先检查更新；已下载的版本可以直接确认安装。".into());
            }
            let update = pending.update.as_ref().ok_or("请先检查更新。")?;
            validate_expected_version(&update.version, expected)?;
            pending.downloaded_package = None;
            pending.status.phase = "downloading";
            pending.status.error = None;
            pending.status.downloaded = 0;
            pending.status.total = None;
            pending.status.backup_id = None;
            Ok(update.clone())
        })
    }
    pub fn downloaded(&self, app: &AppHandle, expected: &str, bytes: Vec<u8>) -> Result<()> {
        self.change(app, |pending| pending.downloaded(expected, bytes))
    }
    pub fn begin_install(
        &self,
        app: &AppHandle,
        expected: &str,
    ) -> Result<(Update, Vec<u8>, Arc<AtomicBool>)> {
        self.change(app, |pending| {
            let update = pending.update.as_ref().ok_or("请先检查更新。")?;
            validate_expected_version(&update.version, expected)?;
            let update = update.clone();
            let bytes = pending.take_downloaded(expected)?;
            Ok((update, bytes, pending.cleanup_started.clone()))
        })
    }
    pub fn progress(&self, app: &AppHandle, downloaded: u64, total: Option<u64>) {
        let _ = self.change(app, |pending| {
            pending.status.downloaded = downloaded;
            pending.status.total = total;
            Ok(())
        });
    }
    pub fn phase(&self, app: &AppHandle, phase: &'static str) -> Result<()> {
        self.change(app, |pending| {
            pending.status.phase = phase;
            Ok(())
        })
    }
    pub fn backed_up(&self, app: &AppHandle, id: String) -> Result<()> {
        self.change(app, |pending| {
            pending.status.backup_id = Some(id);
            Ok(())
        })
    }
    pub fn failed(&self, app: &AppHandle, error: String) -> Result<UpdateStatus> {
        self.change(app, |pending| {
            pending.failed(error);
            Ok(())
        })?;
        self.status()
    }
    pub fn failed_check(
        &self,
        app: &AppHandle,
        error: String,
        automatic: bool,
    ) -> Result<UpdateStatus> {
        self.change(app, |pending| {
            pending.failed_check(error, automatic);
            Ok(())
        })?;
        self.status()
    }
    pub fn restart_required(&self, app: &AppHandle, error: String) -> Result<UpdateStatus> {
        self.failed(app, error)?;
        self.phase(app, "restart-required")?;
        self.status()
    }
}

fn is_stable_release(version: &str, metadata: &serde_json::Value) -> bool {
    // Missing is allowed for Tauri's canonical static JSON. Any present value
    // other than false is rejected, including malformed prerelease metadata.
    if metadata
        .get("prerelease")
        .is_some_and(|value| value != &serde_json::Value::Bool(false))
    {
        return false;
    }
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && (*part == "0" || !part.starts_with('0'))
                && part.parse::<u64>().is_ok()
                && part.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn ensure_idle(status: &UpdateStatus) -> Result<()> {
    if matches!(
        status.phase,
        "checking" | "downloading" | "backing-up" | "installing" | "installed" | "restart-required"
    ) {
        Err("更新操作正在进行或等待重启，请先完成当前操作。".into())
    } else {
        Ok(())
    }
}
fn validate_expected_version(actual: &str, expected: &str) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err("可用版本已变化，请重新检查并确认更新。".into())
    }
}
fn validate_download_url(url: &str) -> Result<()> {
    // The signed bytes remain authoritative; restrict metadata's download origin too.
    // This build accepts only StarDreamTech/starclip. Clients through 0.2.5 still
    // require https://github.com/cpython666/starclip/releases/download/. That prefix
    // belongs on the separately mirrored compatibility manifest, not this repository's latest.json.
    if url.starts_with("https://github.com/StarDreamTech/starclip/releases/download/") {
        Ok(())
    } else {
        Err("更新安装包地址不属于星笺正式发布源，已停止更新。".into())
    }
}

/// Preserve the executable separately from data snapshots. Never overwrite it.
pub fn preserve_application(default: &Path, backup_id: &str) -> Result<()> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let directory = default.join("application-backups").join(backup_id);
    #[cfg(target_os = "macos")]
    let source = executable
        .ancestors()
        .nth(3)
        .filter(|path| path.extension().is_some_and(|ext| ext == "app"))
        .ok_or("请先将星笺安装到应用程序目录，再使用应用内更新。")?;
    fs::create_dir_all(directory.parent().ok_or("应用备份目录无效。")?)
        .map_err(|error| error.to_string())?;
    fs::create_dir(&directory).map_err(|error| format!("无法创建旧版应用备份：{error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/usr/bin/ditto")
            .arg(source)
            .arg(directory.join("Starclip.app"))
            .output()
            .map_err(|error| format!("旧版应用备份失败，未开始安装：{error}"))?;
        if !output.status.success() {
            return Err("旧版应用备份失败，未开始安装。请检查磁盘空间与目录权限。".into());
        }
    }
    #[cfg(not(target_os = "macos"))]
    fs::copy(
        &executable,
        directory.join(executable.file_name().ok_or("应用路径无效。")?),
    )
    .map_err(|error| format!("旧版应用备份失败，未开始安装：{error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_failures_name_github_without_guessing_network_or_exposing_raw_errors() {
        use tauri_plugin_updater::Error;
        let unavailable = check_error_message(Error::ReleaseNotFound);
        assert!(unavailable.contains("GitHub"));
        assert!(unavailable.contains("更新链接访问失败"));
        assert!(unavailable.contains("更新文件尚未发布"));
        assert!(!unavailable.contains("Could not fetch"));
        assert!(!unavailable.contains("当前没有更新"));
        let network = check_error_message(Error::Network("fixed internal detail".into()));
        assert!(network.contains("检查网络连接"));
        assert!(!network.contains("fixed internal detail"));
        let malformed = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        assert!(check_error_message(Error::Serialization(malformed)).contains("格式有误"));
        assert!(
            check_error_message(Error::TargetNotFound("fixed-platform".into()))
                .contains("当前系统和架构")
        );
    }
    #[test]
    fn update_validation_rejects_changed_confirmation_and_untrusted_origin() {
        assert!(validate_expected_version("0.3.0", "0.2.0").is_err());
        assert!(validate_expected_version("0.3.0", "0.3.0").is_ok());
        assert!(validate_download_url(
            "https://github.com/StarDreamTech/starclip/releases/download/v0.3.0/app.tar.gz"
        )
        .is_ok());
        for url in [
            "http://github.com/StarDreamTech/starclip/releases/download/a",
            "https://github.com.evil/StarDreamTech/starclip/releases/download/a",
            "https://github.com/other/starclip/releases/download/a",
        ] {
            assert!(validate_download_url(url).is_err());
        }
        let updates = Updates::new();
        let mut status = updates.status().unwrap();
        for phase in [
            "checking",
            "downloading",
            "backing-up",
            "installing",
            "installed",
            "restart-required",
        ] {
            status.phase = phase;
            assert!(ensure_idle(&status).is_err());
        }
        for phase in ["idle", "error", "available", "ready", "up-to-date"] {
            status.phase = phase;
            assert!(ensure_idle(&status).is_ok());
        }
    }

    fn downloading() -> Pending {
        let mut pending = Updates::new().0.into_inner().unwrap();
        pending.status.phase = "downloading";
        pending.status.available = Some(AvailableUpdate {
            version: "0.3.0".into(),
            date: None,
            notes: None,
        });
        pending
    }

    #[test]
    fn downloaded_update_waits_for_matching_confirmation_and_can_be_taken_only_once() {
        let mut pending = downloading();
        assert!(pending.take_downloaded("0.3.0").is_err());
        pending.downloaded("0.3.0", vec![1, 2, 3]).unwrap();
        assert_eq!(pending.status.phase, "ready");
        assert!(ensure_idle(&pending.status).is_ok());
        assert_eq!(pending.status.downloaded, 3);
        assert!(pending.take_downloaded("0.4.0").is_err());
        assert_eq!(pending.status.phase, "ready");
        assert!(pending.downloaded_package.is_some());

        assert_eq!(pending.take_downloaded("0.3.0").unwrap(), vec![1, 2, 3]);
        assert_eq!(pending.status.phase, "backing-up");
        assert!(pending.downloaded_package.is_none());
        assert!(pending.take_downloaded("0.3.0").is_err());
    }

    #[test]
    fn rechecking_or_failure_discards_ready_bytes_and_rejects_late_downloads() {
        let mut pending = downloading();
        pending.downloaded("0.3.0", vec![1]).unwrap();
        pending.begin_check(false).unwrap();
        assert!(pending.downloaded_package.is_none());
        assert!(pending.status.available.is_none());
        assert_eq!(pending.status.downloaded, 0);
        assert!(pending.downloaded("0.3.0", vec![2]).is_err());
        assert!(pending.take_downloaded("0.3.0").is_err());

        let mut pending = downloading();
        pending.downloaded("0.3.0", vec![1]).unwrap();
        pending.failed("备份未完成".into());
        assert!(pending.downloaded_package.is_none());
        assert_eq!(pending.status.phase, "error");
        assert!(pending.take_downloaded("0.3.0").is_err());
    }

    #[test]
    fn automatic_checks_preserve_ready_bytes_and_failed_checks_keep_the_candidate() {
        let mut pending = downloading();
        pending.downloaded("0.3.0", vec![1, 2]).unwrap();
        assert!(pending.begin_check(true).is_err());
        assert_eq!(pending.status.phase, "ready");
        assert_eq!(
            pending.downloaded_package.as_deref(),
            Some([1, 2].as_slice())
        );
        assert_eq!(pending.status.available.as_ref().unwrap().version, "0.3.0");
        for phase in [
            "checking",
            "downloading",
            "backing-up",
            "installing",
            "installed",
            "restart-required",
        ] {
            pending.status.phase = phase;
            assert!(pending.begin_check(true).is_err());
        }
        pending.status.phase = "available";
        pending.downloaded_package = None;
        pending.begin_check(true).unwrap();
        assert_eq!(pending.status.available.as_ref().unwrap().version, "0.3.0");
        pending.failed_check("固定网络失败".into(), true);
        assert_eq!(pending.status.phase, "available");
        assert_eq!(pending.status.available.as_ref().unwrap().version, "0.3.0");
        assert_eq!(pending.status.error.as_deref(), Some("固定网络失败"));
    }

    #[test]
    fn stable_channel_excludes_prereleases_and_ignoring_requires_exact_current_version() {
        use serde_json::json;
        assert!(is_stable_release("0.3.0", &json!({})));
        assert!(is_stable_release("12.23.456", &json!({"prerelease":false})));
        for version in [
            "v0.3.0",
            "0.3.0-beta.1",
            "0.3.0-rc.1",
            "0.3.0+build",
            "01.2.3",
            "1.2",
            "1.2.3.4",
            "+1.2.3",
            "18446744073709551616.0.0",
        ] {
            assert!(!is_stable_release(version, &json!({})), "{version}");
        }
        for flag in [json!(true), json!("false"), json!(null)] {
            assert!(!is_stable_release("0.3.0", &json!({"prerelease":flag})));
        }
        let updates = Updates(Mutex::new(downloading()));
        let mut saved = false;
        assert!(updates
            .for_available_version("0.4.0", || {
                saved = true;
                Ok(())
            })
            .is_err());
        assert!(!saved);
        updates
            .for_available_version("0.3.0", || {
                saved = true;
                Ok(())
            })
            .unwrap();
        assert!(saved);
        assert_eq!(
            updates.status().unwrap().available.unwrap().version,
            "0.3.0",
            "ignoring does not discard the manually downloadable candidate"
        );
    }

    #[test]
    fn mismatched_or_empty_download_cannot_become_ready() {
        let mut pending = downloading();
        assert!(pending.downloaded("0.4.0", vec![1]).is_err());
        assert!(pending.downloaded("0.3.0", Vec::new()).is_err());
        assert!(pending.downloaded_package.is_none());
        assert_eq!(pending.status.phase, "downloading");

        pending.status.phase = "ready";
        assert!(pending.take_downloaded("0.3.0").is_err());
    }
}
