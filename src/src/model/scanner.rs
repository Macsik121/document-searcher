use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use crate::model::state::FileEntry;

const SUPPORTED_EXTENSIONS: &[&str] = &["txt", "docx", "pdf", "md", "rtf", "odt"];

pub fn is_supported_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn scan_directory(root_path: &Path) -> anyhow::Result<Vec<FileEntry>> {
    let mut files = Vec::new();

    for entry in WalkDir::new(root_path)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && is_supported_extension(path) {
            let relative_path = path.strip_prefix(root_path)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            files.push(FileEntry {
                relative_path,
                absolute_path: path.to_path_buf(),
            });
        }
    }

    // Sort by relative path for consistent ordering
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    Ok(files)
}