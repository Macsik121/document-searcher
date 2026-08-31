use std::path::PathBuf;
use crate::model::state::{FileEntry, SearchMatch};

#[derive(Debug)]
pub enum AppMsg {
    // Folder selection
    SelectFolder,
    FolderSelected(PathBuf),
    FolderSelectionCancelled,

    // Scanning
    ScanStarted,
    ScanCompleted(Vec<FileEntry>),
    ScanError(String),

    // Search
    SearchQueryChanged(String),
    SearchDebounced(String),
    SearchStarted,
    SearchProgress(usize, usize),
    SearchCompleted(Vec<SearchMatch>),
    SearchError(String),

    // Pagination
    LoadMoreResults,

    // UI
    OpenFile(PathBuf),
    ClearError,
}