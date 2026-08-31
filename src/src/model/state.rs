use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub relative_path: String,
    pub absolute_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchMatch {
    pub file_relative_path: String,
    pub contexts: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub current_path: PathBuf,
    pub found_files: Vec<FileEntry>,
    pub search_query: String,
    pub search_results: Vec<SearchMatch>,
    pub results_offset: usize,
    pub results_limit: usize,
    pub is_searching: bool,
    pub search_complete: bool,
    pub error: Option<String>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            current_path: PathBuf::from("/home/documents"),
            results_limit: 100,
            ..Default::default()
        }
    }

    pub fn reset_search(&mut self) {
        self.search_results.clear();
        self.results_offset = 0;
        self.search_complete = false;
        self.error = None;
    }

    pub fn get_visible_results(&self) -> &[SearchMatch] {
        let end = (self.results_offset + self.results_limit).min(self.search_results.len());
        &self.search_results[self.results_offset..end]
    }

    pub fn has_more_results(&self) -> bool {
        self.results_offset + self.results_limit < self.search_results.len()
    }

    pub fn load_more(&mut self) {
        self.results_offset += self.results_limit;
    }
}