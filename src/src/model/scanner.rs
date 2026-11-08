use std::path::Path;

use walkdir::{DirEntry, WalkDir};

use crate::model::state::FileEntry;

pub const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

/// По умолчанию сканер ищет только файлы .docx и .pdf.
pub const DEFAULT_EXTENSIONS: &[&str] = &["docx", "pdf"];

#[derive(Debug, Default)]
pub struct ScanReport {
    pub files: Vec<FileEntry>,
    pub oversized_files: usize,
    pub unreadable_entries: usize,
}

pub fn is_allowed_extension<S: AsRef<str>>(path: &Path, allowed_extensions: &[S]) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            allowed_extensions.iter().any(|supported| {
                let clean = supported.as_ref().trim().trim_start_matches('.');
                !clean.is_empty() && extension.eq_ignore_ascii_case(clean)
            })
        })
}

pub fn is_supported_extension(path: &Path) -> bool {
    is_allowed_extension(path, DEFAULT_EXTENSIONS)
}

fn is_visible_entry(entry: &DirEntry) -> bool {
    entry.depth() == 0
        || entry
            .file_name()
            .to_str()
            .is_some_and(|name| !name.starts_with('.'))
}

pub fn scan_directory(root_path: &Path) -> anyhow::Result<Vec<FileEntry>> {
    Ok(scan_directory_with_report(root_path)?.files)
}

pub fn scan_directory_with_report(root_path: &Path) -> anyhow::Result<ScanReport> {
    if let Ok(custom_ext) = std::env::var("DOCUMENT_SEARCHER_EXTENSIONS") {
        let parsed: Vec<String> = custom_ext
            .split(',')
            .map(|item| item.trim().trim_start_matches('.').to_owned())
            .filter(|item| !item.is_empty())
            .collect();
        if !parsed.is_empty() {
            return scan_directory_with_extensions(root_path, &parsed);
        }
    }
    scan_directory_with_extensions(root_path, DEFAULT_EXTENSIONS)
}

pub fn scan_directory_with_extensions<S: AsRef<str>>(
    root_path: &Path,
    allowed_extensions: &[S],
) -> anyhow::Result<ScanReport> {
    if !root_path.is_dir() {
        anyhow::bail!(
            "Selected path is not a readable directory: {}",
            root_path.display()
        );
    }

    let root_path = root_path.canonicalize()?;
    let mut report = ScanReport::default();
    let walker = WalkDir::new(&root_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(is_visible_entry);

    for result in walker {
        let entry = match result {
            Ok(entry) => entry,
            Err(error) => {
                report.unreadable_entries += 1;
                tracing::warn!(error = %error, "Skipping unreadable directory entry");
                continue;
            }
        };

        if !entry.file_type().is_file() || !is_allowed_extension(entry.path(), allowed_extensions) {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                report.unreadable_entries += 1;
                tracing::warn!(path = %entry.path().display(), error = %error, "Skipping unreadable file");
                continue;
            }
        };

        if metadata.len() > MAX_FILE_SIZE {
            report.oversized_files += 1;
            continue;
        }

        let relative_path = entry
            .path()
            .strip_prefix(&root_path)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");

        report.files.push(FileEntry {
            relative_path,
            absolute_path: entry.path().to_path_buf(),
        });
    }

    report
        .files
        .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn supports_only_docx_and_pdf_case_insensitively_by_default() {
        assert!(is_supported_extension(Path::new("report.PDF")));
        assert!(is_supported_extension(Path::new("docs/contract.DOCX")));
        assert!(!is_supported_extension(Path::new("src/main.rs")));
        assert!(!is_supported_extension(Path::new("notes.txt")));
        assert!(!is_supported_extension(Path::new("report.odt")));
    }

    #[test]
    fn scans_visible_supported_files_and_sorts_paths() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("b")).unwrap();
        fs::create_dir(root.path().join(".private")).unwrap();
        fs::write(root.path().join("z.pdf"), "z").unwrap();
        fs::write(root.path().join("b/a.docx"), "a").unwrap();
        fs::write(root.path().join(".private/hidden.pdf"), "hidden").unwrap();
        fs::write(root.path().join("ignored.txt"), "ignored").unwrap();

        let report = scan_directory_with_report(root.path()).unwrap();
        let paths: Vec<_> = report
            .files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect();
        assert_eq!(paths, ["b/a.docx", "z.pdf"]);
    }

    #[test]
    fn skips_files_over_the_size_limit() {
        let root = tempdir().unwrap();
        let path = root.path().join("large.pdf");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_FILE_SIZE + 1).unwrap();

        let report = scan_directory_with_report(root.path()).unwrap();
        assert!(report.files.is_empty());
        assert_eq!(report.oversized_files, 1);
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("outside.pdf"), "outside").unwrap();
        symlink(outside.path(), root.path().join("linked")).unwrap();

        let report = scan_directory_with_report(root.path()).unwrap();
        assert!(report.files.is_empty());
    }
}
