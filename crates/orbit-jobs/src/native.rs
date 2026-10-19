//! The engine that is not a process.
//!
//! `orbit-native` is the one engine that can always run, because it is compiled
//! into Orbit: no sidecar to install, no `PATH` to get wrong, no version to
//! disagree with. That also makes it the fallback of last resort, so it has to
//! be *predictable* rather than clever.
//!
//! What it does is deliberately a small, honest set:
//!
//! * **Images**: decode, transform (resize, crop, rotate, flip, grayscale) and
//!   re-encode to PNG, JPEG or BMP. Writing a format is what the registry's
//!   `writers` lists gate, so this engine is only ever asked for the three it
//!   declares.
//! * **Text**: Markdown or plain text to HTML, for the subset a dropped note
//!   actually uses.
//! * **Data**: delimiter-separated text to CSV or JSON, and JSON back to CSV.
//!
//! Everything else returns an error naming the milestone, and the registry data
//! agrees: `engines.json` lists exactly these capabilities, so a job that needs
//! something else is never routed here in the first place.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};

use crate::plan::{NativeOp, Resize, Transform};

/// How images are resampled when they change size.
const FILTER: FilterType = FilterType::Lanczos3;

/// JPEG encoder quality when the options do not ask for one.
const DEFAULT_JPEG_QUALITY: u8 = 82;

/// Colour behind transparency when the target has no alpha channel.
const WHITE: [u8; 3] = [255, 255, 255];

/// Why a built-in operation could not finish.
#[derive(Debug, thiserror::Error)]
pub enum NativeError {
    /// The source could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// File that failed.
        path: String,
        /// Underlying error.
        source: io::Error,
    },
    /// The result could not be written.
    #[error("cannot write {path}: {source}")]
    Write {
        /// File that failed.
        path: String,
        /// Underlying error.
        source: io::Error,
    },
    /// The source is not an image this build can decode.
    #[error("{path} is not an image Orbit can read: {source}")]
    Decode {
        /// File that failed.
        path: String,
        /// Underlying decoder error.
        source: image::ImageError,
    },
    /// The target format is outside what this engine declares.
    #[error("the built-in engine cannot write .{0} yet")]
    UnsupportedTarget(String),
    /// The input is well-formed but not usable for the requested conversion.
    #[error("{0}")]
    Data(String),
}

/// Run a built-in operation, writing the result to `dest`.
///
/// `dest` is the plan's scratch file, which the caller publishes on success and
/// removes on failure. Nothing here touches the destination name.
///
/// # Errors
///
/// Fails when the source cannot be read or decoded, when the transform cannot be
/// applied to it, or when the result cannot be written.
pub fn run(op: &NativeOp, dest: &Path) -> Result<(), NativeError> {
    match op {
        NativeOp::Image {
            source,
            target_ext,
            transform,
            quality,
            background,
        } => image(
            source,
            dest,
            target_ext,
            transform,
            *quality,
            background.as_deref(),
        ),
        NativeOp::Markup { source } => markup(source, dest),
        NativeOp::Tabular {
            source,
            target_ext,
            delimiter,
            header,
        } => tabular(source, dest, target_ext, *delimiter, *header),
    }
}

// ------------------------------------------------------------------- images

fn image(
    source: &Path,
    dest: &Path,
    target_ext: &str,
    transform: &Transform,
    quality: Option<u8>,
    background: Option<&str>,
) -> Result<(), NativeError> {
    let picture = image::open(source).map_err(|error| NativeError::Decode {
        path: source.display().to_string(),
        source: error,
    })?;
    let picture = apply(picture, transform)?;
    save(picture, dest, target_ext, quality, background)
}

/// Apply the one transform the ring asked for.
///
/// A crop that misses the picture entirely is an error rather than a panic:
/// the region comes from a text field, and a wrong number should produce a
/// message, not a crash in a worker thread.
fn apply(picture: DynamicImage, transform: &Transform) -> Result<DynamicImage, NativeError> {
    Ok(match transform {
        Transform::None => picture,
        Transform::Grayscale => picture.grayscale(),
        Transform::Resize(resize) => match *resize {
            Resize::Percent(percent) => {
                let width = scaled(picture.width(), percent);
                let height = scaled(picture.height(), percent);
                picture.resize(width, height, FILTER)
            }
            // `resize` fits inside the box and keeps the aspect ratio, which is
            // what every other engine's geometry means here too.
            Resize::Fit(side) => picture.resize(side, side, FILTER),
            Resize::Box { width, height } => picture.resize(width, height, FILTER),
        },
        Transform::Crop {
            x,
            y,
            width,
            height,
        } => {
            if *width == 0 || *height == 0 {
                return Err(NativeError::Data("the crop region has no area".to_owned()));
            }
            if *x >= picture.width() || *y >= picture.height() {
                return Err(NativeError::Data(format!(
                    "the crop starts at {x},{y}, outside a {}x{} image",
                    picture.width(),
                    picture.height()
                )));
            }
            // The remainder is clamped, so a region that runs off the edge
            // gives the overlap rather than an error.
            let width = (*width).min(picture.width() - *x);
            let height = (*height).min(picture.height() - *y);
            picture.crop_imm(*x, *y, width, height)
        }
        Transform::Rotate { degrees } => match degrees {
            180 => picture.rotate180(),
            270 => picture.rotate270(),
            _ => picture.rotate90(),
        },
        Transform::Flip { vertical } => {
            if *vertical {
                picture.flipv()
            } else {
                picture.fliph()
            }
        }
    })
}

/// One dimension at a percentage of the original, never zero.
fn scaled(side: u32, percent: u32) -> u32 {
    let scaled = u64::from(side) * u64::from(percent) / 100;
    u32::try_from(scaled).unwrap_or(u32::MAX).max(1)
}

/// Write the picture, honouring quality where the format has it.
fn save(
    picture: DynamicImage,
    dest: &Path,
    target_ext: &str,
    quality: Option<u8>,
    background: Option<&str>,
) -> Result<(), NativeError> {
    let format = ImageFormat::from_extension(target_ext).filter(|format| {
        matches!(
            format,
            ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Bmp
        )
    });
    let Some(format) = format else {
        return Err(NativeError::UnsupportedTarget(target_ext.to_owned()));
    };

    let file = File::create(dest).map_err(|error| NativeError::Write {
        path: dest.display().to_string(),
        source: error,
    })?;
    let mut writer = BufWriter::new(file);

    let behind = background.map_or(WHITE, parse_colour);
    let result = if format == ImageFormat::Jpeg {
        // JPEG has no alpha, so anything transparent is flattened onto the
        // background rather than coming out black.
        let mut encoder = JpegEncoder::new_with_quality(
            &mut writer,
            quality.unwrap_or(DEFAULT_JPEG_QUALITY).clamp(1, 100),
        );
        encoder.encode_image(&DynamicImage::ImageRgb8(flatten(&picture, behind)))
    } else if format == ImageFormat::Bmp {
        // BMP has no alpha either. PNG keeps the picture exactly as it is, in
        // colour or in the one colour a grayscale operation left.
        let flat = if picture.color().has_alpha() {
            DynamicImage::ImageRgb8(flatten(&picture, behind))
        } else {
            picture
        };
        flat.write_to(&mut writer, format)
    } else {
        picture.write_to(&mut writer, format)
    };

    result.map_err(|error| NativeError::Write {
        path: dest.display().to_string(),
        source: io::Error::other(error.to_string()),
    })?;
    writer.flush().map_err(|error| NativeError::Write {
        path: dest.display().to_string(),
        source: error,
    })
}

/// `#rrggbb` or `#rgb`, white when the text is not one.
///
/// The colour comes from a free-text field, so unrecognised text has to mean
/// something rather than failing a conversion over a spelling.
fn parse_colour(text: &str) -> [u8; 3] {
    let hex = text.trim().trim_start_matches('#');
    let channel = |slice: &str| u8::from_str_radix(slice, 16).ok();
    match hex.len() {
        6 => match (
            channel(&hex[0..2]),
            channel(&hex[2..4]),
            channel(&hex[4..6]),
        ) {
            (Some(red), Some(green), Some(blue)) => [red, green, blue],
            _ => WHITE,
        },
        3 => {
            // `#abc` is `#aabbcc`; 15 * 17 is 255, so a single nibble cannot
            // overflow a channel.
            let digits: Vec<u8> = hex
                .chars()
                .filter_map(|digit| digit.to_digit(16))
                .filter_map(|value| u8::try_from(value).ok())
                .collect();
            match digits[..] {
                [red, green, blue] => [red * 17, green * 17, blue * 17],
                _ => WHITE,
            }
        }
        _ => WHITE,
    }
}

/// Composite an image with alpha onto an opaque colour.
fn flatten(picture: &DynamicImage, background: [u8; 3]) -> RgbImage {
    let rgba = picture.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut opaque = RgbImage::new(width, height);
    for (x, y, pixel) in rgba.enumerate_pixels() {
        let [red, green, blue, alpha] = pixel.0;
        let mix = |channel: u8, behind: u8| -> u8 {
            let alpha = u16::from(alpha);
            let value = u16::from(channel)
                .saturating_mul(alpha)
                .saturating_add(u16::from(behind).saturating_mul(255 - alpha));
            u8::try_from(value / 255).unwrap_or(255)
        };
        opaque.put_pixel(
            x,
            y,
            Rgb([
                mix(red, background[0]),
                mix(green, background[1]),
                mix(blue, background[2]),
            ]),
        );
    }
    opaque
}

// ------------------------------------------------------------------- markup

fn markup(source: &Path, dest: &Path) -> Result<(), NativeError> {
    let text = read(source)?;
    write(dest, &render_markdown(&text))
}

/// Markdown, or plain text, as a minimal HTML document.
///
/// The subset is the one a note dropped on the ring is written in: headings,
/// paragraphs, bullets, numbered lists, block quotes, fenced code, bold,
/// italics, inline code and links. Anything richer — tables, footnotes,
/// embedded HTML — goes through Pandoc when it is installed, and is not faked
/// here.
fn render_markdown(text: &str) -> String {
    let mut html = String::from(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n</head>\n<body>\n",
    );
    let mut paragraph = String::new();
    let mut list: Option<char> = None;
    let mut code = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            close_paragraph(&mut html, &mut paragraph);
            close_list(&mut html, &mut list);
            html.push_str(if code {
                "</code></pre>\n"
            } else {
                "<pre><code>"
            });
            code = !code;
            continue;
        }
        if code {
            html.push_str(&escape(line));
            html.push('\n');
            continue;
        }
        if trimmed.is_empty() {
            close_paragraph(&mut html, &mut paragraph);
            close_list(&mut html, &mut list);
            continue;
        }
        if let Some((level, rest)) = heading(trimmed) {
            close_paragraph(&mut html, &mut paragraph);
            close_list(&mut html, &mut list);
            writeln!(html, "<h{level}>{}</h{level}>", inline(rest)).ok();
            continue;
        }
        if let Some((marker, rest)) = list_item(trimmed) {
            close_paragraph(&mut html, &mut paragraph);
            if list != Some(marker) {
                close_list(&mut html, &mut list);
                html.push_str(if marker == 'o' { "<ol>\n" } else { "<ul>\n" });
                list = Some(marker);
            }
            writeln!(html, "<li>{}</li>", inline(rest)).ok();
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("> ") {
            close_paragraph(&mut html, &mut paragraph);
            close_list(&mut html, &mut list);
            writeln!(html, "<blockquote>{}</blockquote>", inline(rest)).ok();
            continue;
        }
        close_list(&mut html, &mut list);
        // Consecutive lines are one paragraph, as Markdown reads them.
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(&inline(trimmed));
    }

    close_paragraph(&mut html, &mut paragraph);
    close_list(&mut html, &mut list);
    if code {
        html.push_str("</code></pre>\n");
    }
    html.push_str("</body>\n</html>\n");
    html
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    let rest = line.get(hashes..)?.trim_start();
    (1..=6).contains(&hashes).then_some((hashes, rest))
}

/// `- item`, `* item`, `+ item` or `1. item`.
fn list_item(line: &str) -> Option<(char, &str)> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(('u', rest));
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        if let Some(rest) = line.get(digits..).and_then(|rest| rest.strip_prefix(". ")) {
            return Some(('o', rest));
        }
    }
    None
}

fn close_paragraph(html: &mut String, paragraph: &mut String) {
    if !paragraph.is_empty() {
        html.push_str("<p>");
        html.push_str(paragraph);
        html.push_str("</p>\n");
        paragraph.clear();
    }
}

fn close_list(html: &mut String, list: &mut Option<char>) {
    if let Some(marker) = list.take() {
        html.push_str(if marker == 'o' { "</ol>\n" } else { "</ul>\n" });
    }
}

/// Inline markup within one block: code spans, links, bold, italics.
///
/// The text is escaped first, so a note containing `<script>` becomes text
/// rather than markup, and the inline markers survive because escaping does not
/// touch them.
fn inline(text: &str) -> String {
    let escaped = escape(text);
    let mut out = String::new();
    for (index, segment) in escaped.split('`').enumerate() {
        if index % 2 == 1 {
            out.push_str("<code>");
            out.push_str(segment);
            out.push_str("</code>");
        } else {
            out.push_str(&emphasis(&links(segment)));
        }
    }
    out
}

/// `[label](url)`, but only for URLs that are safe to put in a document.
fn links(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let Some(label_end) = rest[start..].find("](") else {
            break;
        };
        let Some(url_end) = rest[start + label_end..].find(')') else {
            break;
        };
        let label = &rest[start + 1..start + label_end];
        let url = &rest[start + label_end + 2..start + label_end + url_end];
        out.push_str(&rest[..start]);
        if ["http://", "https://", "mailto:"]
            .iter()
            .any(|scheme| url.starts_with(scheme))
        {
            let _ = write!(out, "<a href=\"{url}\">{label}</a>");
        } else {
            // `javascript:` and `data:` are how a link becomes a payload, so
            // the label stays and the target goes.
            out.push_str(label);
        }
        rest = &rest[start + label_end + url_end + 1..];
    }
    out.push_str(rest);
    out
}

/// `**bold**` and `*italic*`.
fn emphasis(text: &str) -> String {
    paired(
        &paired(text, "**", "<strong>", "</strong>"),
        "*",
        "<em>",
        "</em>",
    )
}

/// Wrap every second occurrence of `marker`, so an odd count leaves valid HTML.
///
/// A marker only counts when it hugs its text: `1 * 2` is arithmetic, not
/// emphasis, and an unmatched marker stays literal rather than swallowing the
/// rest of the document.
fn paired(text: &str, marker: &str, open: &str, close: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    let mut inside = false;
    while let Some(found) = text[cursor..].find(marker) {
        let start = cursor + found;
        let end = start + marker.len();
        let usable = if inside {
            text[..start]
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_whitespace())
        } else {
            text[end..]
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace())
        };
        if usable {
            out.push_str(&text[cursor..start]);
            out.push_str(if inside { close } else { open });
            inside = !inside;
            cursor = end;
        } else {
            out.push_str(&text[cursor..end]);
            cursor = end;
        }
    }
    out.push_str(&text[cursor..]);
    if inside {
        out.push_str(close);
    }
    out
}

/// The only characters that can change what a browser does with the text.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

// ------------------------------------------------------------------ tabular

fn tabular(
    source: &Path,
    dest: &Path,
    target_ext: &str,
    delimiter: char,
    header: bool,
) -> Result<(), NativeError> {
    let text = read(source)?;
    let rows = if extension(source) == "json" {
        rows_from_json(&text).map_err(|message| {
            NativeError::Data(format!("{} is not a table: {message}", source.display()))
        })?
    } else {
        rows_from_delimited(&text, delimiter)
    };

    let output = match target_ext {
        "csv" => rows_to_delimited(&rows, ','),
        "json" => rows_to_json(&rows, header),
        other => return Err(NativeError::UnsupportedTarget(other.to_owned())),
    };
    write(dest, &output)
}

/// Split delimiter-separated text into rows, following RFC 4180 quoting.
///
/// Hand-written rather than pulled in, because the rule is small and the
/// alternative is a parser with a licence and a security surface.
fn rows_from_delimited(text: &str, delimiter: char) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut characters = text.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '"' if quoted && characters.peek() == Some(&'"') => {
                // A doubled quote inside a quoted field is one quote.
                field.push('"');
                characters.next();
            }
            '"' => quoted = !quoted,
            '\r' if !quoted => {}
            c if c == delimiter && !quoted => {
                row.push(std::mem::take(&mut field));
            }
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            other => field.push(other),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    // A file that ends in a newline has no empty row after it.
    rows.retain(|row| !(row.len() == 1 && row[0].is_empty()));
    rows
}

/// Write rows back out, quoting only the fields that need it.
fn rows_to_delimited(rows: &[Vec<String>], delimiter: char) -> String {
    let mut out = String::new();
    for row in rows {
        let fields: Vec<String> = row
            .iter()
            .map(|field| quote_if_needed(field, delimiter))
            .collect();
        out.push_str(&fields.join(&delimiter.to_string()));
        out.push('\n');
    }
    out
}

fn quote_if_needed(field: &str, delimiter: char) -> String {
    let needs_quotes = field.contains([delimiter, '"', '\n', '\r']);
    if needs_quotes {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

/// Rows as a JSON array.
///
/// With a header, the first row names the columns and each entry is an object.
/// Without one, every entry is an array of cells.
///
/// Columns come out sorted rather than in the order the JSON listed them:
/// `serde_json` parses an object into a sorted map, so the original order is gone
/// by the time this runs. Alphabetical is at least predictable.
fn rows_to_json(rows: &[Vec<String>], header: bool) -> String {
    let mut entries: Vec<serde_json::Value> = Vec::new();
    let (columns, body) = if header && !rows.is_empty() {
        (Some(&rows[0]), &rows[1..])
    } else {
        (None, rows)
    };

    for row in body {
        let entry = match columns {
            Some(columns) => {
                let mut object = serde_json::Map::new();
                for (index, column) in columns.iter().enumerate() {
                    let cell = row.get(index).map_or("", String::as_str);
                    object.insert(column.clone(), cell_value(cell));
                }
                serde_json::Value::Object(object)
            }
            None => serde_json::Value::Array(row.iter().map(|cell| cell_value(cell)).collect()),
        };
        entries.push(entry);
    }
    serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_owned()) + "\n"
}

/// A cell as a JSON scalar.
///
/// Integers only when they survive the round trip, so `01234` stays a string
/// instead of silently becoming the number 1234.
fn cell_value(cell: &str) -> serde_json::Value {
    if let Ok(number) = cell.parse::<i64>() {
        if number.to_string() == cell {
            return serde_json::Value::from(number);
        }
    }
    match cell {
        "true" => serde_json::Value::Bool(true),
        "false" => serde_json::Value::Bool(false),
        other => serde_json::Value::String(other.to_owned()),
    }
}

/// A JSON array of objects or arrays, flattened into rows.
fn rows_from_json(text: &str) -> Result<Vec<Vec<String>>, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let array = value
        .as_array()
        .ok_or_else(|| "the top level must be an array".to_owned())?;

    // Columns from every object in the array, so a key that appears only in a
    // later entry still gets a column. serde_json hands them over sorted.
    let mut columns: Vec<String> = Vec::new();
    for entry in array {
        if let Some(object) = entry.as_object() {
            for key in object.keys() {
                if !columns.iter().any(|column| column == key) {
                    columns.push(key.clone());
                }
            }
        }
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    if !columns.is_empty() {
        rows.push(columns.clone());
    }
    for entry in array {
        let row = match entry {
            serde_json::Value::Object(object) => columns
                .iter()
                .map(|column| object.get(column).map_or(String::new(), cell_text))
                .collect(),
            serde_json::Value::Array(cells) => cells.iter().map(cell_text).collect(),
            scalar => vec![cell_text(scalar)],
        };
        rows.push(row);
    }
    Ok(rows)
}

fn cell_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

// ------------------------------------------------------------------- helpers

fn read(path: &Path) -> Result<String, NativeError> {
    fs::read_to_string(path).map_err(|error| NativeError::Read {
        path: path.display().to_string(),
        source: error,
    })
}

fn write(path: &Path, contents: &str) -> Result<(), NativeError> {
    fs::write(path, contents).map_err(|error| NativeError::Write {
        path: path.display().to_string(),
        source: error,
    })
}

/// The lower-case extension of a path, empty when it has none.
fn extension(path: &Path) -> String {
    path.extension().map_or_else(String::new, |extension| {
        extension.to_string_lossy().to_ascii_lowercase()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Resize;
    use std::path::PathBuf;

    /// A throwaway directory that removes itself.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("orbit-native-{label}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A 40x20 PNG with a transparent right half, so flattening is visible.
    fn picture(dir: &Path) -> PathBuf {
        let path = dir.join("in.png");
        let mut buffer = image::RgbaImage::new(40, 20);
        for (x, _, pixel) in buffer.enumerate_pixels_mut() {
            pixel.0 = if x < 20 {
                [255, 0, 0, 255]
            } else {
                [255, 0, 0, 0]
            };
        }
        DynamicImage::ImageRgba8(buffer).save(&path).unwrap();
        path
    }

    fn run_image(source: &Path, dest: &Path, ext: &str, transform: Transform) -> DynamicImage {
        run(
            &NativeOp::Image {
                source: source.to_path_buf(),
                target_ext: ext.to_owned(),
                transform,
                quality: None,
                background: None,
            },
            dest,
        )
        .unwrap();
        image::open(dest).unwrap()
    }

    #[test]
    fn an_image_is_re_encoded_at_the_asked_for_size() {
        let dir = TempDir::new("resize");
        let source = picture(dir.path());
        let dest = dir.path().join("out.png");
        let out = run_image(&source, &dest, "png", Transform::Resize(Resize::Fit(10)));
        assert_eq!((out.width(), out.height()), (10, 5), "aspect ratio kept");
    }

    #[test]
    fn rotating_and_flipping_swap_the_right_axes() {
        let dir = TempDir::new("rotate");
        let source = picture(dir.path());
        let rotated = run_image(
            &source,
            &dir.path().join("r.png"),
            "png",
            Transform::Rotate { degrees: 90 },
        );
        assert_eq!((rotated.width(), rotated.height()), (20, 40));

        let flipped = run_image(
            &source,
            &dir.path().join("f.png"),
            "png",
            Transform::Flip { vertical: true },
        );
        assert_eq!((flipped.width(), flipped.height()), (40, 20));
    }

    #[test]
    fn a_crop_off_the_edge_is_clamped_rather_than_refused() {
        let dir = TempDir::new("crop-edge");
        let source = picture(dir.path());
        let out = run_image(
            &source,
            &dir.path().join("c.png"),
            "png",
            Transform::Crop {
                x: 30,
                y: 0,
                width: 100,
                height: 100,
            },
        );
        assert_eq!((out.width(), out.height()), (10, 20));
    }

    #[test]
    fn a_crop_entirely_outside_the_picture_is_an_error_not_a_panic() {
        let dir = TempDir::new("crop-outside");
        let source = picture(dir.path());
        let error = run(
            &NativeOp::Image {
                source,
                target_ext: "png".to_owned(),
                transform: Transform::Crop {
                    x: 40,
                    y: 0,
                    width: 10,
                    height: 10,
                },
                quality: None,
                background: None,
            },
            &dir.path().join("out.png"),
        )
        .expect_err("no pixels there");
        assert!(matches!(error, NativeError::Data(_)), "got {error:?}");
    }

    #[test]
    fn jpeg_flattens_transparency_onto_the_background() {
        let dir = TempDir::new("jpeg-alpha");
        let source = picture(dir.path());
        let out = run_image(&source, &dir.path().join("o.jpg"), "jpg", Transform::None);
        let rgb = out.to_rgb8();
        // JPEG is lossy, so the values are close rather than exact.
        let [red, green, blue] = rgb.get_pixel(5, 5).0;
        assert!(
            red > 200 && green < 60 && blue < 60,
            "opaque side stays red, got {:?}",
            rgb.get_pixel(5, 5)
        );
        let [red, green, blue] = rgb.get_pixel(35, 5).0;
        assert!(
            red > 200 && green > 200 && blue > 200,
            "transparent side became the background, not black: {:?}",
            rgb.get_pixel(35, 5)
        );
    }

    #[test]
    fn an_undeclared_target_is_refused() {
        let dir = TempDir::new("target");
        let source = picture(dir.path());
        let error = run(
            &NativeOp::Image {
                source,
                // WebP is ImageMagick's job, not this engine's.
                target_ext: "webp".to_owned(),
                transform: Transform::None,
                quality: None,
                background: None,
            },
            &dir.path().join("out.webp"),
        )
        .expect_err("webp is not declared");
        assert!(matches!(error, NativeError::UnsupportedTarget(_)));
    }

    #[test]
    fn markup_becomes_a_document_with_the_safe_subset() {
        let html = render_markdown(
            "# Title\n\nSome **bold** and *soft* text with `code`.\n\n- one\n- two\n\n1. first\n\
             \n> quoted\n\n```\n<script>alert(1)</script>\n```\n\n[a link](https://example.com) \
             and [a trap](javascript:alert(1))\n",
        );
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<em>soft</em>"));
        assert!(html.contains("<code>code</code>"));
        assert!(html.contains("<ul>\n<li>one</li>"));
        assert!(html.contains("<ol>\n<li>first</li>"));
        assert!(html.contains("<blockquote>quoted</blockquote>"));
        assert!(html.contains("&lt;script&gt;"), "code is escaped");
        assert!(html.contains("<a href=\"https://example.com\">a link</a>"));
        assert!(
            html.contains("a trap") && !html.contains("javascript:"),
            "a javascript: URL keeps its label and loses its target"
        );
    }

    #[test]
    fn an_unclosed_marker_still_produces_valid_html() {
        assert_eq!(emphasis("**starts bold"), "<strong>starts bold</strong>");
        assert_eq!(emphasis("a * b"), "a * b");
    }

    #[test]
    fn quoted_fields_survive_a_round_trip() {
        let rows = rows_from_delimited("a,b\n\"x,y\",\"he said \"\"hi\"\"\"\n", ',');
        assert_eq!(
            rows,
            [
                vec!["a".to_owned(), "b".to_owned()],
                vec!["x,y".to_owned(), "he said \"hi\"".to_owned()]
            ]
        );
        assert_eq!(
            rows_to_delimited(&rows, ','),
            "a,b\n\"x,y\",\"he said \"\"hi\"\"\"\n"
        );
    }

    #[test]
    fn csv_becomes_objects_with_header_and_integers_kept_honest() {
        let rows = rows_from_delimited("name,zip\nAda,01234\n", ',');
        let json = rows_to_json(&rows, true);
        assert!(json.contains("\"name\": \"Ada\""), "got {json}");
        assert!(
            json.contains("\"zip\": \"01234\""),
            "a leading zero must not become a number: {json}"
        );

        let numbered = rows_to_json(&rows_from_delimited("count\n7\n", ','), true);
        assert!(numbered.contains("\"count\": 7"), "got {numbered}");
    }

    #[test]
    fn json_becomes_rows_in_a_predictable_column_order() {
        let rows = rows_from_json(r#"[{"b": 2, "a": "x"}, {"a": "y", "c": true}]"#).unwrap();
        assert_eq!(
            rows,
            [
                vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
                vec!["x".to_owned(), "2".to_owned(), String::new()],
                vec!["y".to_owned(), String::new(), "true".to_owned()]
            ]
        );
        assert!(rows_from_json("{\"not\": \"an array\"}").is_err());
    }
}
