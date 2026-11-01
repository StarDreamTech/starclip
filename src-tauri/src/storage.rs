//! Data-directory switching is committed last. The old directory is never removed.
use crate::store::{Result, Store, SCHEMA_VERSION};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, DirBuilder, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const DATABASE: &str = "starclip.sqlite3";
const SELECTION: &str = "handwriting-app.json";
const LOCATOR: &str = "data-location.json";
const MANAGED_DIRECTORIES: [&str; 2] = ["renderers", "fonts"];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DirectoryMode {
    Move,
    Use,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataLocation {
    pub current_path: String,
    pub default_path: String,
    pub config_path: String,
    pub pending_path: Option<String>,
    pub total_bytes: u64,
    pub database_bytes: u64,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataDirectoryPlan {
    pub mode: DirectoryMode,
    pub current_path: String,
    pub target_path: String,
    pub total_bytes: u64,
    pub warnings: Vec<String>,
}
#[derive(Deserialize, Serialize)]
struct Locator {
    version: u32,
    path: PathBuf,
}

// A single mutex around this object is also the write gate in desktop/.
// Both directories remain locked between a successful switch and process restart.
pub struct Storage {
    pub current: PathBuf,
    default: PathBuf,
    legacy_config: PathBuf,
    pending: Option<PathBuf>,
    update_pending: bool,
    _active_lock: File,
    pending_lock: Option<File>,
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
fn io_error(error: std::io::Error) -> String {
    format!("数据目录操作失败：{error}")
}
fn nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
fn create_private_directory(path: &Path) -> Result<()> {
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(io_error)
}
fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(io_error)
}
fn lock_directory(path: &Path) -> Result<File> {
    let lock_path = path.join(".starclip.lock");
    if fs::symlink_metadata(&lock_path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("数据目录锁文件不能是符号链接。".into());
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(lock_path).map_err(io_error)?;
    file.try_lock().map_err(|_| {
        "该数据目录正被另一个星笺实例使用，或所在磁盘不支持文件锁。请关闭其他实例后重试。"
    })?;
    Ok(file)
}
fn read_small(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io_error)?
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > 16 * 1024 {
        return Err("数据目录配置过大，请检查配置文件。".into());
    }
    Ok(bytes)
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or("无效的配置文件路径。")?;
    let temporary = parent.join(format!(
        ".starclip-location-{}-{}.tmp",
        std::process::id(),
        nonce()
    ));
    let result = (|| {
        let mut file = private_file(&temporary)?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(io_error)?;
        drop(file);
        fs::rename(&temporary, path).map_err(io_error)?;
        // Once rename succeeds the switch is committed; a directory fsync failure must
        // not resume writing to the old database with a locator already pointing elsewhere.
        #[cfg(unix)]
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
fn write_locator(default: &Path, path: &Path) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(&Locator {
        version: 1,
        path: path.to_owned(),
    })
    .map_err(|error| error.to_string())?;
    atomic_write(&default.join(LOCATOR), &bytes)
}
fn check_directory(path: &Path) -> Result<PathBuf> {
    let path = path.canonicalize().map_err(|_| {
        format!(
            "数据目录不存在或无法访问：{}。未创建新数据库。",
            display(path)
        )
    })?;
    if !path.is_dir() {
        return Err("请选择包含 starclip.sqlite3 的数据目录。".into());
    }
    if fs::symlink_metadata(path.join("snapshot.json")).is_ok() {
        return Err("这是只读恢复点，请使用恢复功能复制到新的数据目录。".into());
    }
    let database = path.join(DATABASE);
    if fs::symlink_metadata(&database).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("数据库不能是指向其他位置的符号链接。".into());
    }
    Store::validate_database(&database)?;
    Ok(path)
}

fn renderer_components(path: &str) -> Result<Vec<&str>> {
    // Accept old Windows selections on every platform, but validate before joining:
    // Path::components on Unix does not recognize Windows separators or drive paths.
    let parts: Vec<_> = path.split(['/', '\\']).collect();
    if parts.first() != Some(&"renderers")
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || part.ends_with('.')
                || part.ends_with(' ')
                || part.contains(':')
                || part.chars().any(char::is_control)
        })
    {
        return Err("相对渲染器路径必须位于数据目录的 renderers 文件夹。".into());
    }
    Ok(parts)
}

/// Encode native managed paths with a platform-independent separator.
pub(crate) fn portable_renderer_path(relative: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for part in relative.components() {
        let Component::Normal(part) = part else {
            return Err("受管渲染器路径不能包含根目录或上级目录。".into());
        };
        let part = part
            .to_str()
            .ok_or("受管渲染器文件名不是有效的 Unicode。")?;
        // A literal Unix backslash would become a directory separator on Windows.
        if part.contains('\\') {
            return Err("受管渲染器文件名不能包含反斜杠。".into());
        }
        parts.push(part);
    }
    let path = parts.join("/");
    renderer_components(&path)?;
    Ok(path)
}

/// Resolve portable and legacy Windows managed paths without trusting `..` paths.
pub fn renderer_path(directory: &Path, selected: &Path) -> Result<PathBuf> {
    // Never replace a user's absolute external selection with a similarly named
    // program inside this directory. A missing external program must be reselected.
    if !selected.is_absolute() {
        let selected = selected.to_str().ok_or("渲染器路径不是有效的 Unicode。")?;
        let relative: PathBuf = renderer_components(selected)?.into_iter().collect();
        return Ok(directory.join(relative));
    }
    Ok(selected.to_owned())
}
fn selection_data(directory: &Path, source: &Path) -> Result<Option<(Vec<u8>, bool)>> {
    let path = source.join(SELECTION);
    if !path.exists() {
        return Ok(None);
    }
    if fs::symlink_metadata(&path)
        .map_err(io_error)?
        .file_type()
        .is_symlink()
    {
        return Err("手写应用配置不能是符号链接。".into());
    }
    let mut json: serde_json::Value = serde_json::from_slice(&read_small(&path)?)
        .map_err(|_| "手写应用配置损坏，请重新选择渲染器。")?;
    let selected = PathBuf::from(
        json.get("path")
            .and_then(|value| value.as_str())
            .ok_or("手写应用配置缺少 path。")?,
    );
    let resolved = renderer_path(directory, &selected)?;
    let relative = resolved
        .strip_prefix(directory)
        .ok()
        .filter(|relative| relative.starts_with("renderers"));
    let external = relative.is_none();
    if let Some(relative) = relative {
        json["path"] = portable_renderer_path(relative)?.into();
    } else if resolved.starts_with(directory) {
        return Err("数据目录中的手写渲染器需放在 renderers 子目录后再迁移。原数据未改变。".into());
    }
    Ok(Some((
        serde_json::to_vec_pretty(&json).map_err(|error| error.to_string())?,
        external,
    )))
}
fn file_bytes(path: &Path) -> Result<u64> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(io_error(error)),
    };
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut bytes = 0u64;
    for entry in fs::read_dir(path).map_err(io_error)? {
        bytes = bytes.saturating_add(file_bytes(&entry.map_err(io_error)?.path())?);
    }
    Ok(bytes)
}
fn managed_bytes(directory: &Path, selection: &Path) -> Result<(u64, u64)> {
    let mut database = 0u64;
    for name in [DATABASE, "starclip.sqlite3-wal", "starclip.sqlite3-shm"] {
        database = database.saturating_add(file_bytes(&directory.join(name))?);
    }
    let mut total = database.saturating_add(file_bytes(&selection.join(SELECTION))?);
    for name in MANAGED_DIRECTORIES {
        total = total.saturating_add(file_bytes(&directory.join(name))?);
    }
    Ok((total, database))
}
fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source).map_err(io_error)?;
    if metadata.file_type().is_symlink() {
        return Err("受管资源含符号链接，未迁移。请使用普通文件形式的字体或渲染器目录。".into());
    }
    if metadata.is_dir() {
        create_private_directory(target)?;
        for entry in fs::read_dir(source).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
        sync_directory(target)?;
    } else if metadata.is_file() {
        let mut destination = private_file(target)?;
        let mut input = File::open(source).map_err(io_error)?;
        std::io::copy(&mut input, &mut destination).map_err(io_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let executable = metadata.permissions().mode() & 0o111 != 0;
            destination
                .set_permissions(fs::Permissions::from_mode(if executable {
                    0o700
                } else {
                    0o600
                }))
                .map_err(io_error)?;
        }
        destination.sync_all().map_err(io_error)?;
    } else {
        return Err("受管资源只能包含普通文件与目录。".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryPoint {
    pub id: String,
    pub created_at: u64,
    pub app_version: String,
    pub schema_version: u32,
    pub reason: String,
    pub total_bytes: u64,
    pub path: String,
    pub warnings: Vec<String>,
    pub compatible: bool,
}
#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct RecoveryFile {
    bytes: u64,
    sha256: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryManifest {
    format_version: u32,
    id: String,
    created_at: u64,
    app_version: String,
    schema_version: u32,
    reason: String,
    total_bytes: u64,
    warnings: Vec<String>,
    files: BTreeMap<String, RecoveryFile>,
}
impl RecoveryManifest {
    fn point(&self, directory: &Path) -> RecoveryPoint {
        RecoveryPoint {
            id: self.id.clone(),
            created_at: self.created_at,
            app_version: self.app_version.clone(),
            schema_version: self.schema_version,
            reason: self.reason.clone(),
            total_bytes: self.total_bytes,
            path: display(directory),
            warnings: self.warnings.clone(),
            compatible: self.format_version == 1 && self.schema_version <= SCHEMA_VERSION,
        }
    }
}

fn managed_directory(path: &Path) -> Result<PathBuf> {
    if !path.exists() {
        create_private_directory(path)?;
    }
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("恢复目录必须是普通文件夹，不能使用符号链接。".into());
    }
    path.canonicalize().map_err(io_error)
}
fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(io_error)?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
fn recovery_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 100
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("恢复点编号无效。".into());
    }
    Ok(())
}
fn collect_recovery_files(directory: &Path) -> Result<BTreeMap<String, RecoveryFile>> {
    fn visit(
        root: &Path,
        directory: &Path,
        files: &mut BTreeMap<String, RecoveryFile>,
    ) -> Result<()> {
        for entry in fs::read_dir(directory).map_err(io_error)? {
            let path = entry.map_err(io_error)?.path();
            let relative = path.strip_prefix(root).map_err(|_| "恢复点路径无效。")?;
            if relative == Path::new("snapshot.json") {
                continue;
            }
            let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
            if metadata.file_type().is_symlink() {
                return Err("恢复点含符号链接，已拒绝恢复。".into());
            }
            let managed = MANAGED_DIRECTORIES
                .iter()
                .any(|name| relative.starts_with(name));
            if relative != Path::new(DATABASE) && relative != Path::new(SELECTION) && !managed {
                return Err("恢复点含未识别的文件，已拒绝恢复。".into());
            }
            if metadata.is_dir() {
                if !managed {
                    return Err("恢复点文件类型无效。".into());
                }
                visit(root, &path, files)?;
            } else if metadata.is_file() {
                if files.len() >= 10_000 {
                    return Err("恢复点超过 10000 个托管文件，未继续操作。".into());
                }
                let mut input = File::open(&path).map_err(io_error)?;
                let mut hash = Sha256::new();
                let mut buffer = [0u8; 64 * 1024];
                loop {
                    let length = input.read(&mut buffer).map_err(io_error)?;
                    if length == 0 {
                        break;
                    }
                    hash.update(&buffer[..length]);
                }
                let key = relative
                    .to_str()
                    .ok_or("恢复点文件名不是有效的 Unicode。")?
                    .replace('\\', "/");
                files.insert(
                    key,
                    RecoveryFile {
                        bytes: metadata.len(),
                        sha256: format!("{:x}", hash.finalize()),
                    },
                );
            } else {
                return Err("恢复点含非普通文件。".into());
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    visit(directory, directory, &mut files)?;
    if !files.contains_key(DATABASE) {
        return Err("恢复点缺少数据库。".into());
    }
    Ok(files)
}
fn read_manifest(root: &Path, id: &str) -> Result<(PathBuf, RecoveryManifest)> {
    recovery_id(id)?;
    let root_metadata = fs::symlink_metadata(root).map_err(io_error)?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err("恢复目录无效。".into());
    }
    let directory = root.join(id);
    let metadata = fs::symlink_metadata(&directory).map_err(io_error)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("恢复点不是普通目录。".into());
    }
    let path = directory.join("snapshot.json");
    let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 2 * 1024 * 1024
    {
        return Err("恢复点清单无效或超过 2 MiB。".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io_error)?
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("恢复点清单超过 2 MiB。".into());
    }
    let manifest: RecoveryManifest =
        serde_json::from_slice(&bytes).map_err(|_| "恢复点清单损坏。")?;
    if manifest.id != id {
        return Err("恢复点编号与清单不符。".into());
    }
    Ok((directory, manifest))
}
fn verified_recovery(root: &Path, id: &str) -> Result<(PathBuf, RecoveryManifest)> {
    let (directory, manifest) = read_manifest(root, id)?;
    if manifest.format_version != 1 || manifest.schema_version > SCHEMA_VERSION {
        return Err("此恢复点需要较新版本的星笺，未修改任何数据。".into());
    }
    let actual = collect_recovery_files(&directory)?;
    if actual != manifest.files
        || actual.values().map(|f| f.bytes).sum::<u64>() != manifest.total_bytes
    {
        return Err("恢复点校验失败，文件可能损坏或被更改；原数据未修改。".into());
    }
    Store::validate_database(&directory.join(DATABASE))?;
    let conn = rusqlite::Connection::open_with_flags(
        directory.join(DATABASE),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| e.to_string())?;
    if Store::check_schema_version(&conn)? != manifest.schema_version {
        return Err("恢复点结构版本与清单不符。".into());
    }
    selection_data(&directory, &directory)?;
    Ok((directory, manifest))
}

/// Caller holds the storage write gate. Only managed payloads enter the snapshot;
/// the recovery archive itself is never recursively copied.
pub(crate) fn create_recovery_snapshot(
    root: &Path,
    current: &Path,
    selection: &Path,
    schema_version: u32,
    reason: &str,
    snapshot: impl FnOnce(&Path) -> Result<()>,
) -> Result<RecoveryPoint> {
    if ![
        "manual",
        "before-update",
        "before-restore",
        "before-migration",
    ]
    .contains(&reason)
    {
        return Err("未知的恢复点用途。".into());
    }
    let root = managed_directory(root).map_err(|error| format!("准备恢复点目录失败：{error}"))?;
    let id = format!("{}-{}", nonce(), std::process::id());
    let temporary = root.join(format!(".pending-{id}"));
    create_private_directory(&temporary)
        .map_err(|error| format!("创建恢复点临时目录失败：{error}"))?;
    let result = (|| -> Result<RecoveryPoint> {
        let database = temporary.join(DATABASE);
        drop(private_file(&database).map_err(|error| format!("创建恢复点数据库失败：{error}"))?);
        snapshot(&database).map_err(|error| format!("复制恢复点数据库失败：{error}"))?;
        OpenOptions::new()
            .write(true)
            .open(&database)
            .and_then(|file| file.sync_all())
            .map_err(|error| format!("恢复点数据库刷盘失败：{error}"))?;
        let mut warnings = vec![];
        if let Some((bytes, external)) = selection_data(current, selection)? {
            let mut file = private_file(&temporary.join(SELECTION))?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(io_error)?;
            if external {
                warnings.push("此恢复点仅保留外部手写渲染器的路径设置，未复制外部程序。".into());
            }
        }
        for name in MANAGED_DIRECTORIES {
            let source = current.join(name);
            if fs::symlink_metadata(&source).is_ok() {
                copy_tree(&source, &temporary.join(name))?;
            }
        }
        let files = collect_recovery_files(&temporary)
            .map_err(|error| format!("校验恢复点文件失败：{error}"))?;
        let manifest = RecoveryManifest {
            format_version: 1,
            id: id.clone(),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            app_version: env!("CARGO_PKG_VERSION").into(),
            schema_version,
            reason: reason.into(),
            total_bytes: files.values().map(|file| file.bytes).sum(),
            warnings,
            files,
        };
        let mut file = private_file(&temporary.join("snapshot.json"))
            .map_err(|error| format!("创建恢复点清单失败：{error}"))?;
        let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("恢复点清单超过 2 MiB，未继续操作。".into());
        }
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("写入恢复点清单失败：{error}"))?;
        // Windows cannot rename a directory while one of its children is open.
        drop(file);
        sync_directory(&temporary).map_err(|error| format!("同步恢复点临时目录失败：{error}"))?;
        let destination = root.join(&id);
        fs::rename(&temporary, &destination)
            .map_err(|error| format!("提交恢复点目录失败：{error}"))?;
        sync_directory(&root).map_err(|error| format!("同步恢复点目录失败：{error}"))?;
        Ok(manifest.point(&destination))
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

fn prepare_recovery(default: &Path, id: &str) -> Result<(PathBuf, RecoveryPoint)> {
    let (snapshot, manifest) = verified_recovery(&default.join("recovery"), id)?;
    let recovered = managed_directory(&default.join("recovered"))?;
    let target = recovered.join(format!("Starclip-{}", nonce()));
    create_private_directory(&target)?;
    let result = (|| -> Result<()> {
        for name in [DATABASE, SELECTION].into_iter().chain(MANAGED_DIRECTORIES) {
            let source = snapshot.join(name);
            if source.exists() {
                copy_tree(&source, &target.join(name))?;
            }
        }
        // Validate the copy too, not just its source, before switching the locator.
        if collect_recovery_files(&target)? != manifest.files {
            return Err("恢复副本校验失败，未切换数据。".into());
        }
        check_directory(&target)?;
        sync_directory(&target)?;
        sync_directory(&recovered)?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&target);
        return Err(error);
    }
    Ok((target, manifest.point(&snapshot)))
}

impl Storage {
    pub fn open(default: &Path, legacy_config: &Path) -> Result<Self> {
        if !default.exists() {
            fs::create_dir_all(default).map_err(io_error)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(default, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
        }
        let default = default.canonicalize().map_err(io_error)?;
        let locator_path = default.join(LOCATOR);
        let current = if fs::symlink_metadata(&locator_path).is_ok() {
            let locator: Locator =
                serde_json::from_slice(&read_small(&locator_path)?).map_err(|_| {
                    format!(
                        "数据位置配置无效：{}。请恢复该配置或选择已有目录。",
                        display(&locator_path)
                    )
                })?;
            if locator.version != 1 || !locator.path.is_absolute() {
                return Err(format!(
                    "不支持的数据位置配置：{}。",
                    display(&locator_path)
                ));
            }
            check_directory(&locator.path).map_err(|error| format!("{error}\n定位配置：{}\n旧数据目录未被删除；可重新连接磁盘或选择保留的已有数据目录。", display(&locator_path)))?
        } else {
            if default.join(DATABASE).exists() {
                check_directory(&default)?;
            }
            default.clone()
        };
        let active_lock = lock_directory(&current)?;
        Ok(Self {
            current,
            default,
            legacy_config: legacy_config.to_owned(),
            pending: None,
            update_pending: false,
            _active_lock: active_lock,
            pending_lock: None,
        })
    }

    pub fn default_directory(&self) -> &Path {
        &self.default
    }
    pub fn recovery_directory(&self) -> PathBuf {
        self.default.join("recovery")
    }
    pub fn open_store(&self) -> Result<Store> {
        Store::open_with_recovery(
            &self.current.join(DATABASE),
            &self.handwriting_directory(),
            &self.recovery_directory(),
        )
    }
    pub fn freeze_for_update(&mut self) -> Result<()> {
        self.writable()?;
        self.update_pending = true;
        Ok(())
    }
    pub fn cancel_update_freeze(&mut self) {
        self.update_pending = false;
    }
    pub fn writable(&self) -> Result<()> {
        if self.update_pending {
            return Err("正在安装更新，记录与修改已暂停。更新完成后请重启星笺。".into());
        }
        if self.pending.is_some() {
            return Err("数据目录已切换，记录与修改已暂停。请立即重启星笺以使用新目录。".into());
        }
        Ok(())
    }
    pub fn restart_pending(&self) -> bool {
        self.pending.is_some() || self.update_pending
    }
    pub fn pending_path(&self) -> Option<String> {
        self.pending.as_deref().map(display)
    }
    pub fn release_for_restart(&mut self) -> Result<()> {
        if !self.restart_pending() {
            return Err("当前没有等待重启的数据目录切换。".into());
        }
        // Tauri spawns the replacement process before exiting. Release its target
        // lock first, while pending continues to forbid writes in this process.
        self.pending_lock.take();
        self._active_lock.unlock().map_err(io_error)?;
        Ok(())
    }
    /// Legacy external-renderer selection only. Built-in fonts live under current/fonts.
    pub fn handwriting_directory(&self) -> PathBuf {
        if self.current == self.default
            && !self.current.join(SELECTION).exists()
            && self.legacy_config.join(SELECTION).exists()
        {
            self.legacy_config.clone()
        } else {
            self.current.clone()
        }
    }
    pub fn info(&self) -> Result<DataLocation> {
        let selection = self.handwriting_directory();
        let (total_bytes, database_bytes) = managed_bytes(&self.current, &selection)?;
        let warnings = match selection_data(&self.current, &selection) {
            Ok(Some((_, true))) => vec!["当前手写渲染器位于数据目录外，仅迁移其路径设置，不复制外部程序；在其他电脑上可能需要重新选择渲染器。".into()],
            Err(error) => vec![error],
            _ => vec![],
        };
        Ok(DataLocation {
            current_path: display(&self.current),
            default_path: display(&self.default),
            config_path: display(&self.default.join(LOCATOR)),
            pending_path: self.pending.as_deref().map(display),
            total_bytes,
            database_bytes,
            warnings,
        })
    }
    pub fn plan(&self, mode: DirectoryMode, selected: &Path) -> Result<DataDirectoryPlan> {
        self.writable()?;
        let info = self.info()?;
        let target = match mode {
            DirectoryMode::Move => {
                let parent = selected.canonicalize().map_err(io_error)?;
                if !parent.is_dir() {
                    return Err("请选择用于存放数据的文件夹。".into());
                }
                parent.join(format!("Starclip-{}", nonce()))
            }
            DirectoryMode::Use => check_directory(selected)?,
        };
        if target == self.current {
            return Err("这已经是当前数据目录。".into());
        }
        if mode == DirectoryMode::Move && target.starts_with(&self.current) {
            return Err("目标不能位于当前数据目录内部，请选择其他位置。".into());
        }
        let mut warnings = if mode == DirectoryMode::Use {
            vec!["采用所选目录的历史和偏好设置，不合并当前数据。当前目录保留；在另一台电脑同时打开同一目录可能导致冲突，请先退出另一台电脑上的星笺。".into()]
        } else {
            info.warnings
        };
        let total_bytes = if mode == DirectoryMode::Use {
            match selection_data(&target, &target) {
                Ok(Some((_, true))) => warnings.push(
                    "目标目录记录了外部手写渲染器路径；在本机不可用时需要重新选择渲染器。".into(),
                ),
                Err(error) => warnings.push(error),
                _ => {}
            }
            managed_bytes(&target, &target)?.0
        } else {
            info.total_bytes
        };
        Ok(DataDirectoryPlan {
            mode,
            current_path: info.current_path,
            target_path: display(&target),
            total_bytes,
            warnings,
        })
    }

    /// Caller must hold the desktop write gate and Store lock until this returns.
    pub fn apply(&mut self, store: &Store, mode: DirectoryMode, target: &Path) -> Result<()> {
        self.writable()?;
        match mode {
            DirectoryMode::Use => {
                let target = check_directory(target)?;
                if target == self.current {
                    return Err("这已经是当前数据目录。".into());
                }
                let target_lock = lock_directory(&target)?;
                check_directory(&target)?;
                write_locator(&self.default, &target)?;
                self.pending = Some(target);
                self.pending_lock = Some(target_lock);
            }
            DirectoryMode::Move => {
                if !target.is_absolute() || target.file_name().is_none() {
                    return Err("目标必须是新的绝对目录路径。".into());
                }
                let parent = target
                    .parent()
                    .ok_or("目标目录无效。")?
                    .canonicalize()
                    .map_err(io_error)?;
                let target = parent.join(target.file_name().unwrap());
                if target.starts_with(&self.current) {
                    return Err("目标不能位于当前数据目录内部，请选择其他位置。".into());
                }
                create_private_directory(&target).map_err(|error| {
                    format!("目标必须是不存在的新子目录，不会覆盖任何已有目录：{error}")
                })?;
                let result = (|| -> Result<File> {
                    let target_lock = lock_directory(&target)?;
                    let db = target.join(DATABASE);
                    drop(private_file(&db)?);
                    store.snapshot(&db)?;
                    OpenOptions::new()
                        .write(true)
                        .open(&db)
                        .and_then(|file| file.sync_all())
                        .map_err(io_error)?;
                    for name in MANAGED_DIRECTORIES {
                        let source = self.current.join(name);
                        if fs::symlink_metadata(&source).is_ok() {
                            copy_tree(&source, &target.join(name))?;
                        }
                    }
                    if let Some((bytes, _)) =
                        selection_data(&self.current, &self.handwriting_directory())?
                    {
                        let mut file = private_file(&target.join(SELECTION))?;
                        file.write_all(&bytes)
                            .and_then(|_| file.sync_all())
                            .map_err(io_error)?;
                    }
                    check_directory(&target)?;
                    write_locator(&self.default, &target)?;
                    Ok(target_lock)
                })();
                match result {
                    Ok(target_lock) => {
                        self.pending = Some(target);
                        self.pending_lock = Some(target_lock);
                    }
                    Err(error) => {
                        // Only remove the exclusively created destination owned by this attempt.
                        let _ = fs::remove_dir_all(&target);
                        return Err(error);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn create_recovery_point(&self, store: &Store, reason: &str) -> Result<RecoveryPoint> {
        self.writable()?;
        create_recovery_snapshot(
            &self.recovery_directory(),
            &self.current,
            &self.handwriting_directory(),
            store.schema_version()?,
            reason,
            |target| store.snapshot(target),
        )
    }
    pub fn recovery_points(&self) -> Result<Vec<RecoveryPoint>> {
        Self::recovery_points_at_startup(&self.default)
    }
    pub fn recovery_points_at_startup(default: &Path) -> Result<Vec<RecoveryPoint>> {
        let root = default.join("recovery");
        if !root.exists() {
            return Ok(vec![]);
        }
        let metadata = fs::symlink_metadata(&root).map_err(io_error)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("恢复目录无效。".into());
        }
        let mut points = vec![];
        for entry in fs::read_dir(&root).map_err(io_error)? {
            let name = entry
                .map_err(io_error)?
                .file_name()
                .to_string_lossy()
                .into_owned();
            if name.starts_with('.') {
                continue;
            }
            match read_manifest(&root, &name) {
                Ok((directory, manifest)) => points.push(manifest.point(&directory)),
                Err(error) => points.push(RecoveryPoint {
                    id: name.clone(),
                    created_at: 0,
                    app_version: "unknown".into(),
                    schema_version: 0,
                    reason: "manual".into(),
                    total_bytes: 0,
                    path: display(&root.join(&name)),
                    warnings: vec![error],
                    compatible: false,
                }),
            }
        }
        points.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(points)
    }
    pub fn restore_recovery_point(&mut self, store: &Store, id: &str) -> Result<RecoveryPoint> {
        self.writable()?;
        // Reject damaged/incompatible input before making any current-data snapshot.
        verified_recovery(&self.recovery_directory(), id)?;
        self.create_recovery_point(store, "before-restore")?;
        let (target, point) = prepare_recovery(&self.default, id)?;
        if let Err(error) = self.apply(store, DirectoryMode::Use, &target) {
            let _ = fs::remove_dir_all(&target);
            return Err(error);
        }
        Ok(point)
    }
    /// Used before Store can open, e.g. after a failed migration. Restore to a
    /// fresh directory; the snapshot and original data are both retained.
    pub fn restore_recovery_point_at_startup(default: &Path, id: &str) -> Result<PathBuf> {
        let (target, _) = prepare_recovery(default, id)?;
        if let Err(error) = Self::recover(default, &target) {
            let _ = fs::remove_dir_all(&target);
            return Err(error);
        }
        Ok(target)
    }

    /// Startup recovery never initializes a missing selected database.
    pub fn recover(default: &Path, selected: &Path) -> Result<()> {
        let selected = check_directory(selected)?;
        let _lock = lock_directory(&selected)?;
        write_locator(default, &selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "starclip-storage-test-{}-{}-{}",
                std::process::id(),
                nonce(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            create_private_directory(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn migration_snapshots_wal_preferences_and_portable_renderer_keeps_old_data() {
        let scratch = Scratch::new();
        let source = scratch.0.join("source");
        let mut storage = Storage::open(&source, &source).unwrap();
        let store = Store::open(&source.join(DATABASE)).unwrap();
        let wal = Connection::open(source.join(DATABASE)).unwrap();
        wal.pragma_update(None, "journal_mode", "WAL").unwrap();
        wal.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        let clip = store.add("迁移测试：星河入梦", "原创数据", None).unwrap();
        store.update(clip.id, "保留标题", None, true).unwrap();
        let mut settings = store.settings().unwrap();
        settings.theme = "forest".into();
        settings.show_startup = false;
        store.save_settings(&settings).unwrap();
        assert!(
            source
                .join("starclip.sqlite3-wal")
                .metadata()
                .unwrap()
                .len()
                > 0
        );
        create_private_directory(&source.join("renderers")).unwrap();
        fs::write(
            source.join("renderers/xingjian"),
            b"synthetic renderer, never executed",
        )
        .unwrap();
        fs::write(
            source.join(SELECTION),
            serde_json::json!({"path":source.join("renderers/xingjian")}).to_string(),
        )
        .unwrap();
        let target = scratch.0.join("migrated");
        storage.apply(&store, DirectoryMode::Move, &target).unwrap();
        assert!(storage.writable().is_err());
        assert_eq!(
            storage.info().unwrap().pending_path.as_deref(),
            Some(target.to_str().unwrap())
        );
        assert!(source.join(DATABASE).is_file());
        assert!(source.join(SELECTION).is_file());
        Store::validate_database(&target.join(DATABASE)).unwrap();
        let copied = Store::open(&target.join(DATABASE)).unwrap();
        assert_eq!(copied.clip(clip.id).unwrap().title, "保留标题");
        assert!(copied.clip(clip.id).unwrap().pinned);
        assert_eq!(copied.settings().unwrap(), settings);
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join(SELECTION)).unwrap()).unwrap();
        assert_eq!(json["path"], "renderers/xingjian");
        assert_eq!(
            renderer_path(&target, Path::new(json["path"].as_str().unwrap())).unwrap(),
            target.join("renderers/xingjian")
        );
        assert!(
            Storage::open(&source, &source).is_err(),
            "pending target stays locked"
        );
        storage.release_for_restart().unwrap();
        assert!(
            storage.writable().is_err(),
            "restart handoff never resumes old writes"
        );
        let reopened = Storage::open(&source, &source).unwrap();
        assert_eq!(reopened.current, target);
    }

    #[test]
    fn downloaded_fonts_follow_current_directory_migration_and_recovery() {
        let scratch = Scratch::new();
        let source = scratch.0.join("source");
        let legacy = scratch.0.join("legacy");
        create_private_directory(&legacy).unwrap();
        fs::write(legacy.join(SELECTION), r#"{"path":"renderers/local"}"#).unwrap();
        let mut storage = Storage::open(&source, &legacy).unwrap();
        let store = storage.open_store().unwrap();
        assert_eq!(storage.handwriting_directory(), legacy);
        let before = storage.info().unwrap().total_bytes;
        create_private_directory(&source.join("fonts")).unwrap();
        let font = b"original synthetic font fixture, never parsed as TTF";
        fs::write(source.join("fonts/sample.ttf"), font).unwrap();
        assert_eq!(
            storage.info().unwrap().total_bytes,
            before + font.len() as u64
        );
        let target = scratch.0.join("migrated");
        let plan = storage.plan(DirectoryMode::Move, &scratch.0).unwrap();
        assert_eq!(plan.total_bytes, storage.info().unwrap().total_bytes);
        storage.apply(&store, DirectoryMode::Move, &target).unwrap();
        assert_eq!(fs::read(target.join("fonts/sample.ttf")).unwrap(), font);
        assert_eq!(fs::read(source.join("fonts/sample.ttf")).unwrap(), font);
        assert!(!legacy.join("fonts").exists());
        assert!(
            target.join(SELECTION).is_file(),
            "legacy config still migrates"
        );
        storage.release_for_restart().unwrap();
        drop(storage);
        drop(store);

        let mut storage = Storage::open(&source, &legacy).unwrap();
        let store = storage.open_store().unwrap();
        assert_eq!(storage.current, target);
        let point = storage.create_recovery_point(&store, "manual").unwrap();
        let (snapshot, manifest) =
            verified_recovery(&storage.recovery_directory(), &point.id).unwrap();
        assert_eq!(fs::read(snapshot.join("fonts/sample.ttf")).unwrap(), font);
        assert_eq!(manifest.files["fonts/sample.ttf"].bytes, font.len() as u64);
        fs::write(target.join("fonts/sample.ttf"), b"changed fixed fixture").unwrap();
        storage.restore_recovery_point(&store, &point.id).unwrap();
        let restored = PathBuf::from(storage.pending_path().unwrap());
        assert_ne!(restored, target);
        assert_eq!(fs::read(restored.join("fonts/sample.ttf")).unwrap(), font);
        assert_eq!(
            fs::read(target.join("fonts/sample.ttf")).unwrap(),
            b"changed fixed fixture"
        );
        assert_eq!(fs::read(source.join("fonts/sample.ttf")).unwrap(), font);
    }

    #[test]
    fn existing_target_and_failed_commit_do_not_change_locator_or_old_database() {
        let scratch = Scratch::new();
        let source = scratch.0.join("source");
        let mut storage = Storage::open(&source, &source).unwrap();
        let store = Store::open(&source.join(DATABASE)).unwrap();
        store.add("失败恢复测试", "", None).unwrap();
        let target = scratch.0.join("existing");
        create_private_directory(&target).unwrap();
        fs::write(target.join("keep.txt"), "原文件").unwrap();
        assert!(storage.apply(&store, DirectoryMode::Move, &target).is_err());
        assert_eq!(
            fs::read_to_string(target.join("keep.txt")).unwrap(),
            "原文件"
        );
        assert!(storage.plan(DirectoryMode::Move, &source).is_err());
        assert!(storage
            .apply(&store, DirectoryMode::Move, &source.join("nested"))
            .is_err());
        assert!(!source.join("nested").exists());
        create_private_directory(&source.join(LOCATOR)).unwrap(); // force atomic rename failure
        let failed = scratch.0.join("failed");
        assert!(storage.apply(&store, DirectoryMode::Move, &failed).is_err());
        assert!(!failed.exists());
        assert!(source.join(LOCATOR).is_dir());
        assert!(storage.writable().is_ok());
        assert_eq!(store.clips().unwrap().len(), 1);
    }

    #[test]
    fn missing_or_corrupt_selected_database_is_never_initialized() {
        let scratch = Scratch::new();
        let source = scratch.0.join("source");
        let storage = Storage::open(&source, &source).unwrap();
        let _store = Store::open(&source.join(DATABASE)).unwrap();
        let missing = scratch.0.join("disconnected-drive");
        write_locator(&source, &missing).unwrap();
        drop(storage);
        assert!(Storage::open(&source, &source).is_err());
        assert!(!missing.exists());
        let invalid = scratch.0.join("invalid");
        create_private_directory(&invalid).unwrap();
        assert!(Storage::recover(&source, &invalid).is_err());
        assert!(!invalid.join(DATABASE).exists());
        fs::write(invalid.join(DATABASE), "不是数据库").unwrap();
        assert!(Storage::recover(&source, &invalid).is_err());
        assert_eq!(
            fs::read_to_string(invalid.join(DATABASE)).unwrap(),
            "不是数据库"
        );
        let locator: Locator =
            serde_json::from_slice(&fs::read(source.join(LOCATOR)).unwrap()).unwrap();
        assert_eq!(locator.path, missing);
    }

    #[test]
    fn adoption_is_read_only_until_confirmed_and_respects_directory_lock() {
        let scratch = Scratch::new();
        let source = scratch.0.join("source");
        let target = scratch.0.join("other");
        let mut storage = Storage::open(&source, &source).unwrap();
        let store = Store::open(&source.join(DATABASE)).unwrap();
        let other = Storage::open(&target, &target).unwrap();
        let existing = Store::open(&target.join(DATABASE)).unwrap();
        existing.add("另一目录原创片段", "", None).unwrap();
        let before = fs::read(target.join(DATABASE)).unwrap();
        storage.plan(DirectoryMode::Use, &target).unwrap();
        assert_eq!(before, fs::read(target.join(DATABASE)).unwrap());
        assert!(!source.join(LOCATOR).exists());
        assert!(storage.apply(&store, DirectoryMode::Use, &target).is_err());
        drop(other);
        storage.apply(&store, DirectoryMode::Use, &target).unwrap();
        assert!(source.join(DATABASE).exists());
        assert_eq!(before, fs::read(target.join(DATABASE)).unwrap());
    }

    #[test]
    fn validating_legacy_directory_does_not_upgrade_schema_or_prune_history() {
        let scratch = Scratch::new();
        let database = scratch.0.join(DATABASE);
        let connection = Connection::open(&database).unwrap();
        connection.execute_batch("CREATE TABLE groups(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, color TEXT NOT NULL);
            CREATE TABLE clips(id INTEGER PRIMARY KEY, content TEXT NOT NULL UNIQUE, title TEXT NOT NULL DEFAULT '',kind TEXT NOT NULL,pinned INTEGER NOT NULL DEFAULT 0,group_id INTEGER REFERENCES groups(id),created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,copy_count INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE preferences(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            INSERT INTO clips(content,kind,created_at,updated_at) VALUES('旧目录不可在预览时清理','text',0,0);").unwrap();
        let settings = r#"{"paused":true,"theme":"dark","layout":"grid","maxHistory":500,"shortcut":"CommandOrControl+Shift+V","retentionDays":1}"#;
        connection
            .execute("INSERT INTO preferences VALUES('settings',?1)", [settings])
            .unwrap();
        drop(connection);
        let before = fs::read(&database).unwrap();
        Store::validate_database(&database).unwrap();
        assert_eq!(fs::read(&database).unwrap(), before);
        let connection =
            Connection::open_with_flags(&database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        assert!(connection.prepare("SELECT language FROM clips").is_err());
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM clips", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn managed_renderer_paths_are_portable_and_reject_platform_escapes() {
        let directory = Path::new("synthetic-data");
        let relative = Path::new("renderers")
            .join("星光 pack")
            .join("xingjian.exe");
        assert_eq!(
            portable_renderer_path(&relative).unwrap(),
            "renderers/星光 pack/xingjian.exe"
        );
        for saved in [
            "renderers/星光 pack/xingjian.exe",
            r"renderers\星光 pack\xingjian.exe",
            r"renderers\星光 pack/xingjian.exe",
        ] {
            assert_eq!(
                renderer_path(directory, Path::new(saved)).unwrap(),
                directory.join(&relative)
            );
        }
        for invalid in [
            "",
            "../renderers/xingjian",
            "./renderers/xingjian",
            r"\renderers\xingjian",
            r"renderers\..\outside",
            r"renderers/..\outside",
            r"renderers\C:\outside",
            r"C:renderers\xingjian",
            "renderers/pack:stream/xingjian",
            "renderers/.. /outside",
            "renderers/pack./xingjian",
            "renderers//xingjian",
            "renderers/\0xingjian",
        ] {
            assert!(
                renderer_path(directory, Path::new(invalid)).is_err(),
                "{invalid:?}"
            );
        }
        #[cfg(unix)]
        {
            assert!(portable_renderer_path(Path::new(r"renderers/pack\literal")).is_err());
            for external in [
                r"C:\external\renderers\xingjian.exe",
                r"\\server\share\xingjian.exe",
            ] {
                assert!(renderer_path(directory, Path::new(external)).is_err());
            }
        }
    }

    #[test]
    fn legacy_windows_renderer_selection_survives_migration_and_recovery() {
        let scratch = Scratch::new();
        let source = scratch.0.join("source");
        let mut storage = Storage::open(&source, &source).unwrap();
        let store = Store::open(&source.join(DATABASE)).unwrap();
        fs::create_dir_all(source.join("renderers/pack")).unwrap();
        fs::write(
            source.join("renderers/pack/xingjian.exe"),
            "fixed fixture, never executed",
        )
        .unwrap();
        let legacy = serde_json::json!({"path": r"renderers\pack\xingjian.exe", "fixed": true});
        fs::write(source.join(SELECTION), legacy.to_string()).unwrap();
        let (bytes, external) = selection_data(&source, &source).unwrap().unwrap();
        let normalized: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(!external);
        assert_eq!(normalized["path"], "renderers/pack/xingjian.exe");
        assert_eq!(normalized["fixed"], true);
        assert_eq!(
            fs::read_to_string(source.join(SELECTION)).unwrap(),
            legacy.to_string()
        );

        let point = storage.create_recovery_point(&store, "manual").unwrap();
        let snapshot = storage.recovery_directory().join(&point.id);
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(snapshot.join(SELECTION)).unwrap()).unwrap();
        assert_eq!(saved["path"], "renderers/pack/xingjian.exe");
        // Model an authentic old Windows recovery point, including matching hashes.
        fs::write(snapshot.join(SELECTION), legacy.to_string()).unwrap();
        let (_, mut manifest) = read_manifest(&storage.recovery_directory(), &point.id).unwrap();
        manifest.files = collect_recovery_files(&snapshot).unwrap();
        manifest.total_bytes = manifest.files.values().map(|file| file.bytes).sum();
        fs::write(
            snapshot.join("snapshot.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        verified_recovery(&storage.recovery_directory(), &point.id).unwrap();
        let (restored, _) = prepare_recovery(&source, &point.id).unwrap();
        assert!(
            renderer_path(&restored, Path::new(r"renderers\pack\xingjian.exe"))
                .unwrap()
                .is_file()
        );

        let target = scratch.0.join("migrated");
        storage.apply(&store, DirectoryMode::Move, &target).unwrap();
        let moved: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join(SELECTION)).unwrap()).unwrap();
        assert_eq!(moved["path"], "renderers/pack/xingjian.exe");
        assert!(
            renderer_path(&target, Path::new(moved["path"].as_str().unwrap()))
                .unwrap()
                .is_file()
        );
    }

    #[test]
    fn legacy_external_selection_is_copied_but_external_program_is_not() {
        let scratch = Scratch::new();
        let source = scratch.0.join("source");
        let legacy = scratch.0.join("legacy-config");
        create_private_directory(&legacy).unwrap();
        let external_directory = scratch.0.join("external");
        create_private_directory(&external_directory).unwrap();
        create_private_directory(&external_directory.join("renderers")).unwrap();
        let external = external_directory.join("renderers/xingjian");
        fs::write(&external, "synthetic external program").unwrap();
        fs::write(
            legacy.join(SELECTION),
            serde_json::json!({"path":external}).to_string(),
        )
        .unwrap();
        let mut storage = Storage::open(&source, &legacy).unwrap();
        let store = Store::open(&source.join(DATABASE)).unwrap();
        create_private_directory(&source.join("renderers")).unwrap();
        fs::write(source.join("renderers/xingjian"), "different local program").unwrap();
        assert_eq!(renderer_path(&source, &external).unwrap(), external);
        assert_eq!(storage.info().unwrap().warnings.len(), 1);
        let target = scratch.0.join("migrated");
        storage.apply(&store, DirectoryMode::Move, &target).unwrap();
        assert!(target.join(SELECTION).exists());
        let selection: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join(SELECTION)).unwrap()).unwrap();
        assert_eq!(selection["path"], external.to_str().unwrap());
        assert!(!target.join("external-renderer").exists());
        assert!(legacy.join(SELECTION).exists());
        assert!(renderer_path(&target, Path::new("../outside")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn renderer_symlinks_abort_without_copying_external_files() {
        let scratch = Scratch::new();
        let source = scratch.0.join("source");
        let mut storage = Storage::open(&source, &source).unwrap();
        let store = Store::open(&source.join(DATABASE)).unwrap();
        create_private_directory(&source.join("renderers")).unwrap();
        std::os::unix::fs::symlink(&scratch.0, source.join("renderers/outside")).unwrap();
        let target = scratch.0.join("migrated");
        assert!(storage.apply(&store, DirectoryMode::Move, &target).is_err());
        assert!(!target.exists());
        assert!(!source.join(LOCATOR).exists());
        assert!(storage.writable().is_ok());
        let alias = scratch.0.join("source-alias");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        assert!(storage
            .apply(&store, DirectoryMode::Move, &alias.join("nested"))
            .is_err());
        assert!(!source.join("nested").exists());
    }
    #[cfg(unix)]
    #[test]
    fn font_symlinks_reject_migration_and_recovery_without_switching_data() {
        for link_directory in [true, false] {
            let scratch = Scratch::new();
            let source = scratch.0.join("source");
            let mut storage = Storage::open(&source, &source).unwrap();
            let store = storage.open_store().unwrap();
            if link_directory {
                std::os::unix::fs::symlink(&scratch.0, source.join("fonts")).unwrap();
            } else {
                create_private_directory(&source.join("fonts")).unwrap();
                std::os::unix::fs::symlink(&scratch.0, source.join("fonts/outside")).unwrap();
            }
            let target = scratch.0.join("migrated");
            assert!(storage.apply(&store, DirectoryMode::Move, &target).is_err());
            assert!(storage.create_recovery_point(&store, "manual").is_err());
            assert!(!target.exists());
            assert!(!source.join(LOCATOR).exists());
            assert!(storage.writable().is_ok());
        }
    }

    fn legacy_fixture(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("CREATE TABLE groups(id INTEGER PRIMARY KEY,name TEXT NOT NULL UNIQUE,color TEXT NOT NULL);
            CREATE TABLE clips(id INTEGER PRIMARY KEY,content TEXT NOT NULL UNIQUE,title TEXT NOT NULL DEFAULT '',kind TEXT NOT NULL,pinned INTEGER NOT NULL DEFAULT 0,group_id INTEGER REFERENCES groups(id),created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,copy_count INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE preferences(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            INSERT INTO groups VALUES(4,'固定分组','#ffffff');
            INSERT INTO clips VALUES(7,'原创保留文本','原有标题','text',1,4,1,2,3);
            INSERT INTO clips VALUES(8,'原创到期文本','','text',0,NULL,0,0,0);
            INSERT INTO preferences VALUES('custom-test','fixed preference');").unwrap();
        let settings = r#"{"paused":true,"theme":"dark","layout":"grid","maxHistory":500,"shortcut":"CommandOrControl+Shift+V","retentionDays":1}"#;
        conn.execute("INSERT INTO preferences VALUES('settings',?1)", [settings])
            .unwrap();
    }

    #[test]
    fn legacy_upgrade_snapshots_before_any_schema_write_or_retention_pruning() {
        let scratch = Scratch::new();
        let source = scratch.0.join("source");
        let storage = Storage::open(&source, &source).unwrap();
        legacy_fixture(&source.join(DATABASE));
        let legacy_config = source.join(SELECTION);
        create_private_directory(&source.join("renderers")).unwrap();
        fs::write(source.join("renderers/local"), "synthetic renderer").unwrap();
        fs::write(&legacy_config, r#"{"path":"renderers/local"}"#).unwrap();
        let store = storage.open_store().unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(store.clips().unwrap().len(), 1);
        let clip = store.clip(7).unwrap();
        assert_eq!(clip.title, "原有标题");
        assert_eq!(clip.group_id, Some(4));
        assert_eq!(clip.copy_count, 3);
        let points = storage.recovery_points().unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].reason, "before-migration");
        assert_eq!(points[0].schema_version, 0);
        let snapshot = Path::new(&points[0].path);
        verified_recovery(&storage.recovery_directory(), &points[0].id).unwrap();
        let old = Connection::open_with_flags(
            snapshot.join(DATABASE),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert!(old.prepare("SELECT language FROM clips").is_err());
        assert_eq!(
            old.query_row("SELECT count(*) FROM clips", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            old.query_row(
                "SELECT value FROM preferences WHERE key='custom-test'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "fixed preference"
        );
        assert!(snapshot.join("renderers/local").is_file());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                &fs::read(snapshot.join(SELECTION)).unwrap()
            )
            .unwrap(),
            serde_json::from_slice::<serde_json::Value>(&fs::read(&legacy_config).unwrap())
                .unwrap()
        );
        assert!(!snapshot.join("recovery").exists());
        assert!(
            storage.plan(DirectoryMode::Use, snapshot).is_err(),
            "archive cannot become writable active data"
        );
        drop(store);
        storage.open_store().unwrap();
        assert_eq!(
            storage.recovery_points().unwrap().len(),
            1,
            "no redundant schema migration snapshot"
        );
    }

    #[test]
    fn future_schema_is_rejected_without_writes_or_empty_replacement() {
        let scratch = Scratch::new();
        let path = scratch.0.join(DATABASE);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE future_payload(value TEXT); INSERT INTO future_payload VALUES('fixed future data'); PRAGMA user_version=99;").unwrap();
        drop(conn);
        let before = fs::read(&path).unwrap();
        assert!(Store::open(&path).err().unwrap().contains("较新版本"));
        assert!(Store::validate_database(&path)
            .unwrap_err()
            .contains("较新版本"));
        assert_eq!(before, fs::read(path).unwrap());
        assert!(!scratch.0.join("recovery").exists());
    }

    #[test]
    fn failed_upgrade_rolls_back_early_columns_and_keeps_recovery() {
        let scratch = Scratch::new();
        let path = scratch.0.join(DATABASE);
        legacy_fixture(&path);
        let conn = Connection::open(&path).unwrap();
        // Rebuilding UNIQUE(content) while an image table exists is deliberately unsupported.
        conn.execute_batch("CREATE TABLE clip_images(clip_id INTEGER PRIMARY KEY);")
            .unwrap();
        drop(conn);
        let before = fs::read(&path).unwrap();
        let error = Store::open(&path).err().unwrap();
        assert!(error.contains("恢复点已保留"));
        assert_eq!(before, fs::read(&path).unwrap());
        let conn =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(Store::check_schema_version(&conn).unwrap(), 0);
        assert!(conn.prepare("SELECT language FROM clips").is_err());
        assert!(conn.prepare("SELECT keep_forever FROM clips").is_err());
        assert_eq!(
            Storage::recovery_points_at_startup(&scratch.0)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn failed_snapshot_aborts_migration_before_source_schema_changes() {
        let scratch = Scratch::new();
        let path = scratch.0.join(DATABASE);
        legacy_fixture(&path);
        fs::write(scratch.0.join("recovery"), "not a directory").unwrap();
        let before = fs::read(&path).unwrap();
        assert!(Store::open(&path).is_err());
        assert_eq!(before, fs::read(path).unwrap());
    }

    #[test]
    fn recovery_preserves_wal_images_preferences_and_original_directory() {
        let scratch = Scratch::new();
        let source = scratch.0.join("source");
        let mut storage = Storage::open(&source, &source).unwrap();
        let store = storage.open_store().unwrap();
        let wal = Connection::open(source.join(DATABASE)).unwrap();
        wal.pragma_update(None, "journal_mode", "WAL").unwrap();
        wal.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        let image = crate::images::prepare_rgba(1, 1, &[32, 64, 128, 255]).unwrap();
        let clip = store.add_image(&image, "原创固定图片", None, true).unwrap();
        let mut settings = store.settings().unwrap();
        settings.theme = "forest".into();
        store.save_settings(&settings).unwrap();
        wal.execute(
            "INSERT INTO preferences VALUES('growth-fixture','fixed progress')",
            [],
        )
        .unwrap();
        let point = storage.create_recovery_point(&store, "manual").unwrap();
        let retained = store.add("恢复后仍在原目录的文本", "", None).unwrap();
        storage.restore_recovery_point(&store, &point.id).unwrap();
        assert!(storage.writable().is_err());
        let target = PathBuf::from(storage.pending_path().unwrap());
        assert!(target.starts_with(source.join("recovered")));
        assert_ne!(target, Path::new(&point.path));
        assert!(source.join(DATABASE).is_file());
        assert_eq!(
            store.clip(retained.id).unwrap().content,
            "恢复后仍在原目录的文本"
        );
        let copied = Store::open(&target.join(DATABASE)).unwrap();
        assert_eq!(copied.image_png(clip.id, false).unwrap(), image.png);
        assert_eq!(copied.image_png(clip.id, true).unwrap(), image.thumbnail);
        assert_eq!(copied.settings().unwrap(), settings);
        assert!(copied.clip(retained.id).is_err());
        let conn = Connection::open_with_flags(
            target.join(DATABASE),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT value FROM preferences WHERE key='growth-fixture'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "fixed progress"
        );
        let points = storage.recovery_points().unwrap();
        assert_eq!(points.len(), 2);
        assert!(points.iter().any(|p| p.reason == "before-restore"));
        verified_recovery(&storage.recovery_directory(), &point.id).unwrap();
        storage.release_for_restart().unwrap();
        assert_eq!(Storage::open(&source, &source).unwrap().current, target);
    }

    #[test]
    fn damaged_future_and_path_escape_snapshots_never_switch_data() {
        let scratch = Scratch::new();
        let source = scratch.0.join("source");
        let mut storage = Storage::open(&source, &source).unwrap();
        let store = storage.open_store().unwrap();
        store.add("原创校验测试", "", None).unwrap();
        let point = storage.create_recovery_point(&store, "manual").unwrap();
        let before = fs::read(source.join(DATABASE)).unwrap();
        assert!(storage
            .restore_recovery_point(&store, "../../other")
            .is_err());
        let manifest_path = Path::new(&point.path).join("snapshot.json");
        let original_manifest = fs::read(&manifest_path).unwrap();
        let mut manifest: serde_json::Value = serde_json::from_slice(&original_manifest).unwrap();
        manifest["schemaVersion"] = 99.into();
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(!storage.recovery_points().unwrap()[0].compatible);
        assert!(storage
            .restore_recovery_point(&store, &point.id)
            .unwrap_err()
            .contains("较新版本"));
        fs::write(&manifest_path, original_manifest).unwrap();
        let mut database = OpenOptions::new()
            .append(true)
            .open(Path::new(&point.path).join(DATABASE))
            .unwrap();
        database.write_all(b"tampered").unwrap();
        assert!(storage
            .restore_recovery_point(&store, &point.id)
            .unwrap_err()
            .contains("校验失败"));
        assert_eq!(before, fs::read(source.join(DATABASE)).unwrap());
        assert!(!source.join(LOCATOR).exists());
        assert!(!source.join("recovered").exists());
        assert_eq!(storage.recovery_points().unwrap().len(), 1);
        assert!(storage.writable().is_ok());
    }

    #[test]
    fn startup_recovery_copies_snapshot_without_overwriting_failed_database() {
        let scratch = Scratch::new();
        let source = scratch.0.join("source");
        let storage = Storage::open(&source, &source).unwrap();
        let store = storage.open_store().unwrap();
        store.add("原创启动恢复", "", None).unwrap();
        let point = storage.create_recovery_point(&store, "manual").unwrap();
        drop(store);
        drop(storage);
        fs::write(source.join(DATABASE), "simulated corrupted database").unwrap();
        let target = Storage::restore_recovery_point_at_startup(&source, &point.id).unwrap();
        assert_eq!(
            fs::read_to_string(source.join(DATABASE)).unwrap(),
            "simulated corrupted database"
        );
        assert_eq!(Storage::open(&source, &source).unwrap().current, target);
        verified_recovery(&source.join("recovery"), &point.id).unwrap();
    }

    #[test]
    fn update_freeze_blocks_writes_and_releases_lock_only_for_restart() {
        let scratch = Scratch::new();
        let mut storage = Storage::open(&scratch.0, &scratch.0).unwrap();
        let _store = storage.open_store().unwrap();
        storage.freeze_for_update().unwrap();
        assert!(storage.writable().is_err());
        assert!(storage.restart_pending());
        storage.cancel_update_freeze();
        assert!(storage.writable().is_ok());
        storage.freeze_for_update().unwrap();
        assert!(Storage::open(&scratch.0, &scratch.0).is_err());
        storage.release_for_restart().unwrap();
        assert!(storage.writable().is_err());
        Storage::open(&scratch.0, &scratch.0).unwrap();
    }
    #[test]
    fn migration_retention_failure_also_rolls_back_schema_version() {
        let scratch = Scratch::new();
        let path = scratch.0.join(DATABASE);
        let store = Store::open(&path).unwrap();
        store.add("原创过期触发器测试", "", None).unwrap();
        drop(store);
        let conn = Connection::open(&path).unwrap();
        let settings = r#"{"paused":true,"theme":"dark","layout":"grid","maxHistory":500,"shortcut":"CommandOrControl+Shift+V","retentionDays":1}"#;
        conn.execute("INSERT INTO preferences VALUES('settings',?1)", [settings])
            .unwrap();
        conn.execute_batch("UPDATE clips SET updated_at=0; PRAGMA user_version=0;
            CREATE TRIGGER prevent_cleanup BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'fixed cleanup failure'); END;").unwrap();
        drop(conn);
        let before = fs::read(&path).unwrap();
        assert!(Store::open(&path).err().unwrap().contains("恢复点已保留"));
        assert_eq!(before, fs::read(&path).unwrap());
        let conn =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(Store::check_schema_version(&conn).unwrap(), 0);
        assert_eq!(
            conn.query_row("SELECT count(*) FROM clips", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn failed_restore_locator_commit_retains_current_and_both_snapshots() {
        let scratch = Scratch::new();
        let source = scratch.0.join("source");
        let mut storage = Storage::open(&source, &source).unwrap();
        let store = storage.open_store().unwrap();
        store.add("原创恢复失败测试", "", None).unwrap();
        let point = storage.create_recovery_point(&store, "manual").unwrap();
        create_private_directory(&source.join(LOCATOR)).unwrap();
        let before = fs::read(source.join(DATABASE)).unwrap();
        assert!(storage.restore_recovery_point(&store, &point.id).is_err());
        assert_eq!(before, fs::read(source.join(DATABASE)).unwrap());
        assert_eq!(fs::read_dir(source.join("recovered")).unwrap().count(), 0);
        assert_eq!(storage.recovery_points().unwrap().len(), 2);
        assert!(storage.writable().is_ok());
        assert!(storage.pending_path().is_none());
    }

    #[test]
    fn malformed_manifest_does_not_hide_other_recovery_points() {
        let scratch = Scratch::new();
        let storage = Storage::open(&scratch.0, &scratch.0).unwrap();
        let store = storage.open_store().unwrap();
        let good = storage.create_recovery_point(&store, "manual").unwrap();
        let bad = storage.create_recovery_point(&store, "manual").unwrap();
        fs::write(
            Path::new(&bad.path).join("snapshot.json"),
            "incomplete manifest",
        )
        .unwrap();
        let points = storage.recovery_points().unwrap();
        assert_eq!(points.len(), 2);
        assert!(points.iter().find(|p| p.id == good.id).unwrap().compatible);
        let corrupt = points.iter().find(|p| p.id == bad.id).unwrap();
        assert!(!corrupt.compatible);
        assert!(!corrupt.warnings.is_empty());
    }
}
