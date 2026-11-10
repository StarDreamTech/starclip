//! Bounded, local PNG preparation. No clipboard or filesystem access happens here.
use crate::store::Result;
use image::{
    codecs::png::{PngDecoder, PngEncoder},
    DynamicImage, ImageBuffer, ImageDecoder, ImageEncoder, Limits, Rgba,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};

pub const MAX_PIXELS: u64 = 16_000_000;
pub const MAX_PNG_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_RGBA_BYTES: usize = 64 * 1024 * 1024;
const MAX_EDGE: u32 = 16_000;
const THUMBNAIL_EDGE: u32 = 320;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageMetadata {
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

pub struct PreparedImage {
    pub(crate) png: Vec<u8>,
    pub(crate) thumbnail: Vec<u8>,
    metadata: ImageMetadata,
    digest: String,
}
impl PreparedImage {
    pub fn metadata(&self) -> &ImageMetadata {
        &self.metadata
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// The common path compares lossless PNG bytes without allocating decoded pixels.
    /// Different encodings may still represent the same image, so verify RGBA then.
    pub(crate) fn matches_png(&self, width: u32, height: u32, png: &[u8]) -> Result<bool> {
        if (width, height) != (self.metadata.width, self.metadata.height) {
            return Ok(false);
        }
        if self.png == png {
            return Ok(true);
        }
        let (stored_width, stored_height, stored) = decode_png(png)?;
        if (stored_width, stored_height) != (width, height) {
            return Err("图片数据与尺寸不符，原数据未更改。".into());
        }
        let (_, _, incoming) = decode_png(&self.png)?;
        Ok(incoming.as_slice() == stored.as_slice())
    }

    #[cfg(test)]
    pub(crate) fn force_test_digest(&mut self, digest: &str) {
        self.digest = digest.into();
    }
}

fn rgba_length(width: u32, height: u32) -> Result<usize> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0
        || height == 0
        || width > MAX_EDGE
        || height > MAX_EDGE
        || pixels > MAX_PIXELS
        || pixels * 4 > MAX_RGBA_BYTES as u64
    {
        return Err("图片尺寸超过限制：最多 1600 万像素、边长 16000，解码数据最多 64 MiB。".into());
    }
    Ok((pixels * 4) as usize)
}

struct PngBuffer(Vec<u8>);
impl Write for PngBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_PNG_BYTES {
            return Err(std::io::Error::other("PNG 图片不能超过 20 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let mut output = PngBuffer(Vec::new());
    PngEncoder::new(&mut output)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|error| format!("无法编码 PNG 图片：{error}"))?;
    Ok(output.0)
}

pub fn fingerprint_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<String> {
    if rgba.len() != rgba_length(width, height)? {
        return Err("图片像素长度与尺寸不符。".into());
    }
    let mut hash = Sha256::new();
    hash.update(width.to_le_bytes());
    hash.update(height.to_le_bytes());
    hash.update(rgba);
    Ok(format!("sha256:{:x}", hash.finalize()))
}

pub fn prepare_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<PreparedImage> {
    let digest = fingerprint_rgba(width, height, rgba)?;
    let png = encode_rgba(width, height, rgba)?;
    let source = ImageBuffer::<Rgba<u8>, &[u8]>::from_raw(width, height, rgba)
        .ok_or("图片像素长度与尺寸不符。")?;
    let thumbnail = if width <= THUMBNAIL_EDGE && height <= THUMBNAIL_EDGE {
        png.clone()
    } else {
        let longest = width.max(height);
        let thumbnail = image::imageops::thumbnail(
            &source,
            (width * THUMBNAIL_EDGE / longest).max(1),
            (height * THUMBNAIL_EDGE / longest).max(1),
        );
        encode_rgba(thumbnail.width(), thumbnail.height(), thumbnail.as_raw())?
    };
    Ok(PreparedImage {
        metadata: ImageMetadata {
            width,
            height,
            bytes: png.len() as u64,
        },
        png,
        thumbnail,
        digest,
    })
}

pub fn decode_png(png: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    if png.len() > MAX_PNG_BYTES
        || png.len() < 45
        || !png.starts_with(b"\x89PNG\r\n\x1a\n")
        || &png[8..16] != b"\0\0\0\rIHDR"
        || !png.ends_with(b"\0\0\0\0IEND\xaeB`\x82")
    {
        return Err("图片不是完整的 PNG，或超过 20 MiB。".into());
    }
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    rgba_length(width, height)?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(MAX_RGBA_BYTES as u64);
    let decoder = PngDecoder::with_limits(Cursor::new(png), limits)
        .map_err(|error| format!("PNG 图片损坏或超出解码限制：{error}"))?;
    if decoder.total_bytes() > MAX_RGBA_BYTES as u64
        || decoder.is_apng().map_err(|e| e.to_string())?
    {
        return Err("仅支持解码大小不超过 64 MiB 的静态 PNG 图片。".into());
    }
    let rgba = DynamicImage::from_decoder(decoder)
        .map_err(|error| format!("PNG 图片损坏：{error}"))?
        .into_rgba8()
        .into_raw();
    if rgba.len() != rgba_length(width, height)? {
        return Err("PNG 图片尺寸与像素不符。".into());
    }
    Ok((width, height, rgba))
}

/// Only local, bounded static PNG/JPEG files are accepted for clipboard collections.
pub fn prepare_file(bytes: &[u8]) -> Result<PreparedImage> {
    if bytes.len() > MAX_PNG_BYTES {
        return Err("图片文件超过 20 MiB。".into());
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return prepare_png(bytes);
    }
    if !bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Err("图集仅支持静态 PNG 或 JPEG 图片。".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Jpeg);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(MAX_RGBA_BYTES as u64);
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| "JPEG 图片损坏或超过限制。")?;
    let (width, height) = decoder.dimensions();
    rgba_length(width, height)?;
    let orientation = decoder
        .orientation()
        .map_err(|_| "JPEG 图片方向信息损坏。")?;
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(|_| "JPEG 图片无法解码。")?;
    decoded.apply_orientation(orientation);
    let rgba = decoded.into_rgba8();
    prepare_rgba(rgba.width(), rgba.height(), rgba.as_raw())
}

pub fn prepare_png(png: &[u8]) -> Result<PreparedImage> {
    let (width, height, rgba) = decode_png(png)?;
    prepare_rgba(width, height, &rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_comparison_accepts_other_encodings_and_checks_every_rgba_byte() {
        let pixels = [15, 40, 90, 255, 42, 80, 50, 0];
        let image = prepare_rgba(2, 1, &pixels).unwrap();
        let mut alternate = Vec::new();
        PngEncoder::new_with_quality(
            &mut alternate,
            image::codecs::png::CompressionType::Best,
            image::codecs::png::FilterType::NoFilter,
        )
        .write_image(&pixels, 2, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
        assert_ne!(alternate, image.png);
        assert!(image.matches_png(2, 1, &alternate).unwrap());
        assert!(image.matches_png(2, 1, &image.png).unwrap());
        for index in 0..pixels.len() {
            let mut different = pixels;
            different[index] ^= 1;
            let other = prepare_rgba(2, 1, &different).unwrap();
            assert!(!image.matches_png(2, 1, &other.png).unwrap());
        }
        assert!(!image.matches_png(1, 2, &alternate).unwrap());
        assert!(prepare_file(&image.png).is_ok());
        assert!(prepare_file(b"invalid original fixture").is_err());
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(
                &[20, 40, 60, 80, 100, 120],
                2,
                1,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        assert_eq!(prepare_file(&jpeg).unwrap().metadata().width, 2);
        let exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let mut rotated = jpeg[..2].to_vec();
        rotated.extend_from_slice(&[0xff, 0xe1]);
        rotated.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        rotated.extend_from_slice(exif);
        rotated.extend_from_slice(&jpeg[2..]);
        let rotated = prepare_file(&rotated).unwrap();
        assert_eq!(
            (rotated.metadata().width, rotated.metadata().height),
            (1, 2)
        );
    }
    #[test]
    fn png_roundtrip_thumbnail_identity_and_corruption_limits() {
        let rgba: Vec<u8> = (0..640 * 160)
            .flat_map(|i| [(i % 251) as u8, 110, 230, if i % 2 == 0 { 255 } else { 0 }])
            .collect();
        let prepared = prepare_rgba(640, 160, &rgba).unwrap();
        let (width, height, decoded) = decode_png(&prepared.png).unwrap();
        assert_eq!((width, height), (640, 160));
        assert_eq!(decoded, rgba);
        assert_eq!(
            prepare_png(&prepared.png).unwrap().digest(),
            prepared.digest()
        );
        let (width, height, _) = decode_png(&prepared.thumbnail).unwrap();
        assert_eq!((width, height), (320, 80));
        assert_ne!(
            prepare_rgba(320, 320, &rgba).unwrap().digest(),
            prepared.digest()
        );
        assert!(prepare_rgba(0, 1, &[]).is_err());
        assert!(prepare_rgba(16_001, 1, &[]).is_err());
        assert!(prepare_rgba(4001, 4000, &[]).is_err());
        assert!(prepare_rgba(1, 1, &[0, 0, 0]).is_err());
        assert!(decode_png(&prepared.png[..prepared.png.len() - 1]).is_err());
        let mut damaged = prepared.png.clone();
        damaged[29] ^= 1;
        assert!(decode_png(&damaged).is_err());
        assert!(decode_png(&vec![0; MAX_PNG_BYTES + 1]).is_err());
    }
}
