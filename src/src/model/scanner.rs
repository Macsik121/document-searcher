use std::path::{Path, PathBuf};

use walkdir::{DirEntry, WalkDir};

use crate::model::state::FileEntry;

pub const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

const SUPPORTED_EXTENSIONS: &[&str] = &[
    "txt", "md", "rs", "toml", "json", "xml", "html", "css", "js",
    "ts", "pdf", "docx", "png", "jpg", "jpeg", "tiff", "bmp",
];

#[derive(Debug, Default)]
pub struct ScanReport {
    pub files: Vec<FileEntry>,
    pub oversized_files: usize,
    pub unreadable_entries: usize,
}

pub fn is_supported_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
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
    if !root_path.is_dir() {
        anyhow::bail!("Selected path is not a readable directory: {}", root_path.display());
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

        if !entry.file_type().is_file() || !is_supported_extension(entry.path()) {
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
    fn supports_specified_extensions_case_insensitively() {
        assert!(is_supported_extension(Path::new("report.PDF")));
        assert!(is_supported_extension(Path::new("src/main.RS")));
        assert!(!is_supported_extension(Path::new("report.odt")));
    }

    #[test]
    fn scans_visible_supported_files_and_sorts_paths() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("b")).unwrap();
        fs::create_dir(root.path().join(".private")).unwrap();
        fs::write(root.path().join("z.txt"), "z").unwrap();
        fs::write(root.path().join("b/a.md"), "a").unwrap();
        fs::write(root.path().join(".private/hidden.txt"), "hidden").unwrap();
        fs::write(root.path().join("ignored.odt"), "ignored").unwrap();

        let report = scan_directory_with_report(root.path()).unwrap();
        let paths: Vec<_> = report
            .files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect();
        assert_eq!(paths, ["b/a.md", "z.txt"]);
    }

    #[test]
    fn skips_files_over_the_size_limit() {
        let root = tempdir().unwrap();
        let path = root.path().join("large.txt");
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
        fs::write(outside.path().join("outside.txt"), "outside").unwrap();
        symlink(outside.path(), root.path().join("linked")).unwrap();

        let report = scan_directory_with_report(root.path()).unwrap();
        assert!(report.files.is_empty());
    }
}
