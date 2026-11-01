//! Built-in handwriting rendering with explicitly downloaded, verified fonts.
//! Opening the catalog and rendering installed fonts never use the network.
use crate::store::Result;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::Duration,
};
#[path = "handwriting_papers.rs"]
mod papers;
#[path = "handwriting_render.rs"]
mod rendering;

const MAX_TEXT: usize = 10_000;
const MAX_PNG: usize = 10 * 1024 * 1024;
const MAX_PIXELS: u64 = 8_000_000;
const MAX_FONT: u64 = 16 * 1024 * 1024;
static RENDER_GATE: Mutex<()> = Mutex::new(());
static DOWNLOAD_GATE: Mutex<()> = Mutex::new(());
static INSTALL_GATE: Mutex<()> = Mutex::new(());
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FontDefinition {
    id: String,
    name: String,
    author: String,
    tags: Vec<String>,
    #[cfg(test)]
    file: String,
    bytes: u64,
    sha256: String,
    url: String,
    preview_url: String,
    license: String,
    license_url: String,
    source_url: String,
}
fn catalog() -> &'static [FontDefinition] {
    static FONTS: OnceLock<Vec<FontDefinition>> = OnceLock::new();
    FONTS.get_or_init(|| {
        serde_json::from_str(include_str!("../../public/handwriting-fonts/catalog.json"))
            .expect("built-in font catalog must be valid")
    })
}
fn definition(id: &str) -> Result<&'static FontDefinition> {
    catalog()
        .iter()
        .find(|font| font.id == id)
        .ok_or_else(|| "请选择字体列表中的字体。".into())
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandwritingFont {
    pub id: String,
    pub name: String,
    pub author: String,
    pub tags: Vec<String>,
    pub path: String,
    pub downloaded: bool,
    pub bytes: u64,
    pub preview_url: String,
    pub license: String,
    pub license_url: String,
    pub source_url: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandwritingAssets {
    pub available: bool,
    pub path: Option<String>,
    pub fonts: Vec<HandwritingFont>,
    pub backgrounds: Vec<papers::PaperAsset>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HandwritingOptions {
    pub text: String,
    pub font: String,
    pub background: String,
    pub font_size: u32,
    pub ink: String,
    pub padding: u32,
}

fn font_path(directory: &Path, font: &FontDefinition) -> PathBuf {
    directory.join("fonts").join(format!("{}.ttf", font.id))
}
fn verify_font(font: &FontDefinition, bytes: &[u8]) -> Result<()> {
    if font.bytes > MAX_FONT || bytes.len() as u64 != font.bytes {
        return Err("字体下载不完整或大小不符，未安装，请重试。".into());
    }
    if format!("{:x}", Sha256::digest(bytes)) != font.sha256 {
        return Err("字体校验失败，未安装，请重新下载。".into());
    }
    Ok(())
}
fn regular_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "字体目录不可用。")?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("字体目录必须是普通文件夹，不能使用符号链接。".into());
    }
    Ok(())
}
fn installed_bytes(directory: &Path, font: &FontDefinition) -> Result<Vec<u8>> {
    regular_directory(directory)?;
    regular_directory(&directory.join("fonts"))?;
    let path = font_path(directory, font);
    let meta = fs::symlink_metadata(&path).map_err(|_| "请先下载所选字体。")?;
    if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len() != font.bytes {
        return Err("字体文件不完整，请重新下载。".into());
    }
    let mut bytes = Vec::with_capacity(font.bytes as usize);
    File::open(path)
        .map_err(|_| "无法读取字体。")?
        .take(font.bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取字体。")?;
    verify_font(font, &bytes)?;
    Ok(bytes)
}

pub fn assets(directory: &Path) -> HandwritingAssets {
    HandwritingAssets {
        available: true,
        path: Some("builtin".into()),
        fonts: catalog()
            .iter()
            .map(|font| HandwritingFont {
                id: font.id.clone(),
                name: font.name.clone(),
                author: font.author.clone(),
                tags: font.tags.clone(),
                path: font.id.clone(),
                downloaded: installed_bytes(directory, font).is_ok(),
                bytes: font.bytes,
                preview_url: font.preview_url.clone(),
                license: font.license.clone(),
                license_url: font.license_url.clone(),
                source_url: font.source_url.clone(),
            })
            .collect(),
        backgrounds: papers::assets(directory),
        error: None,
    }
}

/// Compatibility for older callers. No selected executable is read or run.
pub fn select_app(directory: &Path, _selected: &Path) -> Result<HandwritingAssets> {
    Ok(assets(directory))
}

/// Downloads only a built-in font ID. Call off the UI thread, without storage locks.
pub fn download_font_bytes(id: &str) -> Result<Vec<u8>> {
    download_font_bytes_with(id, |_, _| {})
}

/// Same as [`download_font_bytes`], reporting `(received, expected)` bytes while streaming
/// so the desktop UI can show real progress instead of an indeterminate wait.
pub fn download_font_bytes_with(id: &str, mut progress: impl FnMut(u64, u64)) -> Result<Vec<u8>> {
    let font = definition(id)?;
    let bytes = download_asset_bytes(&font.url, font.bytes, "字体", &mut progress)?;
    verify_font(font, &bytes)?;
    Ok(bytes)
}

fn download_asset_bytes(
    url: &str,
    expected: u64,
    label: &str,
    mut progress: impl FnMut(u64, u64),
) -> Result<Vec<u8>> {
    if expected == 0 || expected > MAX_FONT {
        return Err("素材大小超过限制，未下载。".into());
    }
    let _guard = DOWNLOAD_GATE
        .try_lock()
        .map_err(|_| "已有素材正在下载，请稍后重试。")?;
    // Reuse the updater's ring provider; an already installed provider is retained.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .user_agent(concat!("Starclip/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "无法创建素材下载连接，请稍后重试。")?;
    let response = client.get(url).send().map_err(download_error)?;
    if !response.status().is_success() {
        return Err(format!(
            "GitHub {label}下载链接返回 HTTP {}，请稍后重试。",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size != expected)
    {
        return Err("素材下载大小与清单不符，未安装。".into());
    }
    let mut bytes = Vec::with_capacity(expected as usize);
    let mut reader = response.take(expected + 1);
    // One report per percent keeps the bridge quiet while still feeling live.
    let step = (expected / 100).max(16 * 1024);
    let mut reported = 0u64;
    progress(0, expected);
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|_| "GitHub 素材下载中断或超时，请检查网络后重试。")?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        let received = bytes.len() as u64;
        if received == expected || received - reported >= step {
            reported = received;
            progress(received, expected);
        }
    }
    if bytes.len() as u64 != expected {
        return Err(format!("{label}下载不完整或大小不符，未安装，请重试。"));
    }
    Ok(bytes)
}

pub fn download_paper_bytes_with(id: &str, progress: impl FnMut(u64, u64)) -> Result<Vec<u8>> {
    let paper = papers::definition(id)?;
    let bytes = download_asset_bytes(&paper.url, paper.bytes, "纸张", progress)?;
    papers::verify(paper, &bytes)?;
    Ok(bytes)
}

pub fn install_paper(directory: &Path, id: &str, bytes: Vec<u8>) -> Result<HandwritingAssets> {
    papers::install(directory, id, &bytes)?;
    Ok(assets(directory))
}
fn download_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "连接 GitHub 素材服务超时，请检查网络后重试。"
    } else {
        "GitHub 素材链接访问失败，请检查网络连接后重试。"
    }
    .into()
}

struct TemporaryFont(PathBuf);
impl Drop for TemporaryFont {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn write_font(directory: &Path, font: &FontDefinition, bytes: &[u8]) -> Result<()> {
    verify_font(font, bytes)?;
    regular_directory(directory)?;
    let fonts = directory.join("fonts");
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    if !fonts.exists() {
        builder
            .create(&fonts)
            .map_err(|_| "无法创建字体目录，请检查磁盘空间。")?;
    }
    regular_directory(&fonts)?;
    if installed_bytes(directory, font).is_ok() {
        return Ok(());
    }
    let target = font_path(directory, font);
    if fs::symlink_metadata(&target)
        .is_ok_and(|meta| !meta.file_type().is_file() || meta.file_type().is_symlink())
    {
        return Err("字体目标不是普通文件，未安装。".into());
    }
    let mut file_and_path = None;
    for _ in 0..100 {
        let temporary = fonts.join(format!(
            ".download-{}-{}.tmp",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&temporary) {
            Ok(file) => {
                file_and_path = Some((file, TemporaryFont(temporary)));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("无法保存字体，请检查磁盘空间。".into()),
        }
    }
    let (mut file, temporary) = file_and_path.ok_or("无法创建字体下载临时文件。")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "字体保存失败，请检查磁盘空间。")?;
    drop(file);
    fs::rename(&temporary.0, target).map_err(|_| "无法完成字体安装，请重试。")?;
    Ok(())
}
/// Recheck storage writability/location before this short, atomic installation step.
pub fn install_font(directory: &Path, id: &str, bytes: Vec<u8>) -> Result<HandwritingAssets> {
    let font = definition(id)?;
    let _guard = INSTALL_GATE.lock().map_err(|_| "字体安装状态不可用。")?;
    write_font(directory, font, &bytes)?;
    Ok(assets(directory))
}
pub fn download_font(directory: &Path, id: &str) -> Result<HandwritingAssets> {
    let font = definition(id)?;
    if installed_bytes(directory, font).is_ok() {
        return Ok(assets(directory));
    }
    let bytes = download_font_bytes(id)?;
    install_font(directory, id, bytes)
}

fn is_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}
fn color(value: &str) -> [u8; 4] {
    [
        u8::from_str_radix(&value[1..3], 16).unwrap(),
        u8::from_str_radix(&value[3..5], 16).unwrap(),
        u8::from_str_radix(&value[5..7], 16).unwrap(),
        if value.len() == 9 {
            u8::from_str_radix(&value[7..9], 16).unwrap()
        } else {
            255
        },
    ]
}
fn solid_dimensions(background: &str) -> Result<(u32, u32)> {
    let parts: Vec<_> = background.split(':').collect();
    if parts.len() != 4
        || parts[0] != "solid"
        || !(is_color(parts[1])
            || (parts[1].len() == 9
                && parts[1].starts_with('#')
                && parts[1][1..].bytes().all(|b| b.is_ascii_hexdigit())))
    {
        return Err("请选择纯色纸张。".into());
    }
    let width: u32 = parts[2].parse().map_err(|_| "纸张宽度无效。")?;
    let height: u32 = parts[3].parse().map_err(|_| "纸张高度无效。")?;
    if !(128..=4096).contains(&width)
        || !(128..=4096).contains(&height)
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err("纸张尺寸需为 128–4096 像素，且总像素不超过 800 万。".into());
    }
    Ok((width, height))
}
fn validate_options(options: &HandwritingOptions) -> Result<(u32, u32)> {
    if options.text.trim().is_empty()
        || options.text.chars().count() > MAX_TEXT
        || options.text.contains('\0')
    {
        return Err("正文需为 1–10000 个字符，且不能包含空字符。".into());
    }
    if !(8..=120).contains(&options.font_size) || options.padding > 240 || !is_color(&options.ink) {
        return Err("字号需为 8–120，边距为 0–240，墨色为 #RRGGBB。".into());
    }
    definition(&options.font)?;
    let dimensions = if options.background.starts_with("paper:") {
        papers::dimensions(&options.background, options.padding)?
    } else {
        solid_dimensions(&options.background)?
    };
    if dimensions.0.saturating_sub(options.padding * 2) < options.font_size * 2
        || dimensions.1.saturating_sub(options.padding * 2) < options.font_size * 2
    {
        return Err("纸张可书写区域过小，请减小边距或字号。".into());
    }
    Ok(dimensions)
}
pub fn render(directory: &Path, options: HandwritingOptions) -> Result<String> {
    let dimensions = validate_options(&options)?;
    let _guard = RENDER_GATE
        .try_lock()
        .map_err(|_| "已有手写渲染任务，请等待完成后重试。")?;
    let bytes = installed_bytes(directory, definition(&options.font)?)?;
    let image = if options.background.starts_with("paper:") {
        let (image, bounds) = papers::canvas(directory, &options.background)?;
        rendering::render_on_paper(
            &bytes,
            &options.text,
            options.font_size,
            options.padding,
            image,
            bounds,
            color(&options.ink),
        )?
    } else {
        rendering::render(
            &bytes,
            &options.text,
            options.font_size,
            options.padding,
            dimensions,
            color(options.background.split(':').nth(1).unwrap()),
            color(&options.ink),
        )?
    };
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| "手写图片生成失败。")?;
    if png.get_ref().len() > MAX_PNG {
        return Err("生成图片超过 10 MB，请减小纸张。".into());
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(png.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "starclip-font-test-{}-{}",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn options() -> HandwritingOptions {
        HandwritingOptions {
            text: "星光落在纸上。\n  <原样文字> & 123".into(),
            font: "ma-shan-zheng".into(),
            background: "solid:#fffaf1:600:800".into(),
            font_size: 32,
            ink: "#26344a".into(),
            padding: 40,
        }
    }
    #[test]
    fn fresh_catalog_has_previews_and_no_font_or_network_side_effects() {
        let scratch = Scratch::new();
        let listed = assets(&scratch.0);
        assert!(listed.available);
        assert_eq!(listed.path.as_deref(), Some("builtin"));
        assert_eq!(listed.fonts.len(), catalog().len());
        assert!(listed
            .fonts
            .iter()
            .all(|font| !font.downloaded && font.bytes > 0 && font.preview_url.ends_with(".png")));
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0);
        assert!(render(&scratch.0, options()).unwrap_err().contains("字体"));
        assert!(download_font_bytes("../../secret").is_err());
    }
    #[test]
    fn catalog_fonts_have_unique_ids_and_bounded_immutable_sources() {
        let mut ids = std::collections::HashSet::new();
        for font in catalog() {
            assert!(ids.insert(&font.id));
            assert!(font
                .id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'));
            assert!(font.bytes > 0 && font.bytes <= MAX_FONT);
            assert_eq!(font.sha256.len(), 64);
            assert!(font.sha256.bytes().all(|c| c.is_ascii_hexdigit()));
            assert!(!font.author.trim().is_empty());
            assert!(!font.tags.is_empty());
            let source = font
                .url
                .strip_prefix("https://raw.githubusercontent.com/google/fonts/")
                .or_else(|| {
                    font.url.strip_prefix(
                        "https://raw.githubusercontent.com/cpython666/HandwritingGenerator/",
                    )
                })
                .unwrap();
            let (revision, path) = source.split_once('/').unwrap();
            assert_eq!(revision.len(), 40);
            assert!(revision.bytes().all(|c| c.is_ascii_hexdigit()));
            assert!(
                (path.starts_with("ofl/") || path.starts_with("fonts/")) && path.ends_with(".ttf")
            );
        }
    }
    #[test]
    #[ignore = "Explicit local verification using downloaded public font and paper fixtures; no clipboard or network"]
    fn all_original_papers_render_offline_from_verified_fixtures() {
        let scratch = Scratch::new();
        let fixtures = PathBuf::from(
            std::env::var("STARCLIP_PAPER_FIXTURES").expect("paper fixture directory"),
        );
        let font_fixture =
            PathBuf::from(std::env::var("STARCLIP_FONT_FIXTURE").expect("public font fixture"));
        let font = definition("ma-shan-zheng").unwrap();
        write_font(&scratch.0, font, &fs::read(font_fixture).unwrap()).unwrap();
        let output = PathBuf::from(
            std::env::var("STARCLIP_PAPER_OUTPUT").expect("fixed-data output directory"),
        );
        fs::create_dir_all(&output).unwrap();
        let entries: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../public/handwriting-papers/catalog.json"))
                .unwrap();
        for paper in entries {
            let id = paper["id"].as_str().unwrap();
            let original_bytes = fs::read(fixtures.join(paper["file"].as_str().unwrap())).unwrap();
            let original = image::load_from_memory(&original_bytes).unwrap().to_rgba8();
            let background = if let Some(solid) = paper["solid"].as_str() {
                solid.to_owned()
            } else {
                install_paper(&scratch.0, id, original_bytes).unwrap();
                format!("paper:{id}")
            };
            let encoded = render(
                &scratch.0,
                HandwritingOptions {
                    text: "星光落在纸上。\n纸张与文字都留在本机。".into(),
                    font: font.id.clone(),
                    background,
                    font_size: if original.width() < 400 { 16 } else { 30 },
                    ink: paper["ink"].as_str().unwrap().into(),
                    padding: paper["padding"].as_u64().unwrap() as u32,
                },
            )
            .unwrap();
            let png = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap();
            let rendered = image::load_from_memory(&png).unwrap().to_rgba8();
            assert_eq!(rendered.dimensions(), original.dimensions(), "{id}");
            assert_eq!(
                rendered.get_pixel(0, 0),
                original.get_pixel(0, 0),
                "original corner retained: {id}"
            );
            assert!(
                rendered
                    .pixels()
                    .zip(original.pixels())
                    .any(|(a, b)| a != b),
                "text must appear: {id}"
            );
            fs::write(output.join(format!("{id}.png")), png).unwrap();
        }
    }

    #[test]
    fn validates_limits_and_never_accepts_arbitrary_font_or_background_paths() {
        let mut opts = options();
        assert_eq!(validate_options(&opts).unwrap(), (600, 800));
        opts.font = "/private/font.ttf".into();
        assert!(validate_options(&opts).is_err());
        opts = options();
        opts.background = "builtin:bgs/letter.png".into();
        assert!(validate_options(&opts).is_err());
        opts.background = "solid:#fffaf1:4096:4096".into();
        assert!(validate_options(&opts).is_err());
        opts = options();
        opts.text = "星".repeat(MAX_TEXT + 1);
        assert!(validate_options(&opts).is_err());
        opts = options();
        opts.padding = 240;
        opts.font_size = 120;
        assert!(validate_options(&opts).is_err());
    }
    #[test]
    fn failed_hash_or_size_never_creates_a_font() {
        let scratch = Scratch::new();
        let font = &catalog()[0];
        assert!(install_font(&scratch.0, &font.id, b"partial".to_vec()).is_err());
        assert!(install_font(&scratch.0, &font.id, vec![0; font.bytes as usize]).is_err());
        assert!(!scratch.0.join("fonts").exists());
    }
    #[test]
    fn atomic_install_writes_only_selected_id_and_preserves_valid_existing_file() {
        let scratch = Scratch::new();
        let bytes = b"original synthetic fixture";
        let mut fixture = catalog()[0].clone();
        fixture.bytes = bytes.len() as u64;
        fixture.sha256 = format!("{:x}", Sha256::digest(bytes));
        write_font(&scratch.0, &fixture, bytes).unwrap();
        let installed = font_path(&scratch.0, &fixture);
        assert_eq!(fs::read(&installed).unwrap(), bytes);
        assert_eq!(fs::read_dir(scratch.0.join("fonts")).unwrap().count(), 1);
        assert!(!font_path(&scratch.0, &catalog()[1]).exists());
        let modified = fs::metadata(&installed).unwrap().modified().unwrap();
        write_font(&scratch.0, &fixture, bytes).unwrap();
        assert_eq!(
            fs::metadata(&installed).unwrap().modified().unwrap(),
            modified
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlink_font_directory_is_rejected() {
        let scratch = Scratch::new();
        let outside = Scratch::new();
        std::os::unix::fs::symlink(&outside.0, scratch.0.join("fonts")).unwrap();
        let bytes = b"original synthetic fixture";
        let mut fixture = catalog()[0].clone();
        fixture.bytes = bytes.len() as u64;
        fixture.sha256 = format!("{:x}", Sha256::digest(bytes));
        assert!(write_font(&scratch.0, &fixture, bytes).is_err());
        assert_eq!(fs::read_dir(&outside.0).unwrap().count(), 0);
    }
    #[test]
    #[ignore = "Explicit live official font download check; isolated data, original text only"]
    fn official_fonts_download_then_render_offline() {
        let scratch = Scratch::new();
        let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("../artifacts/handwriting-fonts");
        fs::create_dir_all(&output).unwrap();
        for (index, font) in catalog().iter().enumerate() {
            let downloaded = download_font(&scratch.0, &font.id).unwrap();
            assert_eq!(
                downloaded
                    .fonts
                    .iter()
                    .filter(|item| item.downloaded)
                    .count(),
                index + 1
            );
            let mut opts = options();
            opts.font = font.id.clone();
            // render() is purely local: no download client or network fallback.
            let png = base64::engine::general_purpose::STANDARD
                .decode(render(&scratch.0, opts).unwrap())
                .unwrap();
            let image = image::load_from_memory(&png).unwrap().to_rgba8();
            assert_eq!(image.dimensions(), (600, 800));
            assert!(image
                .pixels()
                .any(|pixel| *pixel != image::Rgba([255, 250, 241, 255])));
            let path = output.join(format!("{}-native.png", font.id));
            fs::write(&path, png).unwrap();
            println!(
                "Verified official download and offline render: {}",
                path.display()
            );
        }
    }

    #[test]
    #[ignore = "Requires explicit catalog fixture fonts in ignored artifacts; never downloads during test"]
    fn downloaded_fonts_render_offline_with_original_text() {
        let scratch = Scratch::new();
        for font in catalog() {
            let bytes = fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../artifacts/handwriting-fonts")
                    .join(&font.file),
            )
            .expect("Download the explicit test fixtures first");
            let face = rustybuzz::Face::from_slice(&bytes, 0).unwrap();
            for character in "星光落在纸上愿每天都有小美好".chars() {
                assert!(
                    face.glyph_index(character).is_some(),
                    "{} lacks {character}",
                    font.id
                );
            }
            install_font(&scratch.0, &font.id, bytes).unwrap();
            let mut opts = options();
            opts.font = font.id.clone();
            let encoded = render(&scratch.0, opts).unwrap();
            let png = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap();
            let image = image::load_from_memory(&png).unwrap().to_rgba8();
            assert_eq!(image.dimensions(), (600, 800));
            assert!(image
                .pixels()
                .any(|pixel| *pixel != image::Rgba([255, 250, 241, 255])));
            fs::write(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../artifacts/handwriting-fonts")
                    .join(format!("{}-native.png", font.id)),
                png,
            )
            .unwrap();
            assert!(
                assets(&scratch.0)
                    .fonts
                    .iter()
                    .find(|item| item.id == font.id)
                    .unwrap()
                    .downloaded
            );
        }
    }
}
