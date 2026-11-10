//! Fixed product materials only. No store, user files, network or clipboard access.
use crate::{images, store::Result};
use serde::Deserialize;

pub const MAX_IMAGE_BYTES: usize = 3 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 2_000_000;

#[derive(Deserialize)]
struct Catalog {
    texts: Vec<ShareText>,
    images: Vec<ShareImage>,
}
#[derive(Deserialize)]
struct ShareText {
    id: String,
    content: String,
}
#[derive(Clone, Deserialize)]
pub struct ShareImage {
    pub id: String,
    pub locale: String,
    pub path: String,
    pub filename: String,
    width: u32,
    height: u32,
    bytes: usize,
}

fn catalog() -> Result<Catalog> {
    serde_json::from_str(include_str!("../../src/product-share-catalog.json"))
        .map_err(|_| "分享素材配置无效，请重新安装星笺。".into())
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
pub fn text(id: &str) -> Result<String> {
    if !valid_id(id) {
        return Err("未找到这份分享文案。".into());
    }
    let text = catalog()?
        .texts
        .into_iter()
        .find(|text| text.id == id)
        .ok_or("未找到这份分享文案。")?;
    if text.content.trim().is_empty() || text.content.len() > 16 * 1024 {
        return Err("分享文案为空或过长。".into());
    }
    Ok(text.content)
}
pub fn image(id: &str) -> Result<ShareImage> {
    if !valid_id(id) {
        return Err("未找到这张产品介绍图。".into());
    }
    let image = catalog()?
        .images
        .into_iter()
        .find(|image| image.id == id)
        .ok_or("未找到这张产品介绍图。")?;
    validate_image(&image)?;
    Ok(image)
}
pub fn images_for_locale(locale: &str) -> Result<Vec<ShareImage>> {
    if !matches!(locale, "zh-CN" | "en-US") {
        return Err("分享素材语言无效。".into());
    }
    let images: Vec<_> = catalog()?
        .images
        .into_iter()
        .filter(|i| i.locale == locale)
        .collect();
    if images.is_empty() || images.len() > crate::image_transfer::MAX_IMAGES {
        return Err("分享图片目录无效。".into());
    }
    for image in &images {
        validate_image(image)?;
    }
    Ok(images)
}
fn validate_image(image: &ShareImage) -> Result<()> {
    if !valid_id(&image.id)
        || image.path != format!("share/{}.png", image.id)
        || !image.filename.ends_with(".png")
        || image.filename.len() > 200
        || image.filename.len() <= 4
        || image
            .filename
            .contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        || image.filename.chars().any(char::is_control)
        || image.width == 0
        || image.height == 0
        || u64::from(image.width) * u64::from(image.height) > MAX_IMAGE_PIXELS
        || image.bytes == 0
        || image.bytes > MAX_IMAGE_BYTES
    {
        return Err("产品介绍图配置无效或超过大小限制。".into());
    }
    Ok(())
}
pub fn decode_image(image: &ShareImage, png: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    validate_image(image)?;
    if png.len() != image.bytes || png.len() > MAX_IMAGE_BYTES {
        return Err("产品介绍图缺失或大小不符，请重新安装星笺。".into());
    }
    let pixels = images::decode_png(png)?;
    if (pixels.0, pixels.1) != (image.width, image.height) {
        return Err("产品介绍图尺寸与配置不符。".into());
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn product_catalog_only_resolves_fixed_materials() {
        let catalog = catalog().unwrap();
        assert!(!catalog.texts.is_empty() && !catalog.images.is_empty());
        let mut ids = std::collections::HashSet::new();
        for item in &catalog.texts {
            assert!(ids.insert(&item.id));
            assert_eq!(text(&item.id).unwrap(), item.content);
        }
        ids.clear();
        for item in &catalog.images {
            assert!(ids.insert(&item.id));
            let selected = image(&item.id).unwrap();
            assert_eq!(selected.path, item.path);
            let png = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../public")
                    .join(&selected.path),
            )
            .unwrap();
            decode_image(&selected, &png).unwrap();
        }
        for id in [
            "",
            "../overview",
            "/etc/passwd",
            "https://example.com/a.png",
            "UNKNOWN",
            "unknown-material",
        ] {
            assert!(text(id).is_err());
            assert!(image(id).is_err());
        }
    }
    #[test]
    fn product_image_validation_rejects_wrong_bytes_dimensions_and_paths() {
        let prepared = images::prepare_rgba(2, 2, &[255; 16]).unwrap();
        let mut image = ShareImage {
            id: "fixture".into(),
            locale: "zh-CN".into(),
            path: "share/fixture.png".into(),
            filename: "starclip-fixture.png".into(),
            width: 2,
            height: 2,
            bytes: prepared.png.len(),
        };
        assert_eq!(
            decode_image(&image, &prepared.png).unwrap().2,
            vec![255; 16]
        );
        let mut invalid = prepared.png.clone();
        invalid[0] = 0;
        assert!(decode_image(&image, &invalid).is_err());
        assert!(decode_image(&image, &prepared.png[..prepared.png.len() - 1]).is_err());
        image.width = 3;
        assert!(decode_image(&image, &prepared.png).is_err());
        image.width = 2;
        for path in [
            "../fixture.png",
            "share/../../fixture.png",
            "https://example.com/fixture.png",
        ] {
            image.path = path.into();
            assert!(validate_image(&image).is_err());
        }
        image.path = "share/fixture.png".into();
        image.filename = "../fixture.png".into();
        assert!(validate_image(&image).is_err());
        image.filename = "fixture.png".into();
        image.width = 2001;
        image.height = 1000;
        assert!(validate_image(&image).is_err());
        image.width = 2;
        image.height = 2;
        image.bytes = MAX_IMAGE_BYTES + 1;
        assert!(validate_image(&image).is_err());
    }
}
