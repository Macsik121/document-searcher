use std::path::Path;
use anyhow::Result;
use tesseract_rs::Tesseract;

pub fn extract_text(path: &Path) -> Result<String> {
    let extension = path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .unwrap_or_default();

    match extension.as_str() {
        "txt" | "md" => extract_plain_text(path),
        "docx" => extract_docx_text(path),
        "pdf" => extract_pdf_text(path),
        "rtf" | "odt" => extract_with_ocr(path),
        _ => extract_with_ocr(path),
    }
}

fn extract_plain_text(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read text file: {}", e))
}

fn extract_docx_text(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path)?;
    let docx = docx_rs::read_docx(&file)?;

    let mut text = String::new();
    for paragraph in docx.document.paragraphs {
        for run in paragraph.runs {
            if let Some(t) = run.text {
                text.push_str(&t);
            }
        }
        text.push('\n');
    }

    Ok(text)
}

fn extract_pdf_text(path: &Path) -> Result<String> {
    let text = pdf_extract::extract_text(path)
        .map_err(|e| anyhow::anyhow!("Failed to extract PDF text: {}", e))?;
    Ok(text)
}

fn extract_with_ocr(path: &Path) -> Result<String> {
    // For RTF, ODT and other formats, use Tesseract OCR
    // This requires the file to be rendered as an image first
    // For simplicity, we'll try to use Tesseract directly on the file
    // In production, you'd want to convert to image first (e.g., using LibreOffice headless)

    let api = Tesseract::new(None, None)?;
    api.set_image(path)?;
    let text = api.get_text()?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_extract_plain_text() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "Hello, world!").unwrap();
        writeln!(file, "This is a test.").unwrap();

        let text = extract_text(file.path()).unwrap();
        assert!(text.contains("Hello, world!"));
        assert!(text.contains("This is a test."));
    }
}