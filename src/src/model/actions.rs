use std::path::PathBuf;

use crate::model::state::{FileEntry, SearchMatch};

#[derive(Debug)]
pub enum WorkerCommand {
    Scan {
        root: PathBuf,
        extensions: Vec<String>,
    },
    QueryChanged {
        query: String,
    },
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
    /// Streamed result: a single file with matches has been found. Sent
    /// incrementally as each file completes extraction+search, so the UI can
    /// append results progressively instead of waiting for the whole search.
    SearchResultFound {
        result: SearchMatch,
        completed: usize,
        total: usize,
    },
    SearchProgress {
        completed: usize,
        total: usize,
    },
    SearchCompleted {
        query: String,
        /// Final sorted/consolidated result list. May be empty if all
        /// results were already streamed via SearchResultFound.
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
