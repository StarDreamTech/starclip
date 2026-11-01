//! Pinned paper catalog. Only explicitly chosen, verified originals are cached.
use super::{regular_directory, TemporaryFont, MAX_PIXELS, TEMP_SEQUENCE};
use crate::store::Result;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{atomic::Ordering, OnceLock},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PaperDefinition {
    pub id: String,
    name: String,
    file: String,
    pub bytes: u64,
    sha256: String,
    pub url: String,
    preview_url: String,
    width: u32,
    height: u32,
    insets: [u32; 4],
    ink: String,
    padding: u32,
    solid: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperAsset {
    pub id: String,
    pub name: String,
    pub path: String,
    pub downloaded: bool,
    bytes: u64,
    preview_url: String,
    width: u32,
    height: u32,
    ink: String,
    padding: u32,
}

fn catalog() -> &'static [PaperDefinition] {
    static PAPERS: OnceLock<Vec<PaperDefinition>> = OnceLock::new();
    PAPERS.get_or_init(|| {
        serde_json::from_str(include_str!("../../public/handwriting-papers/catalog.json"))
            .expect("valid built-in paper catalog")
    })
}
pub(super) fn definition(id: &str) -> Result<&'static PaperDefinition> {
    catalog()
        .iter()
        .find(|paper| paper.id == id)
        .ok_or_else(|| "请选择纸张列表中的纸张。".into())
}
fn from_background(background: &str) -> Result<&'static PaperDefinition> {
    let paper = definition(background.strip_prefix("paper:").ok_or("纸张编号无效。")?)?;
    if paper.solid.is_some() {
        return Err("此纸张应使用内置纯色模式。".into());
    }
    Ok(paper)
}
fn path(directory: &Path, paper: &PaperDefinition) -> PathBuf {
    directory.join("papers").join(format!("{}.img", paper.id))
}
pub(super) fn verify(paper: &PaperDefinition, bytes: &[u8]) -> Result<()> {
    if paper.bytes == 0
        || paper.bytes > 4 * 1024 * 1024
        || bytes.len() as u64 != paper.bytes
        || format!("{:x}", Sha256::digest(bytes)) != paper.sha256
    {
        return Err("纸张大小或校验不符，未安装，请重试。".into());
    }
    let dimensions = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "无法识别纸张图片。")?
        .into_dimensions()
        .map_err(|_| "纸张图片损坏。")?;
    if dimensions != (paper.width, paper.height)
        || !(128..=4096).contains(&paper.width)
        || !(128..=4096).contains(&paper.height)
        || u64::from(paper.width) * u64::from(paper.height) > MAX_PIXELS
    {
        return Err("纸张尺寸不符或超过限制。".into());
    }
    Ok(())
}
fn installed(directory: &Path, paper: &PaperDefinition) -> Result<Vec<u8>> {
    regular_directory(directory)?;
    regular_directory(&directory.join("papers"))?;
    let path = path(directory, paper);
    let meta = fs::symlink_metadata(&path).map_err(|_| "请先下载所选纸张。")?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() != paper.bytes {
        return Err("纸张文件不完整，请重新下载。".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| "无法读取纸张。")?
        .take(paper.bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取纸张。")?;
    verify(paper, &bytes)?;
    Ok(bytes)
}
pub(super) fn assets(directory: &Path) -> Vec<PaperAsset> {
    catalog()
        .iter()
        .map(|paper| PaperAsset {
            id: paper.id.clone(),
            name: paper.name.clone(),
            path: paper
                .solid
                .clone()
                .unwrap_or_else(|| format!("paper:{}", paper.id)),
            downloaded: paper.solid.is_some() || installed(directory, paper).is_ok(),
            bytes: paper.bytes,
            preview_url: paper.preview_url.clone(),
            width: paper.width,
            height: paper.height,
            ink: paper.ink.clone(),
            padding: paper.padding,
        })
        .collect()
}
pub(super) fn install(directory: &Path, id: &str, bytes: &[u8]) -> Result<()> {
    let paper = definition(id)?;
    verify(paper, bytes)?;
    regular_directory(directory)?;
    let folder = directory.join("papers");
    if !folder.exists() {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&folder).map_err(|_| "无法创建纸张目录。")?;
    }
    regular_directory(&folder)?;
    if installed(directory, paper).is_ok() {
        return Ok(());
    }
    let target = path(directory, paper);
    if fs::symlink_metadata(&target)
        .is_ok_and(|meta| !meta.is_file() || meta.file_type().is_symlink())
    {
        return Err("纸张目标必须是普通文件。".into());
    }
    let temporary = TemporaryFont(folder.join(format!(
        ".download-{}-{}.tmp",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )));
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary.0).map_err(|_| "无法暂存纸张。")?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| "无法保存纸张。")?;
    drop(file);
    // Any existing file failed verification above; remove it for Windows rename.
    if target.exists() {
        fs::remove_file(&target).map_err(|_| "无法替换损坏的纸张。")?;
    }
    fs::rename(&temporary.0, &target).map_err(|_| "无法安装纸张。")?;
    Ok(())
}
pub(super) fn dimensions(background: &str, padding: u32) -> Result<(u32, u32)> {
    let paper = from_background(background)?;
    let [left, top, right, bottom] = paper.insets;
    if paper.width.saturating_sub(left + right + padding * 2) < 32
        || paper.height.saturating_sub(top + bottom + padding * 2) < 32
    {
        return Err("纸张可书写区域过小，请减小留白。".into());
    }
    Ok((paper.width, paper.height))
}
pub(super) fn canvas(directory: &Path, background: &str) -> Result<(RgbaImage, [u32; 4])> {
    let paper = from_background(background)?;
    let bytes = installed(directory, paper)?;
    let format = if paper.file.ends_with(".png") {
        image::ImageFormat::Png
    } else {
        image::ImageFormat::Jpeg
    };
    let image = image::load_from_memory_with_format(&bytes, format)
        .map_err(|_| "纸张图片损坏。")?
        .to_rgba8();
    Ok((image, paper.insets))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_papers_are_pinned_and_bounded_without_implicit_downloads() {
        assert_eq!(catalog().len(), 9);
        assert_eq!(catalog().iter().filter(|p| p.solid.is_some()).count(), 4);
        for paper in catalog() {
            assert!(paper.url.starts_with("https://raw.githubusercontent.com/cpython666/HandwritingGenerator/d7036998501e9de3dbe57af4aaa1b7d289547ec8/bgs/"));
            assert_eq!(paper.sha256.len(), 64);
            assert!(paper.width as u64 * paper.height as u64 <= MAX_PIXELS);
            assert!(verify(paper, b"original corrupt fixture").is_err());
        }
        assert!(definition("../../private/file").is_err());
        assert!(from_background("/private/paper.jpg").is_err());
        let assets = assets(Path::new("/original/nonexistent-fixture"));
        assert_eq!(assets.iter().filter(|p| p.downloaded).count(), 4);
        assert_eq!(dimensions("paper:letter", 12).unwrap(), (724, 1024));
        assert!(dimensions("paper:letter", 500).is_err());
    }
}
