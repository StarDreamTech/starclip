//! Bounded, local PNG batches. Callers keep the store locked for the entire snapshot.
use crate::store::{Result, Store};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

pub const MAX_IMAGES: usize = 50;
pub const MAX_BYTES: usize = 100 * 1024 * 1024;
const CACHE_BYTES: u64 = 512 * 1024 * 1024;
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);

pub struct Batch {
    pub token: String,
    pub ids: Vec<i64>,
    pub identities: Vec<String>,
    pub paths: Vec<PathBuf>,
    pub preview: Vec<u8>,
    pub allow_private: bool,
    pub created: SystemTime,
    pub retained: bool,
}

pub fn validate(store: &Store, ids: &[i64], allow_private: bool) -> Result<Vec<String>> {
    if ids.is_empty()
        || ids.len() > MAX_IMAGES
        || ids.iter().any(|id| *id <= 0)
        || ids.iter().collect::<HashSet<_>>().len() != ids.len()
    {
        return Err("请选择 1 至 50 张不同的图片。".into());
    }
    ids.iter()
        .map(|id| {
            let clip = store.clip(*id)?;
            if clip.kind != "image" {
                return Err("所选记录包含非图片或已删除的图片，请重新选择。".into());
            }
            if !allow_private
                && (clip.is_private || clip.privacy_manual || !clip.privacy_reasons.is_empty())
            {
                return Err("所选图片包含隐私记录，请确认后再传出。".into());
            }
            Ok(clip.record_identity)
        })
        .collect()
}

fn private_directory(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|_| "无法创建图片目录，请检查权限与磁盘空间。".into())
}

/// Only clean our own flat, random-named batches; never follow symlinks.
pub fn cache_root(root: &Path) -> Result<()> {
    if !root.exists() {
        private_directory(root)?;
    }
    let meta = fs::symlink_metadata(root).map_err(|_| "无法读取图片临时目录。")?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("图片临时目录无效。".into());
    }
    let mut bytes = 0u64;
    for entry in fs::read_dir(root).map_err(|_| "无法读取图片临时目录。")? {
        let entry = entry.map_err(|_| "无法读取图片临时目录。")?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(token) = name.strip_prefix("Starclip-images-").or_else(|| {
            name.strip_prefix(".Starclip-images-")
                .and_then(|s| s.strip_suffix(".tmp"))
        }) else {
            continue;
        };
        if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let meta = fs::symlink_metadata(entry.path()).map_err(|_| "无法读取图片临时目录。")?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            continue;
        }
        if meta
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > TTL)
        {
            fs::remove_dir_all(entry.path()).map_err(|_| "无法清理过期图片临时副本。")?;
        } else {
            for file in fs::read_dir(entry.path()).map_err(|_| "无法读取图片临时目录。")?
            {
                let meta = fs::symlink_metadata(file.map_err(|_| "无法读取图片临时目录。")?.path())
                    .map_err(|_| "无法读取图片临时目录。")?;
                bytes = bytes.saturating_add(meta.len());
            }
        }
    }
    // Reserve a full batch; don't evict live clipboard files to make room.
    if bytes > CACHE_BYTES - MAX_BYTES as u64 {
        return Err(
            "图片临时空间已满，请使用「导出到文件夹」。过期副本将在后续操作时清理。".into(),
        );
    }
    Ok(())
}

pub(crate) fn stage(store: &Store, root: &Path, ids: &[i64], allow_private: bool) -> Result<Batch> {
    let identities = validate(store, ids, allow_private)?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| "无法创建图片传输编号。")?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let directory = root.join(format!(".Starclip-images-{token}.tmp"));
    private_directory(&directory)?;
    let result = (|| {
        let mut paths = Vec::new();
        let mut bytes = 0usize;
        for (index, id) in ids.iter().enumerate() {
            let png = store.image_png(*id, false)?;
            bytes = bytes.saturating_add(png.len());
            if bytes > MAX_BYTES {
                return Err("所选图片总大小超过 100 MiB，请分批操作。".into());
            }
            // Validate one original at a time; never expand all selected images in memory.
            crate::images::decode_png(&png)?;
            // Numeric names avoid leaking private titles and are safe on both platforms.
            let path = directory.join(format!("Starclip-{:02}-{id}.png", index + 1));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&path)
                .map_err(|_| "无法写入图片，请检查权限与磁盘空间。")?;
            file.write_all(&png)
                .and_then(|_| file.sync_all())
                .map_err(|_| "无法写入图片，请检查权限与磁盘空间。")?;
            paths.push(path);
        }
        let preview = store.image_png(ids[0], true)?;
        let (width, height, _) = crate::images::decode_png(&preview)?;
        if width > 320 || height > 320 {
            return Err("图片缩略图无效，请重新收录图片。".into());
        }
        let published = root.join(format!("Starclip-images-{token}"));
        fs::rename(&directory, &published)
            .map_err(|_| "无法完成图片导出，请检查权限与磁盘空间。")?;
        let paths = paths
            .iter()
            .map(|p| published.join(p.file_name().unwrap()))
            .collect();
        Ok(Batch {
            token,
            ids: ids.to_vec(),
            identities,
            paths,
            preview,
            allow_private,
            created: SystemTime::now(),
            retained: false,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&directory);
    }
    result
}

impl Batch {
    pub fn revalidate(&self, store: &Store) -> Result<()> {
        if self.created.elapsed().unwrap_or(TTL) >= TTL
            || validate(store, &self.ids, self.allow_private)? != self.identities
        {
            return Err("图片选择已变化或临时副本已过期，请重新准备拖出。".into());
        }
        for path in &self.paths {
            let meta =
                fs::symlink_metadata(path).map_err(|_| "图片临时副本已不可用，请重新准备拖出。")?;
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err("图片临时副本已不可用，请重新准备拖出。".into());
            }
        }
        Ok(())
    }
}
