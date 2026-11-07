use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
#[cfg(feature = "ocr")]
use std::path::PathBuf;
#[cfg(feature = "ocr")]
use std::process::Command;
#[cfg(feature = "ocr")]
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::model::scanner::MAX_FILE_SIZE;

const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "rs", "toml", "json", "xml", "html", "css", "js", "ts",
];
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "tiff", "bmp"];

pub fn extract_text(path: &Path) -> Result<String> {
    let metadata =
        fs::metadata(path).with_context(|| format!("Cannot access {}", path.display()))?;
    if metadata.len() > MAX_FILE_SIZE {
        anyhow::bail!("File exceeds the 50 MiB extraction limit");
    }

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

fn extract_pdf_text(path: &Path) -> Result<String> {
    match pdf_extract::extract_text(path) {
        Ok(text) if !text.trim().is_empty() => Ok(text),
        Ok(_) => extract_scanned_pdf_text(path).context("PDF has no text layer and OCR failed"),
        Err(pdf_error) => extract_scanned_pdf_text(path).with_context(|| {
            format!("PDF text extraction failed ({pdf_error}); OCR fallback also failed")
        }),
    }
}

#[cfg(feature = "ocr")]
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

    let result = (|| {
        let prefix = directory.join("page");
        let status = Command::new("pdftoppm")
            .args(["-png", "-r", "200"])
            .arg(path)
            .arg(&prefix)
            .status()
            .context("Could not run pdftoppm for scanned PDF OCR")?;
        if !status.success() {
            anyhow::bail!("pdftoppm failed with status {status}");
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

        let mut output = String::new();
        for page in pages {
            output.push_str(&extract_image_text(&page)?);
            output.push('\n');
        }
        if output.trim().is_empty() {
            anyhow::bail!("OCR produced no text for scanned PDF");
        }
        Ok(output)
    })();

    let _ = fs::remove_dir_all(&directory);
    result
}

#[cfg(not(feature = "ocr"))]
fn extract_scanned_pdf_text(_path: &Path) -> Result<String> {
    anyhow::bail!("PDF has no text layer; scanned-PDF OCR requires the ocr feature and pdftoppm")
}

#[cfg(feature = "ocr")]
fn extract_image_text(path: &Path) -> Result<String> {
    let language =
        std::env::var("DOCUMENT_SEARCHER_OCR_LANG").unwrap_or_else(|_| "eng+rus".to_owned());
    let mut engine =
        leptess::LepTess::new(None, &language).context("Could not initialize Tesseract OCR")?;
    engine
        .set_image(path)
        .with_context(|| format!("Could not open image {} for OCR", path.display()))?;
    engine.get_utf8_text().context("Tesseract OCR failed")
}

#[cfg(not(feature = "ocr"))]
fn extract_image_text(_path: &Path) -> Result<String> {
    anyhow::bail!("OCR is only available in the Linux desktop build")
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
}
