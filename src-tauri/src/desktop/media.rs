//! Local image collections and simultaneous clipboard representations.
use super::*;
use clipboard_rs::{Clipboard as MultipleFormats, ClipboardContext};
use std::io::Read;

const BUNDLE_FORMAT: &str = "org.starclip.bundle.v1";
const MAX_HTML: usize = 8 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    version: u32,
    text: String,
    files: Vec<String>,
    token: String,
    source_id: Option<i64>,
}
pub(super) struct IncomingBundle {
    pub text: String,
    pub images: Vec<images::PreparedImage>,
    pub own: Option<(String, i64)>,
}

fn image_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|v| v.to_str())
            .map(|v| v.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg")
    )
}

pub(super) fn read_file_bundle(text: String, paths: &[String]) -> Result<Option<IncomingBundle>> {
    let payload = ClipboardContext::new()
        .ok()
        .and_then(|ctx| ctx.get_buffer(BUNDLE_FORMAT).ok());
    prepare_files(paths, text, payload.as_deref())
}

/// Preparation accepts only native-offered paths, never paths supplied by custom JSON alone.
fn prepare_files(
    paths: &[String],
    text: String,
    payload: Option<&[u8]>,
) -> Result<Option<IncomingBundle>> {
    if paths.is_empty() || !paths.iter().all(|p| image_file(Path::new(p))) {
        return Ok(None);
    }
    if paths.len() > crate::image_transfer::MAX_IMAGES {
        return Err("图文组合需包含 1 至 50 张图片。".into());
    }
    let names = paths
        .iter()
        .filter_map(|p| Path::new(p).file_name().and_then(|n| n.to_str()))
        .collect::<Vec<_>>()
        .join("\n");
    let file_urls = paths
        .iter()
        .filter_map(|p| url::Url::from_file_path(p).ok().map(|u| u.to_string()))
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = if [paths.join("\n"), names, file_urls]
        .iter()
        .any(|v| v.trim() == normalized.trim())
    {
        String::new()
    } else {
        text
    };
    if text.len() > crate::store::MAX_CONTENT_BYTES {
        return Err("单条文字超过 256 KiB，未记录。".into());
    }
    let own = payload
        .filter(|p| p.len() <= 512 * 1024)
        .and_then(|p| serde_json::from_slice::<Payload>(p).ok())
        .filter(|p| p.version == 1 && p.files == paths && p.text == text)
        .and_then(|p| p.source_id.map(|id| (p.token, id)));
    if paths.len() == 1 && text.trim().is_empty() && own.is_none() {
        return Ok(None);
    }
    // Preflight the complete list before opening anything. Mixed/unavailable files stay references.
    for path in paths {
        let path = Path::new(path);
        let Ok(meta) = fs::symlink_metadata(path) else {
            return Ok(None);
        };
        if !path.is_absolute() || !meta.is_file() || meta.file_type().is_symlink() {
            return Ok(None);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Ok(None);
            }
        }
        if meta.len() > images::MAX_PNG_BYTES as u64 {
            return Err("图片文件超过 20 MiB，未收录图文组合。".into());
        }
    }
    let mut list = Vec::new();
    let mut total = 0usize;
    for path in paths {
        let mut options = fs::OpenOptions::new();
        options.read(true);
        // Do not follow a leaf link swapped in after preflight.
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(0x100); /* O_NOFOLLOW */
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(0x20000); /* O_NOFOLLOW */
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000); /* FILE_FLAG_OPEN_REPARSE_POINT */
        }
        let file = options.open(path).map_err(|_| "图片文件已不可用。")?;
        let meta = file.metadata().map_err(|_| "图片文件已不可用。")?;
        if !meta.is_file() {
            return Err("图片文件已不可用。".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("图片文件已不可用。".into());
            }
        }
        let mut bytes = Vec::new();
        file.take(images::MAX_PNG_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "无法读取图片文件。")?;
        if bytes.len() > images::MAX_PNG_BYTES {
            return Err("图片文件超过 20 MiB，未收录图文组合。".into());
        }
        let image = images::prepare_file(&bytes)?;
        total = total.saturating_add(image.metadata().bytes as usize);
        if total > crate::image_transfer::MAX_BYTES {
            return Err("所选图片总大小超过 100 MiB，请分批操作。".into());
        }
        list.push(image);
    }
    Ok(Some(IncomingBundle {
        text,
        images: list,
        own,
    }))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(super) fn write_bundle(
    context: &Context,
    paths: Vec<String>,
    text: String,
    source_id: i64,
) -> Result<()> {
    use clipboard_rs::ClipboardContent as Data;
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "无法创建图文复制标记。")?;
    let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let payload = Payload {
        version: 1,
        text: text.clone(),
        files: paths.clone(),
        token: token.clone(),
        source_id: Some(source_id),
    };
    let payload_bytes = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;
    let mut data = vec![
        Data::Files(paths.clone()),
        Data::Text(text.clone()),
        Data::Other(BUNDLE_FORMAT.into(), payload_bytes.clone()),
    ];
    let mut html = format!(
        "<div><p style=\"white-space:pre-wrap\">{}</p>",
        escape(&text)
    );
    let mut complete = true;
    for path in &paths {
        let png = fs::read(path).map_err(|_| "图片临时副本已不可用，请重试。")?;
        if html.len() + png.len().saturating_mul(4) / 3 + 100 > MAX_HTML {
            complete = false;
            break;
        }
        html.push_str("<p><img src=\"data:image/png;base64,");
        html.push_str(&base64::engine::general_purpose::STANDARD.encode(png));
        html.push_str("\" /></p>");
    }
    if complete {
        html.push_str("</div>");
        data.push(Data::Html(html));
    }
    let mut reader = lock(&context.reader)?;
    let clipboard = ClipboardContext::new().map_err(|_| "无法访问图文剪贴板。")?;
    let failure = "图文未能完整复制，请尝试分别复制文字和图片。";
    clipboard.set(data).map_err(|_| failure)?;
    // The adapter may report only the final representation's result. Verify every required one.
    let files = clipboard
        .get_files()
        .map_err(|_| failure)?
        .into_iter()
        .map(|p| {
            url::Url::parse(&p)
                .ok()
                .filter(|u| u.scheme() == "file")
                .and_then(|u| u.to_file_path().ok())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(p)
        })
        .collect::<Vec<_>>();
    if files != paths
        || clipboard.get_text().map_err(|_| failure)? != text
        || clipboard.get_buffer(BUNDLE_FORMAT).map_err(|_| failure)? != payload_bytes
    {
        return Err(failure.into());
    }
    reader.own_bundle = Some(token);
    reader.copy_image_png = None;
    reader.last_image_id = None;
    reader.last_bundle_id = None;
    // Always verify decoded collection content before treating a native read as our own copy.
    reader.capture = CaptureState::default();
    Ok(())
}

pub(super) fn copy_bundle(app: &AppHandle, context: &Context, id: i64) -> Result<()> {
    let clip = lock(&context.store)?.clip(id)?;
    if clip.kind != "bundle" {
        return Err("这条记录不是图文组合。".into());
    }
    let batch = super::image_transfer::stage_cache(app, &*lock(&context.store)?, &[id], true)?;
    if let Err(error) = write_bundle(
        context,
        batch
            .paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        clip.content,
        id,
    ) {
        if let Some(parent) = batch.paths[0].parent() {
            let _ = fs::remove_dir_all(parent);
        }
        return Err(error);
    }
    lock(&context.store)?.mark_copied(id)?;
    set_clipboard_error(app, context, None);
    changed(app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_files_keep_text_order_repeated_images_and_never_follow_custom_paths() {
        let root = std::env::temp_dir().join(format!(
            "starclip-original-media-{}-{}",
            std::process::id(),
            unix_millis()
        ));
        fs::create_dir(&root).unwrap();
        let a = images::prepare_rgba(2, 1, &[10, 40, 90, 255, 10, 80, 50, 127]).unwrap();
        let b = images::prepare_rgba(2, 1, &[20, 40, 90, 255, 20, 80, 50, 127]).unwrap();
        fs::write(root.join("original-a.png"), &a.png).unwrap();
        fs::write(root.join("original-b.png"), &b.png).unwrap();
        let paths = vec![
            root.join("original-b.png").to_string_lossy().into_owned(),
            root.join("original-a.png").to_string_lossy().into_owned(),
        ];
        let text = "Original caption <script> & newline\nsecond line";
        let bundle = prepare_files(&paths, text.into(), None).unwrap().unwrap();
        assert_eq!(bundle.text, text);
        assert_eq!(bundle.images[0].digest(), b.digest());
        assert_eq!(bundle.images[1].digest(), a.digest());
        assert!(bundle.own.is_none());
        assert!(prepare_files(&paths, paths.join("\n"), None)
            .unwrap()
            .unwrap()
            .text
            .is_empty());
        assert!(prepare_files(
            &paths,
            paths
                .iter()
                .map(|p| Path::new(p).file_name().unwrap().to_str().unwrap())
                .collect::<Vec<_>>()
                .join("\r"),
            None
        )
        .unwrap()
        .unwrap()
        .text
        .is_empty());
        assert!(prepare_files(&paths[..1], String::new(), None)
            .unwrap()
            .is_none());
        let captioned = prepare_files(&paths[..1], text.into(), None)
            .unwrap()
            .unwrap();
        assert_eq!(captioned.images.len(), 1);
        let payload = Payload {
            version: 1,
            text: text.into(),
            files: paths.clone(),
            token: "original token".into(),
            source_id: Some(20),
        };
        let bytes = serde_json::to_vec(&payload).unwrap();
        assert!(prepare_files(&paths, text.into(), Some(&bytes))
            .unwrap()
            .unwrap()
            .own
            .is_some());
        // A custom payload cannot substitute a path not offered in the native file list.
        assert!(prepare_files(
            &[paths[1].clone(), paths[0].clone()],
            text.into(),
            Some(&bytes)
        )
        .unwrap()
        .unwrap()
        .own
        .is_none());
        assert!(prepare_files(&vec![paths[0].clone(); 51], String::new(), None).is_err());
        assert!(prepare_files(
            &[
                paths[0].clone(),
                root.join("original.txt").to_string_lossy().into_owned()
            ],
            text.into(),
            None
        )
        .unwrap()
        .is_none());
        assert!(
            prepare_files(&[paths[0].clone(), paths[0].clone()], String::new(), None)
                .unwrap()
                .unwrap()
                .images
                .len()
                == 2
        );
        #[cfg(unix)]
        {
            let link = root.join("original-link.png");
            std::os::unix::fs::symlink(&paths[0], &link).unwrap();
            assert!(prepare_files(
                &[paths[0].clone(), link.to_string_lossy().into_owned()],
                text.into(),
                None
            )
            .unwrap()
            .is_none());
        }
        assert_eq!(escape("<&\">"), "&lt;&amp;&quot;&gt;");
        fs::remove_dir_all(root).unwrap();
    }
}
