use std::path::PathBuf;

use crate::model::state::{FileEntry, SearchMatch};

#[derive(Debug)]
pub enum WorkerCommand {
    Scan { root: PathBuf },
    QueryChanged { query: String },
    Shutdown,
}

#[derive(Debug)]
pub enum WorkerEvent {
    ScanStarted,
    ScanProgress {
        completed: usize,
    },
    ScanCompleted {
        files: Vec<FileEntry>,
        oversized_files: usize,
        unreadable_entries: usize,
    },
    ScanError(String),
    SearchStarted {
        query: String,
    },
    SearchProgress {
        completed: usize,
        total: usize,
    },
    SearchCompleted {
        query: String,
        results: Vec<SearchMatch>,
    },
    SearchCancelled {
        query: String,
    },
    SearchError {
        query: String,
        error: String,
    },
}

#[derive(Debug)]
pub enum AppMsg {
    SelectFolder,
    FolderSelected(PathBuf),
    FolderSelectionCancelled,
    ScanStarted,
    ScanProgress(usize),
    ScanCompleted {
        files: Vec<FileEntry>,
        oversized_files: usize,
        unreadable_entries: usize,
    },
    ScanError(String),
    SearchQueryChanged(String),
    SearchStarted(String),
    SearchProgress(usize, usize),
    SearchCompleted(Vec<SearchMatch>),
    SearchCancelled,
    SearchError(String),
    LoadMoreResults,
    OpenFile(PathBuf),
    ClearError,
}
