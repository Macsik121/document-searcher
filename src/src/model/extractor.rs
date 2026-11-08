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
    let bytes =
        fs::read(path).with_context(|| format!("Cannot read text file: {}", path.display()))?;
    Ok(decode_text_bytes(&bytes))
}

/// Decode arbitrary bytes from a `.txt`/`.md`/source file into a `String` as
/// robustly as possible, so that legacy non-UTF-8 documents (common on Russian
/// Windows) are still indexed and searchable instead of being skipped.
///
/// Decoder layers (first wins):
/// 1. **BOM fast-path** — UTF-8 / UTF-16LE / UTF-16BE byte-order marks are
///    authoritative; they decode directly without guessing.
/// 2. **Strict UTF-8 fast-path** — plain valid UTF-8 (no BOM) is the common
///    case and returns verbatim. A NUL-ratio guard rejects BOM-less UTF-16,
///    which would otherwise decode as a UTF-8 string full of NULs.
/// 3. **`chardetng` detection** — for everything else, a language-model
///    detector picks the most likely legacy encoding (CP1251 Cyrillic,
///    KOI8-R, Windows-1252, etc.) and `encoding_rs` decodes it lossily.
///
/// This never bails on encoding grounds — only an IO failure in the caller
/// produces an error. Unmappable bytes become `U+FFFD`.
fn decode_text_bytes(bytes: &[u8]) -> String {
    // Layer 1: byte-order mark.
    if bytes.len() >= 3 && bytes[..3] == [0xEF, 0xBB, 0xBF] {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if bytes.len() >= 2 {
        if bytes[..2] == [0xFF, 0xFE] {
            return decode_utf16(bytes[2..].as_ref(), /* little */ true);
        }
        if bytes[..2] == [0xFE, 0xFF] {
            return decode_utf16(bytes[2..].as_ref(), /* little */ false);
        }
    }

    // Layer 2: strict UTF-8, guarded against BOM-less UTF-16.
    if is_likely_strict_utf8(bytes) {
        if let Ok(text) = String::from_utf8(bytes.to_vec()) {
            return text;
        }
    }

    // Layer 3: detect a legacy encoding and decode it.
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let encoding = detector.guess(None, true);
    let mut output = String::with_capacity(bytes.len());
    let (_text, _enc, _had_errors) = encoding.decode(bytes, &mut output);
    output
}

/// Decode UTF-16 bytes (BOM already stripped) into a `String`, replacing
/// invalid surrogates with `U+FFFD`. `little_endian` selects LE vs BE.
fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i + 1 < bytes.len() {
        let unit = if little_endian {
            u16::from_le_bytes([bytes[i], bytes[i + 1]])
        } else {
            u16::from_be_bytes([bytes[i], bytes[i + 1]])
        };
        units.push(unit);
        i += 2;
    }
    if i < bytes.len() {
        // Trailing odd byte — treat it as a Latin-1 code unit.
        units.push(u16::from(bytes[i]));
    }
    String::from_utf16_lossy(&units)
}

/// Heuristic gate for the UTF-8 fast-path.
///
/// Returns `true` when the bytes are plausibly UTF-8 (or pure ASCII): no
/// embedded NULs beyond a tiny ratio, and no obviously valid strict-UTF-8
/// prefix that is actually BOM-less UTF-16. BOM-less UTF-16 text decodes
/// under UTF-8 as a long run of `'\0'` characters (e.g. `"H\0e\0l\0l\0o\0"`),
/// so a high NUL ratio signals "skip UTF-8, go to the detector."
fn is_likely_strict_utf8(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return true;
    }
    // A cheap pre-filter: only run the full validation if the bytes look
    // ASCII-shaped or contain continuation bytes that UTF-8 expects.
    let mut nul_count = 0usize;
    let mut high_count = 0usize;
    const SAMPLE: usize = 4096;
    let take = bytes.len().min(SAMPLE);
    for &b in &bytes[..take] {
        if b == 0 {
            nul_count += 1;
        } else if b >= 0x80 {
            high_count += 1;
        }
    }
    // BOM-less UTF-16 of ASCII text is ~50% NULs; real UTF-8 text has none.
    if nul_count * 20 > take {
        return false;
    }
    // Pure ASCII: always valid UTF-8.
    if high_count == 0 {
        return true;
    }
    // Mixed bytes: validate honestly. `from_utf8` on the whole slice below
    // does the real check; this gate only decides whether to *try* it.
    true
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
    let padded = if !hex.len().is_multiple_of(2) {
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
        if !map.two_byte.is_empty() && bytes.len() >= 2 && bytes.len().is_multiple_of(2) {
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
            "ET" if !page_text.ends_with('\n') => {
                page_text.push('\n');
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
    let doc =
        Document::load(path).with_context(|| format!("lopdf could not load {}", path.display()))?;
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
    let extraction = std::panic::catch_unwind(move || pdf_extract::extract_text(&path_for_extract));

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
    fn rejects_unsupported_extensions() {
        let file = NamedTempFile::with_suffix(".odt").unwrap();
        assert!(extract_text(file.path()).is_err());
    }

    #[test]
    fn decodes_cp1251_cyrillic_text() {
        // "Привет, мир!" encoded in Windows-1251 (CP1251) — the legacy
        // encoding that Russian Windows still writes `.txt` files in.
        let cp1251: &[u8] = &[
            0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2, 0x2C, 0x20, 0xEC, 0xE8, 0xF0, 0x21,
        ];
        let text = decode_text_bytes(cp1251);
        assert!(text.contains("Привет"), "decoded: {text:?}");
        assert!(text.contains("мир"), "decoded: {text:?}");
    }

    #[test]
    fn decodes_utf16le_with_bom() {
        // "Hello, мир!" as UTF-16LE with a BOM, like Notepad writes it.
        let bytes: &[u8] = &[
            0xFF, 0xFE, // BOM: UTF-16LE
            b'H', 0x00, b'e', 0x00, b'l', 0x00, b'l', 0x00, b'o', 0x00, b',', 0x00, b' ', 0x00,
            0x3C, 0x04, // м
            0x38, 0x04, // и
            0x40, 0x04, // р
            b'!', 0x00,
        ];
        let text = decode_text_bytes(bytes);
        assert_eq!(text, "Hello, мир!");
    }

    #[test]
    fn decodes_bomless_utf16le_ascii() {
        // BOM-less UTF-16LE of ASCII text — must NOT decode as a NUL-laden
        // UTF-8 string. The detector/NUL-guard should route it to UTF-16.
        let bytes: &[u8] = &[b'H', 0x00, b'e', 0x00, b'l', 0x00, b'l', 0x00, b'o', 0x00];
        let text = decode_text_bytes(bytes);
        assert_eq!(text.trim_matches('\0'), "Hello");
        assert!(!text.contains("Hello\0"), "should not keep NULs: {text:?}");
    }

    #[test]
    fn plain_text_never_bails_on_encoding() {
        // Even a stream of pure garbage bytes must decode (lossily) to a
        // String, never an error — only IO failures can error.
        let text = decode_text_bytes(&[0xFF, 0xFE, 0xFD, 0xFC, 0x00, 0x80]);
        // The leading FF FE is a UTF-16LE BOM; the rest is one odd byte,
        // decoded lossily. We only assert it did not panic / bail.
        let _ = text;
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
