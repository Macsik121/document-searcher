Ниже приведены **полные тексты всех 5 изменяемых файлов** (`src/Cargo.toml`, `src/build.rs`, `src/src/model/extractor.rs`, `src/src/main.rs`, `src/src/app_win.rs`). Остальные файлы ядра (`scanner.rs`, `search.rs`, `worker.rs`, `state.rs`, `actions.rs`, `lib.rs`) менять не нужно — они уже полностью готовы.

---

### 1. `src/Cargo.toml`

Удалены неиспользуемые C-зависимости (`gtk4`, `libadwaita`, `relm4`, `leptess`), а фичи `default`, `windows-desktop` и `desktop` объединены, чтобы сборка работала одинаково на **Windows, ALT Linux и Ubuntu** даже без флагов:

```toml
[package]
name = "document-searcher"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[features]
default = ["desktop-ui", "ocr"]
desktop-ui = []
ocr = []
windows-ocr = ["ocr"]
windows-desktop = ["desktop-ui", "ocr"]
desktop = ["desktop-ui", "ocr"]

[dependencies]
# Background processing
tokio = { version = "1.38", features = ["rt-multi-thread", "macros", "fs", "sync", "time"] }
tokio-util = "0.7"

# Document processing
anyhow = "1.0"
lopdf = "0.31"
pdf-extract = "0.7"
quick-xml = "0.31"
walkdir = "2.4"
zip = "0.6"

# Diagnostics
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
tempfile = "3"
```

---

### 2. `src/build.rs`

Универсальный скрипт сборки без внешних C/GLib-утилит:

```rust
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
}
```

---

### 3. `src/src/model/extractor.rs`

Полный кроссплатформенный 3-уровневый экстрактор (постраничный декодер кириллицы `lopdf` + `pdf-extract` внутри `catch_unwind` + единый OCR через `tesseract`/`pdftoppm` для Linux и Windows + резервный Windows WinRT OCR):

```rust
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
#[cfg(any(feature = "ocr", feature = "windows-ocr"))]
use std::process::Command;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::SystemTime;
#[cfg(any(feature = "ocr", feature = "windows-ocr"))]
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use lopdf::{content::Content, Dictionary, Document, Object, ObjectId};
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::model::scanner::MAX_FILE_SIZE;

const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "rs", "toml", "json", "xml", "html", "css", "js", "ts",
];
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "tiff", "bmp"];

const MAX_CACHED_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 4096;

#[derive(Hash, PartialEq, Eq, Clone)]
struct CacheKey {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

type CacheValue = std::result::Result<Arc<String>, String>;

static TEXT_CACHE: OnceLock<RwLock<HashMap<CacheKey, CacheValue>>> = OnceLock::new();

fn cache_store() -> &'static RwLock<HashMap<CacheKey, CacheValue>> {
    TEXT_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Normalizes Windows extended-length verbatim paths (`\\?\C:\...` and
/// `\\?\UNC\server\share\...`) produced by `Path::canonicalize()` into standard
/// paths accepted by OS Shell APIs and external CLI tools.
pub fn normalize_platform_path(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(unc_tail) = raw.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{unc_tail}"));
    }
    if let Some(unc_tail) = raw.strip_prefix(r"\\?\unc\") {
        return PathBuf::from(format!(r"\\{unc_tail}"));
    }
    if let Some(stripped) = raw.strip_prefix(r"\\?\") {
        return PathBuf::from(stripped);
    }
    path.to_path_buf()
}

pub fn extract_text(path: &Path) -> Result<String> {
    let metadata =
        fs::metadata(path).with_context(|| format!("Cannot access {}", path.display()))?;
    if metadata.len() > MAX_FILE_SIZE {
        anyhow::bail!("File exceeds the 50 MiB extraction limit");
    }

    let key = CacheKey {
        path: path.to_path_buf(),
        len: metadata.len(),
        modified: metadata.modified().ok(),
    };

    if let Ok(guard) = cache_store().read() {
        if let Some(cached) = guard.get(&key) {
            return match cached {
                Ok(text) => Ok((**text).clone()),
                Err(err_msg) => anyhow::bail!("{err_msg}"),
            };
        }
    }

    let result = extract_text_uncached(path);

    if let Ok(mut guard) = cache_store().write() {
        if guard.len() >= MAX_CACHE_ENTRIES {
            guard.clear();
        }
        match &result {
            Ok(text) if text.len() <= MAX_CACHED_TEXT_BYTES => {
                guard.insert(key, Ok(Arc::new(text.clone())));
            }
            Ok(_) => {}
            Err(error) => {
                guard.insert(key, Err(format!("{error:#}")));
            }
        }
    }

    result
}

fn extract_text_uncached(path: &Path) -> Result<String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();

    if TEXT_EXTENSIONS
        .iter()
        .any(|supported| extension.eq_ignore_ascii_case(supported))
    {
        return extract_plain_text(path);
    }

    if extension.eq_ignore_ascii_case("docx") {
        return extract_docx_text(path);
    }

    if extension.eq_ignore_ascii_case("pdf") {
        return extract_pdf_text(path);
    }

    if IMAGE_EXTENSIONS
        .iter()
        .any(|supported| extension.eq_ignore_ascii_case(supported))
    {
        return extract_image_text(path);
    }

    anyhow::bail!("Unsupported document format: .{extension}");
}

fn extract_plain_text(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| {
        format!(
            "Text file is unreadable or is not UTF-8: {}",
            path.display()
        )
    })
}

fn extract_docx_text(path: &Path) -> Result<String> {
    let file = File::open(path).with_context(|| format!("Cannot open {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("Invalid DOCX archive")?;
    let mut document = archive
        .by_name("word/document.xml")
        .context("DOCX document.xml is missing")?;

    if document.size() > MAX_FILE_SIZE {
        anyhow::bail!("DOCX document.xml exceeds the 50 MiB extraction limit");
    }

    let mut xml = String::with_capacity(document.size() as usize);
    document
        .read_to_string(&mut xml)
        .context("DOCX document.xml is not valid UTF-8")?;

    let mut reader = Reader::from_str(&xml);
    reader.trim_text(false);
    let mut output = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Text(text)) => {
                output.push_str(&text.unescape().context("Invalid XML text")?);
            }
            Ok(Event::Start(element)) if element.name().as_ref() == b"w:tab" => {
                output.push('\t');
            }
            Ok(Event::Empty(element)) if element.name().as_ref() == b"w:tab" => {
                output.push('\t');
            }
            Ok(Event::Start(element)) if element.name().as_ref() == b"w:br" => {
                output.push('\n');
            }
            Ok(Event::Empty(element)) if element.name().as_ref() == b"w:br" => {
                output.push('\n');
            }
            Ok(Event::End(element)) if element.name().as_ref() == b"w:p" => {
                output.push('\n');
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(error).context("Malformed DOCX document.xml"),
        }
    }

    Ok(output)
}

// ---------------------------------------------------------------------------
// Resilient PDF text extraction (lopdf Cyrillic /uniXXXX + ToUnicode + CP1251).
// ---------------------------------------------------------------------------

fn decode_afii_cyrillic(num: u32) -> Option<char> {
    match num {
        10017..=10022 => char::from_u32(0x0410 + (num - 10017)),
        10023 => Some('Ё'),
        10024..=10049 => char::from_u32(0x0416 + (num - 10024)),
        10065..=10070 => char::from_u32(0x0430 + (num - 10065)),
        10071 => Some('ё'),
        10072..=10097 => char::from_u32(0x0436 + (num - 10072)),
        894 => Some('№'),
        _ => None,
    }
}

fn decode_glyph_name(name: &str) -> Option<char> {
    let base = name.split('.').next().unwrap_or(name);
    if let Some(hex) = base.strip_prefix("uni") {
        if hex.len() >= 4 {
            if let Ok(code) = u32::from_str_radix(&hex[..4], 16) {
                return char::from_u32(code);
            }
        }
    }
    if let Some(hex) = base.strip_prefix('u') {
        if (4..=6).contains(&hex.len()) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(code) = u32::from_str_radix(hex, 16) {
                return char::from_u32(code);
            }
        }
    }
    if let Some(num_str) = base.strip_prefix("afii") {
        if let Ok(num) = num_str.parse::<u32>() {
            return decode_afii_cyrillic(num);
        }
    }
    match base {
        "space" | "nbspace" => Some(' '),
        "period" => Some('.'),
        "comma" => Some(','),
        "colon" => Some(':'),
        "semicolon" => Some(';'),
        "hyphen" | "minus" | "endash" | "emdash" => Some('-'),
        "guillemotleft" => Some('«'),
        "guillemotright" => Some('»'),
        "quotedblleft" | "quotedblright" | "quotedblbase" | "quotedbl" => Some('"'),
        "quotesingle" | "quoteleft" | "quoteright" => Some('\''),
        "parenleft" => Some('('),
        "parenright" => Some(')'),
        "slash" => Some('/'),
        "numbersign" => Some('#'),
        "percent" => Some('%'),
        "plus" => Some('+'),
        "equal" => Some('='),
        "question" => Some('?'),
        "exclam" => Some('!'),
        "zero" => Some('0'),
        "one" => Some('1'),
        "two" => Some('2'),
        "three" => Some('3'),
        "four" => Some('4'),
        "five" => Some('5'),
        "six" => Some('6'),
        "seven" => Some('7'),
        "eight" => Some('8'),
        "nine" => Some('9'),
        s if s.len() == 1 => s.chars().next(),
        _ => None,
    }
}

#[derive(Default)]
struct FontCharMap {
    single_byte: HashMap<u8, String>,
    two_byte: HashMap<u16, String>,
}

fn parse_hex_token(token: &str) -> Option<Vec<u8>> {
    let hex = token.trim().trim_start_matches('<').trim_end_matches('>');
    if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let padded = if hex.len() % 2 != 0 {
        format!("{hex}0")
    } else {
        hex.to_owned()
    };
    let mut bytes = Vec::with_capacity(padded.len() / 2);
    let mut idx = 0;
    while idx + 2 <= padded.len() {
        let b = u8::from_str_radix(&padded[idx..idx + 2], 16).ok()?;
        bytes.push(b);
        idx += 2;
    }
    Some(bytes)
}

fn decode_utf16be_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    if bytes.len() == 1 {
        return char::from_u32(u32::from(bytes[0])).map(|c| c.to_string());
    }
    let mut u16_units = Vec::with_capacity(bytes.len() / 2);
    let mut idx = 0;
    while idx + 1 < bytes.len() {
        u16_units.push(u16::from_be_bytes([bytes[idx], bytes[idx + 1]]));
        idx += 2;
    }
    String::from_utf16(&u16_units).ok()
}

fn insert_cmap_entry(map: &mut FontCharMap, src_bytes: &[u8], dst_str: String) {
    if src_bytes.len() == 1 {
        map.single_byte.insert(src_bytes[0], dst_str);
    } else if src_bytes.len() == 2 {
        let code = u16::from_be_bytes([src_bytes[0], src_bytes[1]]);
        map.two_byte.insert(code, dst_str);
    }
}

fn parse_tounicode_cmap(cmap_text: &str, map: &mut FontCharMap) {
    let mut lines = cmap_text.lines().map(str::trim);
    while let Some(line) = lines.next() {
        if line.ends_with("beginbfchar") {
            for entry in lines.by_ref() {
                if entry.starts_with("endbfchar") {
                    break;
                }
                let tokens: Vec<&str> = entry
                    .split('>')
                    .filter_map(|part| {
                        let start = part.find('<')?;
                        Some(&part[start + 1..])
                    })
                    .collect();
                if tokens.len() >= 2 {
                    if let (Some(src), Some(dst)) =
                        (parse_hex_token(tokens[0]), parse_hex_token(tokens[1]))
                    {
                        if let Some(decoded) = decode_utf16be_bytes(&dst) {
                            insert_cmap_entry(map, &src, decoded);
                        }
                    }
                }
            }
        } else if line.ends_with("beginbfrange") {
            for entry in lines.by_ref() {
                if entry.starts_with("endbfrange") {
                    break;
                }
                let tokens: Vec<&str> = entry
                    .split('>')
                    .filter_map(|part| {
                        let start = part.find('<')?;
                        Some(&part[start + 1..])
                    })
                    .collect();
                if tokens.len() >= 3 {
                    let (Some(start_b), Some(end_b), Some(dst_b)) = (
                        parse_hex_token(tokens[0]),
                        parse_hex_token(tokens[1]),
                        parse_hex_token(tokens[2]),
                    ) else {
                        continue;
                    };
                    if start_b.len() == 1 && end_b.len() == 1 && !dst_b.is_empty() {
                        let base_char = decode_utf16be_bytes(&dst_b)
                            .and_then(|s| s.chars().next())
                            .map(|c| c as u32);
                        if let Some(base) = base_char {
                            for code in start_b[0]..=end_b[0] {
                                let offset = u32::from(code - start_b[0]);
                                if let Some(ch) = char::from_u32(base + offset) {
                                    map.single_byte.insert(code, ch.to_string());
                                }
                            }
                        }
                    } else if start_b.len() == 2 && end_b.len() == 2 && !dst_b.is_empty() {
                        let start_u16 = u16::from_be_bytes([start_b[0], start_b[1]]);
                        let end_u16 = u16::from_be_bytes([end_b[0], end_b[1]]);
                        let base_char = decode_utf16be_bytes(&dst_b)
                            .and_then(|s| s.chars().next())
                            .map(|c| c as u32);
                        if let Some(base) = base_char {
                            if end_u16 >= start_u16 && (end_u16 - start_u16) <= 1024 {
                                for code in start_u16..=end_u16 {
                                    let offset = u32::from(code - start_u16);
                                    if let Some(ch) = char::from_u32(base + offset) {
                                        map.two_byte.insert(code, ch.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn resolve_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match obj {
        Object::Dictionary(dict) => Some(dict),
        Object::Reference(id) => doc.get_object(*id).ok()?.as_dict().ok(),
        _ => None,
    }
}

fn build_font_char_map(doc: &Document, font_dict: &Dictionary) -> FontCharMap {
    let mut map = FontCharMap::default();

    if let Ok(encoding_obj) = font_dict.get(b"Encoding") {
        if let Some(enc_dict) = resolve_dict(doc, encoding_obj) {
            let diffs_obj = enc_dict.get(b"Differences").ok().and_then(|obj| match obj {
                Object::Array(arr) => Some(arr),
                Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| o.as_array().ok()),
                _ => None,
            });
            if let Some(diffs) = diffs_obj {
                let mut current_code: u16 = 0;
                for item in diffs {
                    match item {
                        Object::Integer(code) if (0..=255).contains(code) => {
                            current_code = *code as u16;
                        }
                        Object::Name(name_bytes) => {
                            if current_code <= 255 {
                                let name = String::from_utf8_lossy(name_bytes);
                                if let Some(ch) = decode_glyph_name(&name) {
                                    map.single_byte.insert(current_code as u8, ch.to_string());
                                }
                            }
                            current_code = current_code.saturating_add(1);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    if let Ok(to_unicode_obj) = font_dict.get(b"ToUnicode") {
        let stream_res = match to_unicode_obj {
            Object::Stream(stream) => Some(stream),
            Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| o.as_stream().ok()),
            _ => None,
        };
        if let Some(stream) = stream_res {
            if let Ok(decompressed) = stream.decompressed_content() {
                let cmap_text = String::from_utf8_lossy(&decompressed);
                parse_tounicode_cmap(&cmap_text, &mut map);
            }
        }
    }

    map
}

fn decode_cp1251_byte(byte: u8) -> char {
    match byte {
        0xA8 => 'Ё',
        0xB8 => 'ё',
        0xB9 => '№',
        0xAB => '«',
        0xBB => '»',
        0x96 | 0x97 => '-',
        0xC0..=0xFF => char::from_u32(0x0410 + u32::from(byte - 0xC0)).unwrap_or(' '),
        0x20..=0x7E | b'\n' | b'\r' | b'\t' => byte as char,
        _ => ' ',
    }
}

fn decode_pdf_bytes_with_font(bytes: &[u8], font_map: Option<&FontCharMap>) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        if let Some(s) = decode_utf16be_bytes(&bytes[2..]) {
            return s;
        }
    }

    if let Some(map) = font_map {
        if !map.two_byte.is_empty() && bytes.len() >= 2 && bytes.len() % 2 == 0 {
            let mut out = String::with_capacity(bytes.len());
            let mut matched_any = false;
            let mut idx = 0;
            while idx + 1 < bytes.len() {
                let code = u16::from_be_bytes([bytes[idx], bytes[idx + 1]]);
                if let Some(mapped) = map.two_byte.get(&code) {
                    out.push_str(mapped);
                    matched_any = true;
                } else if let Some(ch) = char::from_u32(u32::from(code)) {
                    if ch.is_alphanumeric() || ch.is_whitespace() {
                        out.push(ch);
                    }
                }
                idx += 2;
            }
            if matched_any {
                return out;
            }
        }

        if !map.single_byte.is_empty() {
            let mut out = String::with_capacity(bytes.len());
            for &b in bytes {
                if let Some(mapped) = map.single_byte.get(&b) {
                    out.push_str(mapped);
                } else {
                    out.push(decode_cp1251_byte(b));
                }
            }
            return out;
        }
    }

    bytes.iter().map(|&b| decode_cp1251_byte(b)).collect()
}

fn append_pdf_text_operand(obj: &Object, font_map: Option<&FontCharMap>, output: &mut String) {
    match obj {
        Object::String(bytes, _) => {
            output.push_str(&decode_pdf_bytes_with_font(bytes, font_map));
        }
        Object::Array(items) => {
            for item in items {
                match item {
                    Object::String(bytes, _) => {
                        output.push_str(&decode_pdf_bytes_with_font(bytes, font_map));
                    }
                    Object::Integer(offset) if *offset < -120 => {
                        output.push(' ');
                    }
                    Object::Real(offset) if *offset < -120.0 => {
                        output.push(' ');
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn extract_page_custom_text(doc: &Document, page_id: ObjectId) -> String {
    let Ok(raw_content) = doc.get_page_content(page_id) else {
        return String::new();
    };
    let Ok(content) = Content::decode(&raw_content) else {
        return String::new();
    };

    let mut font_maps: BTreeMap<Vec<u8>, FontCharMap> = BTreeMap::new();
    for (name, dict) in doc.get_page_fonts(page_id) {
        font_maps.insert(name, build_font_char_map(doc, dict));
    }

    let mut current_font: Option<Vec<u8>> = None;
    let mut page_text = String::new();

    for op in &content.operations {
        match op.operator.as_str() {
            "Tf" => {
                if let Some(Object::Name(font_name)) = op.operands.first() {
                    current_font = Some(font_name.clone());
                }
            }
            "Tj" | "'" => {
                let fmap = current_font.as_ref().and_then(|f| font_maps.get(f));
                if let Some(operand) = op.operands.first() {
                    append_pdf_text_operand(operand, fmap, &mut page_text);
                }
                if op.operator == "'" {
                    page_text.push('\n');
                }
            }
            "\"" => {
                let fmap = current_font.as_ref().and_then(|f| font_maps.get(f));
                if let Some(operand) = op.operands.get(2) {
                    append_pdf_text_operand(operand, fmap, &mut page_text);
                }
                page_text.push('\n');
            }
            "TJ" => {
                let fmap = current_font.as_ref().and_then(|f| font_maps.get(f));
                if let Some(operand) = op.operands.first() {
                    append_pdf_text_operand(operand, fmap, &mut page_text);
                }
            }
            "Td" | "TD" | "T*" => {
                if !page_text.ends_with(' ') && !page_text.ends_with('\n') {
                    page_text.push(' ');
                }
            }
            "ET" => {
                if !page_text.ends_with('\n') {
                    page_text.push('\n');
                }
            }
            _ => {}
        }
    }

    page_text
}

fn count_alphabetic(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphabetic()).count()
}

fn extract_pdf_with_lopdf(path: &Path) -> Result<String> {
    let doc = Document::load(path)
        .with_context(|| format!("lopdf could not load {}", path.display()))?;
    let pages = doc.get_pages();
    if pages.is_empty() {
        anyhow::bail!("PDF contains 0 pages");
    }

    let mut combined = String::new();
    for (&page_num, &page_id) in &pages {
        let builtin = doc.extract_text(&[page_num]).unwrap_or_default();
        let custom = extract_page_custom_text(&doc, page_id);

        if count_alphabetic(&custom) >= count_alphabetic(&builtin) && !custom.trim().is_empty() {
            combined.push_str(&custom);
            combined.push('\n');
        } else if !builtin.trim().is_empty() {
            combined.push_str(&builtin);
            combined.push('\n');
        }
    }

    Ok(combined)
}

fn panic_payload_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(msg) = payload.downcast_ref::<&str>() {
        (*msg).to_owned()
    } else if let Some(msg) = payload.downcast_ref::<String>() {
        msg.clone()
    } else {
        "unknown parser panic".to_owned()
    }
}

fn extract_pdf_text(path: &Path) -> Result<String> {
    let lopdf_text = extract_pdf_with_lopdf(path).ok().unwrap_or_default();
    let lopdf_alpha = count_alphabetic(&lopdf_text);

    if lopdf_alpha >= 32 {
        return Ok(lopdf_text);
    }

    let path_for_extract = path.to_path_buf();
    let extraction =
        std::panic::catch_unwind(move || pdf_extract::extract_text(&path_for_extract));

    match extraction {
        Ok(Ok(text)) => {
            if count_alphabetic(&text) > lopdf_alpha && !text.trim().is_empty() {
                return Ok(text);
            }
            if !lopdf_text.trim().is_empty() {
                return Ok(lopdf_text);
            }
            if !text.trim().is_empty() {
                return Ok(text);
            }
            extract_scanned_pdf_text(path).context("PDF has no text layer and OCR failed")
        }
        Ok(Err(pdf_error)) => {
            if !lopdf_text.trim().is_empty() {
                return Ok(lopdf_text);
            }
            extract_scanned_pdf_text(path).with_context(|| {
                format!("PDF text extraction failed ({pdf_error}); OCR fallback also failed")
            })
        }
        Err(panic_payload) => {
            if !lopdf_text.trim().is_empty() {
                return Ok(lopdf_text);
            }
            let panic_msg = panic_payload_message(panic_payload);
            extract_scanned_pdf_text(path).with_context(|| {
                format!("PDF parser panicked ({panic_msg}); OCR fallback also failed")
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Cross-platform Scanned PDF & Image OCR (Windows + ALT Linux + Ubuntu).
// ---------------------------------------------------------------------------

#[cfg(any(feature = "ocr", feature = "windows-ocr"))]
fn configure_background_command(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
    }
}

#[cfg(all(any(feature = "ocr", feature = "windows-ocr"), target_os = "windows"))]
fn extract_scanned_pdf_via_windows_winrt(path: &Path) -> Result<String> {
    let clean_path = normalize_platform_path(path);
    let ps_script = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$pdfPath = $env:DOC_SEARCHER_PDF_PATH
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Storage.StorageFile, Windows.Storage, ContentType=WindowsRuntime]
$null = [Windows.Data.Pdf.PdfDocument, Windows.Data.Pdf, ContentType=WindowsRuntime]
$null = [Windows.Media.Ocr.OcrEngine, Windows.Media.Ocr, ContentType=WindowsRuntime]
$null = [Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics.Imaging, ContentType=WindowsRuntime]
$null = [Windows.Storage.Streams.InMemoryRandomAccessStream, Windows.Storage.Streams, ContentType=WindowsRuntime]

$asTaskGeneric = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
    $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
})[0]

function Await($WinRtTask, $ResultType) {
    $asTask = $asTaskGeneric.MakeGenericMethod($ResultType)
    $netTask = $asTask.Invoke($null, @($WinRtTask))
    $netTask.Wait(-1) | Out-Null
    $netTask.Result
}

$file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($pdfPath)) ([Windows.Storage.StorageFile])
$pdf = Await ([Windows.Data.Pdf.PdfDocument]::LoadFromFileAsync($file)) ([Windows.Data.Pdf.PdfDocument])
$ocr = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
if ($null -eq $ocr) { exit 2 }

$maxPages = [Math]::Min([int]$pdf.PageCount, 15)
for ($i = 0; $i -lt $maxPages; $i++) {
    $page = $pdf.GetPage($i)
    $stream = New-Object Windows.Storage.Streams.InMemoryRandomAccessStream
    $op = $page.RenderToStreamAsync($stream)
    $asTaskAction = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
        $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncAction'
    })[0]
    $netAction = $asTaskAction.Invoke($null, @($op))
    $netAction.Wait(-1) | Out-Null
    $decoder = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
    $bitmap = Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])
    $res = Await ($ocr.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
    Write-Output $res.Text
    $stream.Dispose()
    $page.Dispose()
}
"#;

    let mut command = Command::new("powershell");
    configure_background_command(&mut command);
    let output = command
        .args(["-NoProfile", "-NonInteractive", "-Command", ps_script])
        .env("DOC_SEARCHER_PDF_PATH", &clean_path)
        .output()
        .context("Could not start Windows WinRT PDF OCR")?;

    if !output.status.success() {
        anyhow::bail!("Windows WinRT PDF OCR exited with {}", output.status);
    }

    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.trim().is_empty() {
        anyhow::bail!("Windows WinRT PDF OCR produced empty text");
    }
    Ok(text)
}

#[cfg(any(feature = "ocr", feature = "windows-ocr"))]
fn resolve_pdftoppm_binary() -> PathBuf {
    if let Ok(custom) = std::env::var("DOCUMENT_SEARCHER_PDFTOPPM_BIN") {
        let custom_path = PathBuf::from(custom);
        if custom_path.exists() {
            return custom_path;
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates = vec![
            PathBuf::from(r"C:\Program Files\poppler\Library\bin\pdftoppm.exe"),
            PathBuf::from(r"C:\Program Files\poppler\bin\pdftoppm.exe"),
        ];
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let mut user_poppler = PathBuf::from(local_app_data);
            user_poppler.push("Programs");
            user_poppler.push("poppler");
            user_poppler.push("bin");
            user_poppler.push("pdftoppm.exe");
            candidates.push(user_poppler);
        }
        for candidate in candidates {
            if candidate.is_file() {
                return candidate;
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        for candidate in [
            "/usr/bin/pdftoppm",
            "/usr/local/bin/pdftoppm",
            "/app/bin/pdftoppm",
        ] {
            let path = PathBuf::from(candidate);
            if path.is_file() {
                return path;
            }
        }
    }

    PathBuf::from("pdftoppm")
}

#[cfg(any(feature = "ocr", feature = "windows-ocr"))]
fn resolve_tesseract_binary() -> PathBuf {
    if let Ok(custom) = std::env::var("DOCUMENT_SEARCHER_TESSERACT_BIN") {
        let custom_path = PathBuf::from(custom);
        if custom_path.exists() {
            return custom_path;
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates = vec![
            PathBuf::from(r"C:\Program Files\Tesseract-OCR\tesseract.exe"),
            PathBuf::from(r"C:\Program Files (x86)\Tesseract-OCR\tesseract.exe"),
        ];
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let mut user_tesseract = PathBuf::from(local_app_data);
            user_tesseract.push("Programs");
            user_tesseract.push("Tesseract-OCR");
            user_tesseract.push("tesseract.exe");
            candidates.push(user_tesseract);
        }
        for candidate in candidates {
            if candidate.is_file() {
                return candidate;
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        for candidate in [
            "/usr/bin/tesseract",
            "/usr/local/bin/tesseract",
            "/app/bin/tesseract",
        ] {
            let path = PathBuf::from(candidate);
            if path.is_file() {
                return path;
            }
        }
    }

    PathBuf::from("tesseract")
}

#[cfg(any(feature = "ocr", feature = "windows-ocr"))]
fn run_tesseract_cli(binary: &Path, image_path: &Path, language: &str) -> Result<String> {
    let mut command = Command::new(binary);
    configure_background_command(&mut command);

    if std::env::var_os("TESSDATA_PREFIX").is_none() {
        if let Some(parent) = binary.parent() {
            let tessdata = parent.join("tessdata");
            if tessdata.is_dir() {
                command.env("TESSDATA_PREFIX", &tessdata);
            }
        }
    }

    let output = command
        .arg(image_path)
        .arg("stdout")
        .args(["-l", language])
        .output()
        .with_context(|| {
            format!(
                "Could not execute Tesseract OCR ({})",
                binary.to_string_lossy()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Tesseract OCR failed ({}): {}",
            output.status,
            stderr.trim()
        );
    }

    String::from_utf8(output.stdout).context("Tesseract OCR output is not valid UTF-8")
}

#[cfg(any(feature = "ocr", feature = "windows-ocr"))]
fn extract_image_text(path: &Path) -> Result<String> {
    let binary = resolve_tesseract_binary();
    let clean_path = normalize_platform_path(path);
    let language_env = std::env::var("DOCUMENT_SEARCHER_OCR_LANG").ok();
    let language = language_env.as_deref().unwrap_or("eng+rus");

    match run_tesseract_cli(&binary, &clean_path, language) {
        Ok(text) => Ok(text),
        Err(first_error) if language_env.is_none() => {
            if let Ok(rus_text) = run_tesseract_cli(&binary, &clean_path, "rus") {
                return Ok(rus_text);
            }
            run_tesseract_cli(&binary, &clean_path, "eng").or(Err(first_error))
        }
        Err(error) => Err(error),
    }
}

#[cfg(not(any(feature = "ocr", feature = "windows-ocr")))]
fn extract_image_text(_path: &Path) -> Result<String> {
    anyhow::bail!("OCR is disabled in this build")
}

#[cfg(any(feature = "ocr", feature = "windows-ocr"))]
fn extract_scanned_pdf_text(path: &Path) -> Result<String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "document-searcher-pdf-{}-{timestamp}",
        std::process::id()
    ));
    fs::create_dir(&directory).with_context(|| {
        format!(
            "Could not create temporary PDF rendering directory {}",
            directory.display()
        )
    })?;

    let pdftoppm_result = (|| {
        let prefix = directory.join("page");
        let clean_path = normalize_platform_path(path);
        let pdftoppm_bin = resolve_pdftoppm_binary();
        let mut command = Command::new(&pdftoppm_bin);
        configure_background_command(&mut command);
        let output = command
            .args(["-png", "-r", "200"])
            .arg(&clean_path)
            .arg(&prefix)
            .output()
            .with_context(|| {
                format!(
                    "Could not run pdftoppm ({}) for scanned PDF OCR",
                    pdftoppm_bin.to_string_lossy()
                )
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "pdftoppm failed with status {}: {}",
                output.status,
                stderr.trim()
            );
        }

        let mut pages: Vec<PathBuf> = fs::read_dir(&directory)
            .context("Could not list rendered PDF pages")?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|page| page.extension().is_some_and(|extension| extension == "png"))
            .collect();
        pages.sort();
        if pages.is_empty() {
            anyhow::bail!("pdftoppm produced no PDF pages");
        }

        let mut extracted = String::new();
        for page in pages {
            extracted.push_str(&extract_image_text(&page)?);
            extracted.push('\n');
        }
        if extracted.trim().is_empty() {
            anyhow::bail!("OCR produced no text for scanned PDF");
        }
        Ok(extracted)
    })();

    let _ = fs::remove_dir_all(&directory);

    if pdftoppm_result.is_ok() {
        return pdftoppm_result;
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(winrt_text) = extract_scanned_pdf_via_windows_winrt(path) {
            return Ok(winrt_text);
        }
    }

    pdftoppm_result
}

#[cfg(not(any(feature = "ocr", feature = "windows-ocr")))]
fn extract_scanned_pdf_text(_path: &Path) -> Result<String> {
    anyhow::bail!("PDF has no text layer; scanned-PDF OCR requires the ocr feature and pdftoppm")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn extracts_utf8_plain_text() {
        let mut file = NamedTempFile::with_suffix(".txt").unwrap();
        writeln!(file, "Привет, searchable world!").unwrap();

        let text = extract_text(file.path()).unwrap();
        assert!(text.contains("Привет, searchable world!"));
    }

    #[test]
    fn rejects_invalid_utf8_in_text_files() {
        let mut file = NamedTempFile::with_suffix(".txt").unwrap();
        file.write_all(&[0xff, 0xfe]).unwrap();
        assert!(extract_text(file.path()).is_err());
    }

    #[test]
    fn rejects_unsupported_extensions() {
        let file = NamedTempFile::with_suffix(".odt").unwrap();
        assert!(extract_text(file.path()).is_err());
    }

    #[test]
    fn normalizes_windows_verbatim_and_unc_paths() {
        assert_eq!(
            normalize_platform_path(Path::new(r"\\?\C:\Users\Admin\Documents\report.pdf")),
            PathBuf::from(r"C:\Users\Admin\Documents\report.pdf")
        );
        assert_eq!(
            normalize_platform_path(Path::new(r"\\?\UNC\fileserver\shared\doc.docx")),
            PathBuf::from(r"\\fileserver\shared\doc.docx")
        );
        assert_eq!(
            normalize_platform_path(Path::new("/home/documents/sample.txt")),
            PathBuf::from("/home/documents/sample.txt")
        );
    }

    #[test]
    fn decodes_cyrillic_uni_and_afii_glyph_names() {
        assert_eq!(decode_glyph_name("uni041B"), Some('Л'));
        assert_eq!(decode_glyph_name("uni044F"), Some('я'));
        assert_eq!(decode_glyph_name("afii10017"), Some('А'));
        assert_eq!(decode_glyph_name("afii10065"), Some('а'));
        assert_eq!(decode_glyph_name("guillemotleft.cap"), Some('«'));
        assert_eq!(decode_glyph_name("hyphen.cap"), Some('-'));
    }
}
```

---

### 4. `src/src/main.rs`

Универсальная точка входа для всех ОС (Windows, ALT Linux, Ubuntu):

```rust
use std::env;
use std::path::Path;
use std::process::ExitCode;

use document_searcher::model;
use model::scanner::{scan_directory_with_extensions, scan_directory_with_report};
use model::search::search_files;

#[cfg(any(
    feature = "desktop-ui",
    feature = "windows-desktop",
    feature = "desktop"
))]
#[path = "app_win.rs"]
mod app_win;

fn print_help(program: &str) {
    println!(
        "Document Searcher (.docx & .pdf)\n\n\
         Usage:\n\
         \x20 {program} [directory]                       Open Desktop UI (in directory or current folder)\n\
         \x20 {program} gui [directory]                   Open Desktop UI in specified directory\n\
         \x20 {program} scan <directory> [--ext docx,pdf] Scan directory in console\n\
         \x20 {program} search <directory> <query>        Search documents in console\n\n\
         By default, only .docx and .pdf files are indexed."
    );
}

fn parse_ext_flag(args: &mut Vec<String>) -> anyhow::Result<Option<Vec<String>>> {
    if let Some(pos) = args.iter().position(|arg| arg == "--ext") {
        args.remove(pos);
        let Some(raw) = args.get(pos).cloned() else {
            anyhow::bail!("--ext requires a comma-separated list of extensions (e.g. docx,pdf)");
        };
        args.remove(pos);
        let list: Vec<String> = raw
            .split(',')
            .map(|item| item.trim().trim_start_matches('.').to_owned())
            .filter(|item| !item.is_empty())
            .collect();
        if list.is_empty() {
            anyhow::bail!("--ext list cannot be empty");
        }
        return Ok(Some(list));
    }
    Ok(None)
}

fn print_scan(root: &Path, extensions: Option<&[String]>) -> anyhow::Result<()> {
    let report = match extensions {
        Some(ext) => scan_directory_with_extensions(root, ext)?,
        None => scan_directory_with_report(root)?,
    };
    println!("Folder: {}", root.display());
    println!("Files found: {}", report.files.len());
    if report.oversized_files > 0 {
        println!("Skipped oversized files: {}", report.oversized_files);
    }
    if report.unreadable_entries > 0 {
        println!("Skipped unreadable entries: {}", report.unreadable_entries);
    }
    for file in report.files {
        println!("{}", file.relative_path);
    }
    Ok(())
}

fn print_search(root: &Path, query: &str, extensions: Option<&[String]>) -> anyhow::Result<()> {
    let report = match extensions {
        Some(ext) => scan_directory_with_extensions(root, ext)?,
        None => scan_directory_with_report(root)?,
    };
    let results = search_files(&report.files, query)?;
    println!("Matches for {query:?}: {} files", results.len());
    for result in results {
        println!("\n{}", result.file_relative_path);
        for occurrence in result.occurrences {
            println!("  {}", occurrence.snippet);
        }
    }
    Ok(())
}

fn run_cli_or_gui() -> anyhow::Result<()> {
    let mut args: Vec<String> = env::args().collect();
    let program = if args.is_empty() {
        "document-searcher".to_owned()
    } else {
        args.remove(0)
    };

    let extensions = parse_ext_flag(&mut args)?;
    if let Some(ext_list) = &extensions {
        env::set_var("DOCUMENT_SEARCHER_EXTENSIONS", ext_list.join(","));
    }

    if args.is_empty() {
        #[cfg(any(
            feature = "desktop-ui",
            feature = "windows-desktop",
            feature = "desktop"
        ))]
        {
            app_win::run();
            return Ok(());
        }
        #[cfg(not(any(
            feature = "desktop-ui",
            feature = "windows-desktop",
            feature = "desktop"
        )))]
        {
            print_help(&program);
            return Ok(());
        }
    }

    match args[0].as_str() {
        "scan" => {
            if args.len() != 2 {
                print_help(&program);
                anyhow::bail!("scan requires a <directory>");
            }
            print_scan(Path::new(&args[1]), extensions.as_deref())
        }
        "search" => {
            if args.len() != 3 {
                print_help(&program);
                anyhow::bail!("search requires <directory> and <query>");
            }
            print_search(Path::new(&args[1]), &args[2], extensions.as_deref())
        }
        "gui" => {
            #[cfg(any(
                feature = "desktop-ui",
                feature = "windows-desktop",
                feature = "desktop"
            ))]
            {
                if let Some(dir) = args.get(1) {
                    env::set_var("DOCUMENT_SEARCHER_INITIAL_DIR", dir);
                }
                app_win::run();
                Ok(())
            }
            #[cfg(not(any(
                feature = "desktop-ui",
                feature = "windows-desktop",
                feature = "desktop"
            )))]
            {
                anyhow::bail!("GUI requires the desktop-ui feature");
            }
        }
        "--help" | "-h" => {
            print_help(&program);
            Ok(())
        }
        other => {
            let candidate_path = Path::new(other);
            if candidate_path.is_dir() {
                #[cfg(any(
                    feature = "desktop-ui",
                    feature = "windows-desktop",
                    feature = "desktop"
                ))]
                {
                    env::set_var("DOCUMENT_SEARCHER_INITIAL_DIR", other);
                    app_win::run();
                    return Ok(());
                }
            }
            print_help(&program);
            anyhow::bail!("unknown command or non-existent directory: {other}");
        }
    }
}

fn main() -> ExitCode {
    match run_cli_or_gui() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
```

---

### 5. `src/src/app_win.rs`

Кроссплатформенный десктопный модуль с полной поддержкой **Windows 10/11** (`SHBrowseForFolderW`, `ShellExecuteW`, `NUL`) и **ALT Linux / Ubuntu** (`zenity` / `kdialog` / `yad`, `xdg-open`, `~/.config`, `chromium` / `firefox`), а также полем быстрого ввода пути папки прямо в интерфейсе:

```rust
//! Cross-platform desktop application (Windows, ALT Linux, Ubuntu).
//!
//! Runs `Worker` on a dedicated Tokio runtime thread via `RuntimeBridge` and
//! serializes all state mutations and `WorkerCommand` dispatches through a
//! single `Controller` monitor.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::mpsc::{self as std_mpsc, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::runtime::Builder;
use tokio::sync::mpsc;

use crate::model::actions::{WorkerCommand, WorkerEvent};
use crate::model::extractor::normalize_platform_path;
use crate::model::state::{AppState, FileEntry, SearchMatch, SearchStatus};
use crate::model::worker::Worker;

const HTTP_WORKER_THREADS: usize = 4;
const HTTP_QUEUE_CAPACITY: usize = 32;
const MAX_REQUEST_BYTES: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Worker bridge: dedicated Tokio runtime on its own thread.
// ---------------------------------------------------------------------------

struct RuntimeBridge {
    commands: mpsc::Sender<WorkerCommand>,
    runtime_thread: Option<thread::JoinHandle<()>>,
}

impl RuntimeBridge {
    fn start() -> (Self, mpsc::Receiver<WorkerEvent>) {
        let (command_tx, command_rx) = mpsc::channel(32);
        let (event_tx, event_rx) = mpsc::channel(64);

        let runtime_thread = thread::Builder::new()
            .name("document-searcher-worker".to_owned())
            .spawn(move || {
                let runtime = match Builder::new_multi_thread()
                    .enable_all()
                    .thread_name("document-searcher-worker-task")
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        tracing::error!(%error, "Could not create Tokio runtime");
                        return;
                    }
                };
                runtime.block_on(Worker::default().run(command_rx, event_tx));
            })
            .expect("failed to start worker runtime thread");

        (
            Self {
                commands: command_tx,
                runtime_thread: Some(runtime_thread),
            },
            event_rx,
        )
    }

    fn send(&self, command: WorkerCommand) {
        match self.commands.try_send(command) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(command)) => {
                let sender = self.commands.clone();
                thread::spawn(move || {
                    if let Err(error) = sender.blocking_send(command) {
                        tracing::warn!(%error, "Worker command queue is unavailable");
                    }
                });
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!("Worker command queue is closed");
            }
        }
    }
}

impl Drop for RuntimeBridge {
    fn drop(&mut self) {
        let commands = self.commands.clone();
        if let Some(runtime_thread) = self.runtime_thread.take() {
            thread::spawn(move || {
                if let Err(error) = commands.blocking_send(WorkerCommand::Shutdown) {
                    tracing::warn!(%error, "Worker command queue closed during shutdown");
                }
                if let Err(error) = runtime_thread.join() {
                    tracing::error!(?error, "Worker runtime thread panicked");
                }
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Cross-platform settings persistence (Windows %APPDATA% / Linux XDG_CONFIG).
// ---------------------------------------------------------------------------

fn settings_file_path() -> Option<PathBuf> {
    let mut base = if let Some(appdata) = std::env::var_os("APPDATA") {
        PathBuf::from(appdata)
    } else if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".config")
    } else {
        let profile = std::env::var_os("USERPROFILE")?;
        PathBuf::from(profile).join(".config")
    };
    base.push("DocumentSearcher");
    base.push("settings.conf");
    Some(base)
}

fn load_persisted_scan_path() -> Option<PathBuf> {
    let config_path = settings_file_path()?;
    let content = fs::read_to_string(config_path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(normalize_platform_path(Path::new(trimmed)))
}

fn persist_scan_path(path: &Path) {
    let Some(config_path) = settings_file_path() else {
        return;
    };
    if let Some(parent) = config_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            tracing::warn!(%error, "Could not create settings directory");
            return;
        }
    }
    let clean = normalize_platform_path(path);
    if let Err(error) = fs::write(&config_path, clean.to_string_lossy().as_bytes()) {
        tracing::warn!(%error, "Could not persist scan-path to settings file");
    }
}

fn resolve_initial_state() -> AppState {
    let mut state = AppState::new();

    if let Ok(cli_dir) = std::env::var("DOCUMENT_SEARCHER_INITIAL_DIR") {
        let path = PathBuf::from(cli_dir);
        if let Ok(canonical) = path.canonicalize() {
            let clean = normalize_platform_path(&canonical);
            persist_scan_path(&clean);
            state.set_scan_path(clean);
            return state;
        }
    }

    if let Some(saved_path) = load_persisted_scan_path() {
        if saved_path.is_dir() {
            state.set_scan_path(saved_path);
            return state;
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        if cwd.is_dir() {
            let clean = normalize_platform_path(&cwd);
            state.set_scan_path(clean);
            return state;
        }
    }

    state
}

// ---------------------------------------------------------------------------
// OS Platform Adapter: Windows (Win32) & Linux (ALT Linux / Ubuntu).
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod platform {
    use std::ffi::{OsStr, OsString};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::ptr;

    type Hwnd = isize;
    type Lparam = isize;
    type Lpitemidlist = *mut core::ffi::c_void;
    type Uint = u32;

    const BIF_RETURNONLYFSDIRS: Uint = 0x0000_0001;
    const BIF_NEWDIALOGSTYLE: Uint = 0x0000_0040;
    const BFFM_INITIALIZED: Uint = 1;
    const BFFM_SETSELECTIONW: Uint = 0x0400 + 103;
    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const MAX_PATH: usize = 260;
    const SW_SHOWNORMAL: i32 = 1;

    #[repr(C)]
    struct BrowseInfoW {
        hwnd_owner: Hwnd,
        pidl_root: Lpitemidlist,
        psz_display_name: *mut u16,
        lpsz_title: *const u16,
        ul_flags: Uint,
        lpfn: Option<unsafe extern "system" fn(Hwnd, Uint, Lparam, Lparam) -> i32>,
        l_param: Lparam,
        i_image: i32,
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHBrowseForFolderW(lpbi: *mut BrowseInfoW) -> Lpitemidlist;
        fn SHGetPathFromIDListW(pidl: Lpitemidlist, psz_path: *mut u16) -> i32;
        fn ShellExecuteW(
            hwnd: Hwnd,
            lp_operation: *const u16,
            lp_file: *const u16,
            lp_parameters: *const u16,
            lp_directory: *const u16,
            n_show_cmd: i32,
        ) -> isize;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(pv_reserved: *mut core::ffi::c_void, dw_co_init: u32) -> i32;
        fn CoUninitialize();
        fn CoTaskMemFree(pv: *mut core::ffi::c_void);
    }

    #[link(name = "user32")]
    extern "system" {
        fn SendMessageW(hwnd: Hwnd, msg: Uint, w_param: usize, l_param: Lparam) -> isize;
        fn SetForegroundWindow(hwnd: Hwnd) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            lp_file_name: *const u16,
            dw_desired_access: u32,
            dw_share_mode: u32,
            lp_security_attributes: *mut core::ffi::c_void,
            dw_creation_disposition: u32,
            dw_flags_and_attributes: u32,
            h_template_file: isize,
        ) -> isize;
        fn SetStdHandle(n_std_handle: u32, h_handle: isize) -> i32;
    }

    fn to_wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    unsafe extern "system" fn browse_callback(
        hwnd: Hwnd,
        msg: Uint,
        _l_param: Lparam,
        lp_data: Lparam,
    ) -> i32 {
        if msg == BFFM_INITIALIZED {
            let _ = SetForegroundWindow(hwnd);
            if lp_data != 0 {
                let _ = SendMessageW(hwnd, BFFM_SETSELECTIONW, 1, lp_data);
            }
        }
        0
    }

    pub fn silence_stdout_spam() {
        const GENERIC_WRITE: u32 = 0x4000_0000;
        const FILE_SHARE_READ_WRITE: u32 = 0x0000_0001 | 0x0000_0002;
        const OPEN_EXISTING: u32 = 3;
        const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;
        const INVALID_HANDLE_VALUE: isize = -1;

        let nul = to_wide(OsStr::new("NUL"));
        unsafe {
            let handle = CreateFileW(
                nul.as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_READ_WRITE,
                ptr::null_mut(),
                OPEN_EXISTING,
                0,
                0,
            );
            if handle != INVALID_HANDLE_VALUE && handle != 0 {
                let _ = SetStdHandle(STD_OUTPUT_HANDLE, handle);
            }
        }
    }

    pub fn pick_folder(initial_dir: &Path) -> Option<PathBuf> {
        let title = to_wide(OsStr::new("Select a folder to search"));
        let initial = if initial_dir.is_dir() {
            Some(to_wide(initial_dir.as_os_str()))
        } else {
            None
        };
        let l_param = match &initial {
            Some(wide) => wide.as_ptr() as Lparam,
            None => 0,
        };
        let mut display_name = [0u16; MAX_PATH];

        unsafe {
            let hr = CoInitializeEx(ptr::null_mut(), COINIT_APARTMENTTHREADED);
            let should_uninit = hr >= 0;

            let mut bi = BrowseInfoW {
                hwnd_owner: 0,
                pidl_root: ptr::null_mut(),
                psz_display_name: display_name.as_mut_ptr(),
                lpsz_title: title.as_ptr(),
                ul_flags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
                lpfn: Some(browse_callback),
                l_param,
                i_image: 0,
            };

            let pidl = SHBrowseForFolderW(&mut bi);
            if pidl.is_null() {
                if should_uninit {
                    CoUninitialize();
                }
                return None;
            }

            let mut path_buf = [0u16; MAX_PATH];
            let ok = SHGetPathFromIDListW(pidl, path_buf.as_mut_ptr());
            CoTaskMemFree(pidl);
            if should_uninit {
                CoUninitialize();
            }

            if ok == 0 {
                return None;
            }

            let len = path_buf
                .iter()
                .position(|&ch| ch == 0)
                .unwrap_or(path_buf.len());
            Some(PathBuf::from(OsString::from_wide(&path_buf[..len])))
        }
    }

    pub fn open_with_default_app(path: &Path) -> anyhow::Result<()> {
        let operation = to_wide(OsStr::new("open"));
        let target = to_wide(path.as_os_str());
        let code = unsafe {
            ShellExecuteW(
                0,
                operation.as_ptr(),
                target.as_ptr(),
                ptr::null(),
                ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if code <= 32 {
            anyhow::bail!(
                "Could not open {} (ShellExecuteW code {code})",
                path.display()
            );
        }
        Ok(())
    }

    fn find_app_browser() -> Option<PathBuf> {
        let mut candidates = Vec::new();
        for env_key in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
            if let Ok(base) = std::env::var(env_key) {
                let base_path = PathBuf::from(base);
                let mut edge = base_path.clone();
                edge.push("Microsoft");
                edge.push("Edge");
                edge.push("Application");
                edge.push("msedge.exe");
                candidates.push(edge);

                let mut chrome = base_path.clone();
                chrome.push("Google");
                chrome.push("Chrome");
                chrome.push("Application");
                chrome.push("chrome.exe");
                candidates.push(chrome);

                let mut yandex = base_path;
                yandex.push("Yandex");
                yandex.push("YandexBrowser");
                yandex.push("Application");
                yandex.push("browser.exe");
                candidates.push(yandex);
            }
        }
        candidates.into_iter().find(|path| path.is_file())
    }

    pub fn spawn_desktop_window(url: &str, profile_dir: &Path) -> Option<Child> {
        if let Some(browser) = find_app_browser() {
            let user_data_arg = format!("--user-data-dir={}", profile_dir.display());
            let app_arg = format!("--app={url}");
            if let Ok(child) = Command::new(browser)
                .args([
                    &app_arg,
                    "--window-size=1200,800",
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--disable-extensions",
                    &user_data_arg,
                ])
                .spawn()
            {
                return Some(child);
            }
        }

        let _ = open_with_default_app(Path::new(url));
        None
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use std::fs::OpenOptions;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};

    #[cfg(unix)]
    extern "C" {
        fn dup2(oldfd: i32, newfd: i32) -> i32;
    }

    pub fn silence_stdout_spam() {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            if let Ok(dev_null) = OpenOptions::new().write(true).open("/dev/null") {
                unsafe {
                    let _ = dup2(dev_null.as_raw_fd(), 1);
                }
            }
        }
    }

    pub fn pick_folder(initial_dir: &Path) -> Option<PathBuf> {
        let start_dir = if initial_dir.is_dir() {
            format!("{}/", initial_dir.display())
        } else {
            String::new()
        };

        // 1. Try zenity (standard on GNOME / MATE / Xfce / ALT Linux Workstation / Ubuntu).
        let mut zenity_cmd = Command::new("zenity");
        zenity_cmd.args([
            "--file-selection",
            "--directory",
            "--title=Select a folder to search",
        ]);
        if !start_dir.is_empty() {
            zenity_cmd.arg(format!("--filename={start_dir}"));
        }
        if let Ok(output) = zenity_cmd.stderr(Stdio::null()).output() {
            if output.status.success() {
                let selected = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if !selected.is_empty() {
                    return Some(PathBuf::from(selected));
                }
            }
            return None;
        }

        // 2. Try kdialog (standard on KDE Plasma / ALT Linux KWorkstation / Kubuntu).
        let mut kdialog_cmd = Command::new("kdialog");
        kdialog_cmd.arg("--getexistingdirectory");
        if initial_dir.is_dir() {
            kdialog_cmd.arg(initial_dir);
        } else {
            kdialog_cmd.arg(".");
        }
        kdialog_cmd.args(["--title", "Select a folder to search"]);
        if let Ok(output) = kdialog_cmd.stderr(Stdio::null()).output() {
            if output.status.success() {
                let selected = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if !selected.is_empty() {
                    return Some(PathBuf::from(selected));
                }
            }
            return None;
        }

        // 3. Try yad (lightweight GTK dialog).
        if let Ok(output) = Command::new("yad")
            .args([
                "--file",
                "--directory",
                "--title=Select a folder to search",
            ])
            .stderr(Stdio::null())
            .output()
        {
            if output.status.success() {
                let selected = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if !selected.is_empty() {
                    return Some(PathBuf::from(selected));
                }
            }
        }

        None
    }

    pub fn open_with_default_app(path: &Path) -> anyhow::Result<()> {
        if Command::new("xdg-open")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            return Ok(());
        }

        if Command::new("gio")
            .args(["open"])
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            return Ok(());
        }

        anyhow::bail!(
            "Could not open {} via xdg-open or gio open",
            path.display()
        )
    }

    fn find_linux_app_browser() -> Option<String> {
        let browsers = [
            "chromium",
            "chromium-browser",
            "google-chrome",
            "google-chrome-stable",
            "yandex-browser",
            "brave-browser",
            "microsoft-edge",
        ];
        for name in browsers {
            for prefix in ["/usr/bin", "/usr/local/bin", "/snap/bin"] {
                let candidate = Path::new(prefix).join(name);
                if candidate.is_file() {
                    return Some(candidate.to_string_lossy().into_owned());
                }
            }
        }
        None
    }

    pub fn spawn_desktop_window(url: &str, profile_dir: &Path) -> Option<Child> {
        if let Some(browser) = find_linux_app_browser() {
            let user_data_arg = format!("--user-data-dir={}", profile_dir.display());
            let app_arg = format!("--app={url}");
            if let Ok(child) = Command::new(browser)
                .args([
                    &app_arg,
                    "--window-size=1200,800",
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--disable-extensions",
                    &user_data_arg,
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                return Some(child);
            }
        }

        if let Ok(child) = Command::new("firefox")
            .args(["--new-window", url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            return Some(child);
        }

        let _ = open_with_default_app(Path::new(url));
        None
    }
}

fn open_file(path: &Path) -> anyhow::Result<()> {
    let clean = normalize_platform_path(path);
    if !clean.is_file() {
        anyhow::bail!("File does not exist: {}", clean.display());
    }
    platform::open_with_default_app(&clean)
}

// ---------------------------------------------------------------------------
// Application UI state holder & synchronized Controller monitor.
// ---------------------------------------------------------------------------

struct WinUiState {
    state: AppState,
    status_text: String,
    version: u64,
    last_query_seq: u64,
    folder_dialog_open: bool,
}

impl WinUiState {
    fn new(initial_state: AppState) -> Self {
        Self {
            state: initial_state,
            status_text: "Ready".to_owned(),
            version: 1,
            last_query_seq: 0,
            folder_dialog_open: false,
        }
    }

    fn bump(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    fn is_indexed_file(&self, candidate: &Path) -> bool {
        let normalized_candidate = normalize_platform_path(candidate);
        let candidate_str = normalized_candidate.to_string_lossy();
        self.state.found_files.iter().any(|entry| {
            let entry_norm = normalize_platform_path(&entry.absolute_path);
            entry_norm == normalized_candidate
                || entry_norm
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&candidate_str)
        })
    }

    fn set_status(&mut self, status: SearchStatus, detail: Option<&str>) {
        self.state.status = status;
        let text = match status {
            SearchStatus::Idle => detail.unwrap_or("Ready"),
            SearchStatus::Scanning => detail.unwrap_or("Scanning…"),
            SearchStatus::Searching => detail.unwrap_or("Searching…"),
            SearchStatus::Success => detail.unwrap_or("Search complete"),
            SearchStatus::Error => detail.unwrap_or("An error occurred"),
        };
        self.status_text = text.to_owned();
        self.bump();
    }

    fn set_error(&mut self, error: String) {
        self.state.error = Some(error.clone());
        self.set_status(SearchStatus::Error, Some(&error));
    }

    fn scan_started(&mut self) {
        self.state.status = SearchStatus::Scanning;
        self.state.error = None;
        self.state.found_files.clear();
        self.state.search_results.clear();
        self.state.results_offset = 0;
        self.set_status(SearchStatus::Scanning, Some("Scanning…"));
    }

    fn scan_completed(&mut self, files: Vec<FileEntry>, oversized: usize, unreadable: usize) {
        self.state.skipped_oversized_files = oversized;
        self.state.skipped_unreadable_entries = unreadable;
        self.state.found_files = files
            .into_iter()
            .map(|entry| FileEntry {
                relative_path: entry.relative_path,
                absolute_path: normalize_platform_path(&entry.absolute_path),
            })
            .collect();
        self.state.status = SearchStatus::Idle;
        let summary = match (oversized, unreadable) {
            (0, 0) => format!("Ready — {} files indexed", self.state.found_files.len()),
            _ => format!(
                "Ready — {} files indexed; skipped {} oversized and {} unreadable entries",
                self.state.found_files.len(),
                oversized,
                unreadable
            ),
        };
        self.set_status(SearchStatus::Idle, Some(&summary));
    }

    fn accept_query_edit(&mut self, query: String, seq: Option<u64>) -> bool {
        if let Some(seq_num) = seq {
            if seq_num < self.last_query_seq {
                return false;
            }
            self.last_query_seq = seq_num;
        }

        self.state.search_query = query.clone();
        self.state.search_results.clear();
        self.state.results_offset = 0;
        self.state.error = None;

        if self.state.status == SearchStatus::Scanning {
            self.bump();
            return true;
        }

        if query.trim().is_empty() {
            let count = self.state.found_files.len();
            self.set_status(
                SearchStatus::Idle,
                Some(&format!("Ready — {count} files indexed")),
            );
        } else {
            self.set_status(SearchStatus::Searching, Some("Waiting for input…"));
        }
        true
    }

    fn search_started(&mut self, query: &str) {
        if self.state.status == SearchStatus::Scanning {
            return;
        }
        if self.state.search_query != query || query.trim().is_empty() {
            return;
        }
        self.state.search_results.clear();
        self.state.results_offset = 0;
        self.state.error = None;
        self.set_status(SearchStatus::Searching, Some("Searching…"));
    }

    fn search_progress(&mut self, completed: usize, total: usize) {
        if self.state.status == SearchStatus::Scanning || self.state.search_query.trim().is_empty()
        {
            return;
        }
        let progress = format!("Searching… {completed}/{total}");
        self.set_status(SearchStatus::Searching, Some(&progress));
    }

    fn search_completed(&mut self, query: String, results: Vec<SearchMatch>) {
        if self.state.search_query != query {
            return;
        }
        self.state.search_results = results
            .into_iter()
            .map(|item| SearchMatch {
                file_relative_path: item.file_relative_path,
                file_absolute_path: normalize_platform_path(&item.file_absolute_path),
                contexts: item.contexts,
                occurrences: item.occurrences,
            })
            .collect();
        self.state.results_offset = 0;
        self.state.error = None;

        if query.trim().is_empty() {
            if self.state.status == SearchStatus::Scanning {
                self.bump();
                return;
            }
            let count = self.state.found_files.len();
            self.set_status(
                SearchStatus::Idle,
                Some(&format!("Ready — {count} files indexed")),
            );
        } else {
            let count = self.state.search_results.len();
            self.set_status(
                SearchStatus::Success,
                Some(&format!("Search complete — {count} matching files")),
            );
        }
    }

    fn load_more(&mut self) {
        self.state.load_more();
        self.bump();
    }
}

/// Synchronizes UI state mutations and `WorkerCommand` dispatch under a single
/// lock so commands are always delivered to `Worker` in the exact order of
/// state transitions.
struct Controller {
    ui: WinUiState,
    bridge: RuntimeBridge,
}

impl Controller {
    fn submit_query(&mut self, query: String, seq: Option<u64>) {
        if self.ui.accept_query_edit(query.clone(), seq) {
            self.bridge.send(WorkerCommand::QueryChanged { query });
        }
    }

    fn start_folder_scan(&mut self, folder: PathBuf) {
        let clean_folder = normalize_platform_path(&folder);
        self.ui.folder_dialog_open = false;
        self.ui.state.search_query.clear();
        self.ui.state.set_scan_path(clean_folder.clone());
        self.ui.scan_started();
        self.bridge.send(WorkerCommand::Scan { root: clean_folder });
    }
}

// ---------------------------------------------------------------------------
// JSON serialization & session token generation.
// ---------------------------------------------------------------------------

fn generate_session_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let stack_addr = &nanos as *const u128 as usize;
    format!("{nanos:032x}{pid:08x}{stack_addr:016x}")
}

fn json_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

fn serialize_state_json(ui: &WinUiState) -> String {
    let status_name = match ui.state.status {
        SearchStatus::Idle => "idle",
        SearchStatus::Scanning => "scanning",
        SearchStatus::Searching => "searching",
        SearchStatus::Success => "success",
        SearchStatus::Error => "error",
    };

    let current_page = ui.state.get_visible_results();
    let total_results = ui.state.search_results.len();
    let visible_end = if current_page.is_empty() {
        0
    } else {
        ui.state
            .results_offset
            .saturating_add(current_page.len())
            .min(total_results)
    };
    let visible_results = &ui.state.search_results[..visible_end];

    let mut json = String::with_capacity(4096);
    json.push('{');
    json.push_str(&format!("\"version\":{},", ui.version));
    json.push_str(&format!(
        "\"currentPath\":\"{}\",",
        json_escape(&normalize_platform_path(&ui.state.current_path).to_string_lossy())
    ));
    json.push_str(&format!(
        "\"searchQuery\":\"{}\",",
        json_escape(&ui.state.search_query)
    ));
    json.push_str(&format!("\"fileCount\":{},", ui.state.found_files.len()));
    json.push_str(&format!(
        "\"skippedOversized\":{},",
        ui.state.skipped_oversized_files
    ));
    json.push_str(&format!(
        "\"skippedUnreadable\":{},",
        ui.state.skipped_unreadable_entries
    ));
    json.push_str(&format!("\"status\":\"{status_name}\","));
    json.push_str(&format!(
        "\"statusText\":\"{}\",",
        json_escape(&ui.status_text)
    ));
    json.push_str(&format!("\"totalResults\":{total_results},"));
    json.push_str(&format!("\"hasMore\":{},", ui.state.has_more_results()));
    json.push_str(&format!("\"dialogOpen\":{},", ui.folder_dialog_open));

    json.push_str("\"files\":[");
    for (idx, file) in ui.state.found_files.iter().enumerate() {
        if idx > 0 {
            json.push(',');
        }
        json.push_str(&format!(
            "{{\"rel\":\"{}\",\"abs\":\"{}\"}}",
            json_escape(&file.relative_path),
            json_escape(&file.absolute_path.to_string_lossy())
        ));
    }
    json.push_str("],\"results\":[");

    for (idx, result) in visible_results.iter().enumerate() {
        if idx > 0 {
            json.push(',');
        }
        json.push_str(&format!(
            "{{\"num\":{},\"rel\":\"{}\",\"abs\":\"{}\",\"occurrences\":[",
            idx + 1,
            json_escape(&result.file_relative_path),
            json_escape(&result.file_absolute_path.to_string_lossy())
        ));
        for (occ_idx, occ) in result.occurrences.iter().enumerate() {
            if occ_idx > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                "{{\"before\":\"{}\",\"matched\":\"{}\",\"after\":\"{}\"}}",
                json_escape(&occ.before),
                json_escape(&occ.matched),
                json_escape(&occ.after)
            ));
        }
        json.push_str("]}");
    }
    json.push_str("]}");
    json
}

// ---------------------------------------------------------------------------
// Fixed-size HTTP worker pool & strict HTTP/UTF-8 request handling.
// ---------------------------------------------------------------------------

struct HttpWorkerPool {
    sender: Option<SyncSender<TcpStream>>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl HttpWorkerPool {
    fn start(
        controller: Arc<Mutex<Controller>>,
        shutdown_requested_at: Arc<Mutex<Option<Instant>>>,
        session_token: Arc<String>,
    ) -> Self {
        let (tx, rx) = std_mpsc::sync_channel::<TcpStream>(HTTP_QUEUE_CAPACITY);
        let shared_rx = Arc::new(Mutex::new(rx));
        let mut workers = Vec::with_capacity(HTTP_WORKER_THREADS);

        for index in 0..HTTP_WORKER_THREADS {
            let rx_clone = Arc::clone(&shared_rx);
            let ctrl_clone = Arc::clone(&controller);
            let shutdown_clone = Arc::clone(&shutdown_requested_at);
            let token_clone = Arc::clone(&session_token);

            let handle = thread::Builder::new()
                .name(format!("document-searcher-http-{index}"))
                .spawn(move || loop {
                    let stream = {
                        let Ok(guard) = rx_clone.lock() else {
                            break;
                        };
                        match guard.recv() {
                            Ok(stream) => stream,
                            Err(_) => break,
                        }
                    };
                    handle_client(stream, &ctrl_clone, &shutdown_clone, &token_clone);
                })
                .expect("failed to spawn HTTP worker thread");
            workers.push(handle);
        }

        Self {
            sender: Some(tx),
            workers,
        }
    }

    fn dispatch(&self, stream: TcpStream) {
        if let Some(sender) = &self.sender {
            let _ = sender.try_send(stream);
        }
    }
}

impl Drop for HttpWorkerPool {
    fn drop(&mut self) {
        self.sender.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

struct HttpRequest {
    method: String,
    path: String,
    token_header: Option<String>,
    query_seq: Option<u64>,
    body: String,
}

fn read_http_request(stream: &mut TcpStream) -> Option<HttpRequest> {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let mut buffer = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];

    let header_end = loop {
        let bytes_read = stream.read(&mut chunk).ok()?;
        if bytes_read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..bytes_read]);
        if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        if buffer.len() > MAX_REQUEST_BYTES {
            return None;
        }
    };

    let header_str = std::str::from_utf8(&buffer[..header_end]).ok()?;
    let mut lines = header_str.lines();
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();

    let mut content_length = 0usize;
    let mut token_header = None;
    let mut query_seq = None;

    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let trimmed_name = name.trim();
            if trimmed_name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().ok()?;
            } else if trimmed_name.eq_ignore_ascii_case("x-document-searcher-token") {
                token_header = Some(value.trim().to_owned());
            } else if trimmed_name.eq_ignore_ascii_case("x-query-seq") {
                query_seq = value.trim().parse().ok();
            }
        }
    }

    if content_length > MAX_REQUEST_BYTES {
        return None;
    }

    let target_len = header_end.saturating_add(content_length);
    while buffer.len() < target_len {
        let bytes_read = stream.read(&mut chunk).ok()?;
        if bytes_read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..bytes_read]);
    }

    let body = std::str::from_utf8(&buffer[header_end..target_len])
        .ok()?
        .to_owned();

    Some(HttpRequest {
        method,
        path,
        token_header,
        query_seq,
        body,
    })
}

fn write_http_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn handle_client(
    mut stream: TcpStream,
    controller: &Arc<Mutex<Controller>>,
    shutdown_requested_at: &Arc<Mutex<Option<Instant>>>,
    session_token: &str,
) {
    let Some(req) = read_http_request(&mut stream) else {
        return;
    };

    if req.method == "POST" {
        let header_matches = req.token_header.as_deref() == Some(session_token);
        let beacon_matches = req.path == "/api/shutdown" && req.body.trim() == session_token;
        if !header_matches && !beacon_matches {
            write_http_response(
                &mut stream,
                "403 Forbidden",
                "text/plain; charset=utf-8",
                b"Forbidden",
            );
            return;
        }
    }

    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => {
            if let Ok(mut shutdown) = shutdown_requested_at.lock() {
                *shutdown = None;
            }
            let html = INDEX_HTML.replace("__SESSION_TOKEN__", session_token);
            write_http_response(
                &mut stream,
                "200 OK",
                "text/html; charset=utf-8",
                html.as_bytes(),
            );
        }
        ("GET", "/api/state") => {
            if let Ok(mut shutdown) = shutdown_requested_at.lock() {
                *shutdown = None;
            }
            let payload = match controller.lock() {
                Ok(guard) => serialize_state_json(&guard.ui),
                Err(_) => "{\"error\":\"state lock poisoned\"}".to_owned(),
            };
            write_http_response(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                payload.as_bytes(),
            );
        }
        ("POST", "/api/query") => {
            if let Ok(mut guard) = controller.lock() {
                guard.submit_query(req.body, req.query_seq);
            }
            write_http_response(&mut stream, "200 OK", "text/plain; charset=utf-8", b"ok");
        }
        ("POST", "/api/load-more") => {
            if let Ok(mut guard) = controller.lock() {
                guard.ui.load_more();
            }
            write_http_response(&mut stream, "200 OK", "text/plain; charset=utf-8", b"ok");
        }
        ("POST", "/api/open-file") => {
            let target = PathBuf::from(&req.body);
            let allowed = match controller.lock() {
                Ok(guard) => guard.ui.is_indexed_file(&target),
                Err(_) => false,
            };
            if !allowed {
                write_http_response(
                    &mut stream,
                    "403 Forbidden",
                    "text/plain; charset=utf-8",
                    b"File is not in the current scan index",
                );
                return;
            }
            if let Err(error) = open_file(&target) {
                tracing::warn!(%error, path = %target.display(), "Could not open file");
                if let Ok(mut guard) = controller.lock() {
                    guard.ui.set_error(error.to_string());
                }
            }
            write_http_response(&mut stream, "200 OK", "text/plain; charset=utf-8", b"ok");
        }
        ("POST", "/api/set-folder") => {
            let candidate = PathBuf::from(req.body.trim());
            if let Ok(canonical) = candidate.canonicalize() {
                let clean = normalize_platform_path(&canonical);
                if clean.is_dir() {
                    persist_scan_path(&clean);
                    if let Ok(mut guard) = controller.lock() {
                        guard.start_folder_scan(clean);
                    }
                    write_http_response(&mut stream, "200 OK", "text/plain; charset=utf-8", b"ok");
                    return;
                }
            }
            if let Ok(mut guard) = controller.lock() {
                guard.ui.set_error(format!(
                    "Selected path is not a readable directory: {}",
                    candidate.display()
                ));
            }
            write_http_response(
                &mut stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                b"invalid directory",
            );
        }
        ("POST", "/api/choose-folder") => {
            let initial_dir = match controller.lock() {
                Ok(mut guard) => {
                    if guard.ui.folder_dialog_open {
                        write_http_response(
                            &mut stream,
                            "200 OK",
                            "text/plain; charset=utf-8",
                            b"busy",
                        );
                        return;
                    }
                    guard.ui.folder_dialog_open = true;
                    guard.ui.bump();
                    normalize_platform_path(&guard.ui.state.current_path)
                }
                Err(_) => return,
            };

            let ctrl_clone = Arc::clone(controller);
            thread::spawn(move || {
                let selected = platform::pick_folder(&initial_dir);
                match selected {
                    Some(folder) => {
                        persist_scan_path(&folder);
                        if let Ok(mut guard) = ctrl_clone.lock() {
                            guard.start_folder_scan(folder);
                        }
                    }
                    None => {
                        if let Ok(mut guard) = ctrl_clone.lock() {
                            guard.ui.folder_dialog_open = false;
                            guard.ui.bump();
                        }
                    }
                }
            });

            write_http_response(&mut stream, "200 OK", "text/plain; charset=utf-8", b"ok");
        }
        ("POST", "/api/shutdown") => {
            if let Ok(mut shutdown) = shutdown_requested_at.lock() {
                *shutdown = Some(Instant::now());
            }
            write_http_response(&mut stream, "200 OK", "text/plain; charset=utf-8", b"bye");
        }
        _ => {
            write_http_response(
                &mut stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                b"Not Found",
            );
        }
    }
}

fn should_terminate_loop(
    controller: &Mutex<Controller>,
    shutdown_requested_at: &Mutex<Option<Instant>>,
) -> bool {
    let dialog_open = match controller.lock() {
        Ok(guard) => guard.ui.folder_dialog_open,
        Err(_) => false,
    };
    if dialog_open {
        return false;
    }

    let Ok(shutdown_guard) = shutdown_requested_at.lock() else {
        return false;
    };
    match *shutdown_guard {
        Some(instant) => instant.elapsed() >= Duration::from_millis(1200),
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Entry point.
// ---------------------------------------------------------------------------

pub fn run() {
    platform::silence_stdout_spam();

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let (bridge, mut events) = RuntimeBridge::start();
    let initial_state = resolve_initial_state();
    let initial_path = initial_state.current_path.clone();

    let mut initial_controller = Controller {
        ui: WinUiState::new(initial_state),
        bridge,
    };
    initial_controller.ui.scan_started();
    initial_controller
        .bridge
        .send(WorkerCommand::Scan { root: initial_path });

    let controller = Arc::new(Mutex::new(initial_controller));

    // Forward WorkerEvent messages into Controller.ui.
    {
        let ctrl_events = Arc::clone(&controller);
        thread::Builder::new()
            .name("document-searcher-ui-events".to_owned())
            .spawn(move || {
                while let Some(event) = events.blocking_recv() {
                    let Ok(mut guard) = ctrl_events.lock() else {
                        break;
                    };
                    match event {
                        WorkerEvent::ScanStarted => guard.ui.scan_started(),
                        WorkerEvent::ScanProgress { completed } => guard.ui.set_status(
                            SearchStatus::Scanning,
                            Some(&format!("Scanning… {completed} files")),
                        ),
                        WorkerEvent::ScanCompleted {
                            files,
                            oversized_files,
                            unreadable_entries,
                        } => guard
                            .ui
                            .scan_completed(files, oversized_files, unreadable_entries),
                        WorkerEvent::ScanError(error) => guard.ui.set_error(error),
                        WorkerEvent::SearchStarted { query } => guard.ui.search_started(&query),
                        WorkerEvent::SearchProgress { completed, total } => {
                            guard.ui.search_progress(completed, total)
                        }
                        WorkerEvent::SearchCompleted { query, results } => {
                            guard.ui.search_completed(query, results)
                        }
                        WorkerEvent::SearchCancelled { query } => {
                            if guard.ui.state.search_query == query {
                                guard
                                    .ui
                                    .set_status(SearchStatus::Idle, Some("Search cancelled"));
                            }
                        }
                        WorkerEvent::SearchError { query, error } => {
                            if guard.ui.state.search_query == query {
                                guard.ui.set_error(error);
                            }
                        }
                    }
                }
            })
            .expect("failed to start UI event forwarding thread");
    }

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("failed to bind local UI server on 127.0.0.1");
    let port = listener
        .local_addr()
        .expect("failed to read local UI server address")
        .port();
    let _ = listener.set_nonblocking(true);

    let session_token = Arc::new(generate_session_token());
    let shutdown_requested_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let pool = HttpWorkerPool::start(
        Arc::clone(&controller),
        Arc::clone(&shutdown_requested_at),
        Arc::clone(&session_token),
    );

    let url = format!("http://127.0.0.1:{port}");
    tracing::info!(%url, "Document Searcher Desktop UI started");

    let profile_dir = std::env::temp_dir().join(format!(
        "document-searcher-ui-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    let _ = fs::create_dir_all(&profile_dir);

    let window_child = platform::spawn_desktop_window(&url, &profile_dir);

    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                pool.dispatch(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => {
                tracing::warn!(%error, "UI listener accept error");
                thread::sleep(Duration::from_millis(50));
            }
        }

        if should_terminate_loop(&controller, &shutdown_requested_at) {
            break;
        }
    }

    drop(pool);
    if let Some(mut child) = window_child {
        let _ = child.kill();
    }
    let _ = fs::remove_dir_all(&profile_dir);
}

// ---------------------------------------------------------------------------
// Offline embedded UI (uses r##"..."## so "#..." inside JS/CSS never closes it).
// ---------------------------------------------------------------------------

const INDEX_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Document Searcher</title>
<style>
  :root {
    --bg-main: #ffffff;
    --bg-sidebar: #f8fafc;
    --border-color: #e2e8f0;
    --text-primary: #1e293b;
    --text-secondary: #64748b;
    --text-muted: #94a3b8;
    --accent: #2563eb;
    --accent-hover: #1d4ed8;
    --accent-soft: #eff6ff;
    --success: #16a34a;
    --error: #dc2626;
    --mark-bg: #fef08a;
  }
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body {
    font-family: "Segoe UI", -apple-system, BlinkMacSystemFont, sans-serif;
    background: var(--bg-main);
    color: var(--text-primary);
    height: 100vh;
    width: 100vw;
    display: flex;
    overflow: hidden;
    user-select: none;
  }
  aside {
    width: 42%;
    min-width: 320px;
    max-width: 560px;
    background: var(--bg-sidebar);
    border-right: 2px solid var(--border-color);
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .sidebar-header {
    padding: 20px 20px 14px 20px;
    border-bottom: 1px solid var(--border-color);
  }
  .folder-badge {
    font-family: Consolas, "Courier New", monospace;
    font-size: 13px;
    color: #334155;
    background: #e2e8f0;
    padding: 6px 10px;
    border-radius: 6px;
    display: block;
    word-break: break-all;
    margin-bottom: 8px;
    user-select: text;
  }
  .file-count {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
  }
  .section-caption {
    padding: 12px 20px 6px 20px;
    font-size: 11px;
    font-weight: 700;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .files-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 4px 12px 12px 12px;
  }
  .file-row {
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    padding: 8px 10px;
    border-radius: 6px;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 10px;
    color: #334155;
    font-size: 13.5px;
    font-weight: 500;
    transition: background 0.12s, color 0.12s;
  }
  .file-row:hover {
    background: #e2e8f0;
    color: #0f172a;
  }
  .file-row-icon {
    color: var(--accent);
    flex-shrink: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .file-row-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sidebar-footer {
    padding: 12px 20px;
    border-top: 1px solid var(--border-color);
    background: var(--bg-sidebar);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .path-input-row {
    display: flex;
    gap: 6px;
  }
  .path-input {
    flex: 1;
    border: 1px solid #cbd5e1;
    border-radius: 6px;
    padding: 7px 10px;
    font-family: Consolas, "Courier New", monospace;
    font-size: 12.5px;
    color: var(--text-primary);
    outline: none;
  }
  .path-input:focus {
    border-color: var(--accent);
  }
  .btn-secondary {
    background: #e2e8f0;
    color: #1e293b;
    border: none;
    border-radius: 6px;
    padding: 7px 12px;
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
  }
  .btn-secondary:hover {
    background: #cbd5e1;
  }
  .btn-primary {
    background: var(--accent);
    color: #ffffff;
    border: none;
    border-radius: 8px;
    padding: 10px 18px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    width: 100%;
    transition: background 0.15s;
  }
  .btn-primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .btn-primary:disabled {
    opacity: 0.6;
    cursor: default;
  }
  main {
    flex: 1;
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }
  .search-pane {
    padding: 28px 32px 20px 32px;
    border-bottom: 2px solid var(--border-color);
    background: #ffffff;
  }
  .search-title {
    font-size: 22px;
    font-weight: 700;
    color: #0f172a;
    margin-bottom: 6px;
  }
  .search-subtitle {
    font-size: 14px;
    color: var(--text-secondary);
    margin-bottom: 16px;
  }
  .search-input-wrap {
    display: flex;
    max-width: 720px;
  }
  .search-input {
    flex: 1;
    border: 1.5px solid #cbd5e1;
    border-radius: 8px;
    padding: 11px 14px;
    font-size: 15px;
    color: var(--text-primary);
    outline: none;
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .search-input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px rgba(37, 99, 235, 0.15);
  }
  .status-bar {
    margin-top: 14px;
    min-height: 24px;
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 13.5px;
  }
  .spinner {
    width: 18px;
    height: 18px;
    border: 2.5px solid #bfdbfe;
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
    display: none;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }
  .status-icon {
    font-size: 16px;
    font-weight: bold;
    display: none;
  }
  .status-icon-success { color: var(--success); }
  .status-icon-error { color: var(--error); }
  .status-idle { color: var(--text-secondary); }
  .status-scanning, .status-searching { color: var(--accent); font-weight: 600; }
  .status-success { color: var(--success); font-weight: 700; }
  .status-error { color: var(--error); font-weight: 600; }
  .results-pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: #ffffff;
  }
  .results-header {
    padding: 16px 32px 12px 32px;
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 17px;
    font-weight: 700;
    color: #1e293b;
  }
  .results-badge {
    background: #dbeafe;
    color: #1d4ed8;
    font-size: 12.5px;
    padding: 2px 10px;
    border-radius: 999px;
    font-weight: 600;
  }
  .results-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 4px 32px 24px 32px;
  }
  .result-card {
    border: 1px solid var(--border-color);
    border-radius: 10px;
    padding: 14px 18px;
    margin-bottom: 12px;
    background: #ffffff;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.03);
    transition: border-color 0.15s, background 0.15s;
  }
  .result-card:hover {
    border-color: #93c5fd;
    background: rgba(239, 246, 255, 0.35);
  }
  .result-link {
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
    text-align: left;
    font-size: 15.5px;
    font-weight: 700;
    color: var(--accent);
    text-decoration: underline;
    display: inline-block;
  }
  .result-link:hover {
    color: var(--accent-hover);
  }
  .result-abs-path {
    font-family: Consolas, "Courier New", monospace;
    font-size: 11.5px;
    color: var(--text-muted);
    margin-top: 2px;
    margin-bottom: 8px;
    user-select: text;
  }
  .context-box {
    font-family: Consolas, "Courier New", monospace;
    font-size: 13px;
    color: #1e293b;
    background: #f1f5f9;
    padding: 6px 10px;
    border-radius: 5px;
    margin-top: 6px;
    margin-left: 16px;
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
  }
  mark.search-match {
    background: var(--mark-bg);
    color: #0f172a;
    font-weight: 700;
    padding: 1px 4px;
    border-radius: 3px;
  }
  .load-more-wrap {
    padding: 12px 32px 18px 32px;
    text-align: center;
    display: none;
  }
  .btn-more {
    background: var(--accent);
    color: #ffffff;
    border: none;
    border-radius: 8px;
    padding: 9px 28px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
  }
  .btn-more:hover {
    background: var(--accent-hover);
  }
  .empty-hint {
    color: var(--text-muted);
    font-size: 14px;
    padding: 12px 8px;
  }
</style>
</head>
<body>
  <aside>
    <div class="sidebar-header">
      <div id="folderLabel" class="folder-badge">Folder: /home/documents</div>
      <div id="fileCountLabel" class="file-count">0 files are found</div>
    </div>
    <div class="section-caption">Files found in folder</div>
    <div id="filesList" class="files-scroll"></div>
    <div class="sidebar-footer">
      <div class="path-input-row">
        <input
          id="manualPathInput"
          class="path-input"
          type="text"
          placeholder="Paste folder path and press Go…"
          autocomplete="off"
          spellcheck="false"
        />
        <button id="goPathBtn" class="btn-secondary" type="button">Go</button>
      </div>
      <button id="changeFolderBtn" class="btn-primary" type="button">Change folder</button>
    </div>
  </aside>

  <main>
    <section class="search-pane">
      <h1 class="search-title">Write what you want to find!</h1>
      <p class="search-subtitle">
        The search runs through all supported files found in the selected folder
      </p>
      <div class="search-input-wrap">
        <input
          id="searchInput"
          