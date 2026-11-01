//! CPU text shaping adapted from HandwritingGenerator/rust/src/rendering.rs.
//! Copyright (c) 2023 CodeFly. MIT; see public/handwriting-fonts/RENDERER-LICENSE.txt.
//! No font is embedded and all transient glyph/layout data is dropped after rendering.
use crate::store::Result;
use fontdue::{Font, FontSettings, Metrics};
use image::{Rgba, RgbaImage};
use std::collections::{BTreeMap, HashMap, HashSet};
use unicode_segmentation::UnicodeSegmentation;

struct Layout {
    font_size: u32,
    font_spacing: f32,
    x: u32,
    y: u32,
    rect_width: u32,
    rect_height: u32,
}

pub(super) fn render(
    bytes: &[u8],
    text: &str,
    font_size: u32,
    padding: u32,
    dimensions: (u32, u32),
    paper: [u8; 4],
    ink: [u8; 4],
) -> Result<RgbaImage> {
    render_on_paper(
        bytes,
        text,
        font_size,
        padding,
        RgbaImage::from_pixel(dimensions.0, dimensions.1, Rgba(paper)),
        [0; 4],
        ink,
    )
}

pub(super) fn render_on_paper(
    bytes: &[u8],
    text: &str,
    font_size: u32,
    padding: u32,
    mut image: RgbaImage,
    insets: [u32; 4],
    ink: [u8; 4],
) -> Result<RgbaImage> {
    let [left, top, right, bottom] = insets;
    let width = image.width().saturating_sub(left + right + padding * 2);
    let height = image.height().saturating_sub(top + bottom + padding * 2);
    if width < font_size * 2 || height < font_size * 2 {
        return Err("纸张可书写区域过小，请减小留白或字号。".into());
    }
    let font = Font::from_bytes(bytes, FontSettings::default())
        .map_err(|_| "字体文件无法解析，请重新下载。")?;
    let face = rustybuzz::Face::from_slice(bytes, 0).ok_or("字体文件无法解析，请重新下载。")?;
    let config = Layout {
        font_size,
        font_spacing: 0.1,
        x: left + padding,
        y: top + padding,
        rect_width: width,
        rect_height: height,
    };
    let px = font_size as f32 * 96.0 / 72.0;
    let metrics = font
        .horizontal_line_metrics(px)
        .ok_or("字体缺少横排度量。")?;
    let mut baseline = config.y as f32 + metrics.ascent;
    let mut bitmaps = HashMap::new();
    let normalized = text
        .replace("\r\n", "\n")
        .replace(['\r', '\u{2028}', '\u{2029}'], "\n")
        .replace('\t', "    ");
    'paragraphs: for paragraph in normalized.split('\n') {
        for line in wrap_lines(
            paragraph,
            &face,
            px,
            config.font_spacing,
            config.rect_width as f32,
        ) {
            if baseline - metrics.ascent >= (config.y + config.rect_height) as f32 {
                break 'paragraphs;
            }
            draw_line(
                &mut image,
                &font,
                &face,
                &line,
                baseline,
                &config,
                ink,
                &mut bitmaps,
            );
            baseline += metrics.new_line_size * 1.4;
        }
    }
    Ok(image)
}

fn shape(text: &str, face: &rustybuzz::Face<'_>, letter_spacing: f32) -> rustybuzz::GlyphBuffer {
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    // Letter spacing applies between grapheme clusters, so discretionary
    // ligatures must not swallow their spacing slots.
    let features = if letter_spacing != 0.0 {
        vec!["liga=0".parse().unwrap(), "clig=0".parse().unwrap()]
    } else {
        Vec::new()
    };
    rustybuzz::shape(face, &features, buffer)
}

/// Wrap at Unicode word boundaries, falling back to complete grapheme clusters
/// for a word wider than the region. Whitespace remains part of the line.
fn wrap_lines(
    text: &str,
    face: &rustybuzz::Face<'_>,
    px: f32,
    spacing: f32,
    width: f32,
) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let shaped = shape(text, face, spacing);
    let scale = px / face.units_per_em() as f32;
    let mut advance = BTreeMap::<usize, f32>::new();
    for (info, position) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        *advance.entry(info.cluster as usize).or_default() += position.x_advance as f32 * scale;
    }
    let units: Vec<(usize, usize, f32)> = text
        .grapheme_indices(true)
        .map(|(start, grapheme)| {
            let end = start + grapheme.len();
            let advance = advance
                .range(start..end)
                .map(|(_, advance)| *advance)
                .sum::<f32>();
            (start, end, advance + spacing * px)
        })
        .collect();
    let breaks: HashSet<usize> = unicode_linebreak::linebreaks(text)
        .map(|(index, _)| index)
        .collect();
    let mut lines = Vec::new();
    let mut start = 0;
    while start < units.len() {
        let mut pen = 0.0;
        let mut cursor = start;
        let mut last_break = None;
        while cursor < units.len() {
            let next = pen + units[cursor].2;
            // Keep at least one cluster so even a very narrow region advances.
            if next - spacing * px > width && cursor > start {
                break;
            }
            pen = next;
            cursor += 1;
            if breaks.contains(&units[cursor - 1].1) {
                last_break = Some(cursor);
            }
        }
        if cursor < units.len() {
            cursor = last_break.filter(|end| *end > start).unwrap_or(cursor);
        }
        lines.push(text[units[start].0..units[cursor - 1].1].to_owned());
        start = cursor;
    }
    lines
}

type BitmapCache = HashMap<u16, (Metrics, Vec<u8>)>;

#[allow(clippy::too_many_arguments)]
fn draw_line(
    image: &mut RgbaImage,
    font: &Font,
    face: &rustybuzz::Face<'_>,
    text: &str,
    baseline: f32,
    config: &Layout,
    color: [u8; 4],
    cache: &mut BitmapCache,
) {
    let px = config.font_size as f32 * 96.0 / 72.0;
    let scale = px / face.units_per_em() as f32;
    let shaped = shape(text, face, config.font_spacing);
    let mut pen = config.x as f32;
    let mut previous_cluster = None;
    for (info, position) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        if previous_cluster.is_some_and(|cluster| cluster != info.cluster) {
            pen += config.font_spacing * px;
        }
        previous_cluster = Some(info.cluster);
        let glyph_id = info.glyph_id as u16;
        // Bound the transient glyph cache when rendering very diverse text.
        if cache.len() >= 512 && !cache.contains_key(&glyph_id) {
            cache.clear();
        }
        let (metrics, bitmap) = cache
            .entry(glyph_id)
            .or_insert_with(|| font.rasterize_indexed(glyph_id, px));
        let left = (pen + position.x_offset as f32 * scale).round() as i32 + metrics.xmin;
        let top = (baseline - position.y_offset as f32 * scale).round() as i32
            - metrics.ymin
            - metrics.height as i32;
        for row in 0..metrics.height {
            let y = top + row as i32;
            if y < config.y as i32
                || y >= (config.y + config.rect_height).min(image.height()) as i32
            {
                continue;
            }
            for column in 0..metrics.width {
                let coverage = bitmap[row * metrics.width + column];
                if coverage == 0 || color[3] == 0 {
                    continue;
                }
                let x = left + column as i32;
                if x < config.x as i32
                    || x >= (config.x + config.rect_width).min(image.width()) as i32
                {
                    continue;
                }
                blend(image.get_pixel_mut(x as u32, y as u32), color, coverage);
            }
        }
        pen += position.x_advance as f32 * scale;
    }
}

fn blend(destination: &mut Rgba<u8>, source: [u8; 4], coverage: u8) {
    let alpha = f32::from(source[3]) / 255.0 * f32::from(coverage) / 255.0;
    let old_alpha = f32::from(destination[3]) / 255.0;
    let combined = alpha + old_alpha * (1.0 - alpha);
    if combined == 0.0 {
        return;
    }
    for channel in 0..3 {
        destination[channel] = ((f32::from(source[channel]) * alpha
            + f32::from(destination[channel]) * old_alpha * (1.0 - alpha))
            / combined)
            .round() as u8;
    }
    destination[3] = (combined * 255.0).round() as u8;
}
