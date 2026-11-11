use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub relative_path: String,
    pub absolute_path: PathBuf,
}

/// Represents a single page extracted from a document (PDF/DOCX).
/// For formats without explicit page breaks (plain text, DOCX), the entire
/// document is represented as a single page with `number = 1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedPage {
    /// 1-indexed page number for display and linking.
    pub number: u32,
    /// Text content of this page.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOccurrence {
    pub before: String,
    pub matched: String,
    pub after: String,
    pub snippet: String,
    /// Page number where this occurrence was found (1-indexed).
    /// `None` for formats without explicit pages (plain text files).
    pub page: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    pub file_relative_path: String,
    pub file_absolute_path: PathBuf,
    pub contexts: Vec<String>,
    pub occurrences: Vec<SearchOccurrence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSettings {
    pub scan_path: PathBuf,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            scan_path: PathBuf::from("/home/documents"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchStatus {
    #[default]
    Idle,
    Scanning,
    Searching,
    Success,
    Error,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub settings: AppSettings,
    pub current_path: PathBuf,
    pub found_files: Vec<FileEntry>,
    pub search_query: String,
    pub search_results: Vec<SearchMatch>,
    pub results_offset: usize,
    pub results_limit: usize,
    pub status: SearchStatus,
    pub error: Option<String>,
    pub skipped_oversized_files: usize,
    pub skipped_unreadable_entries: usize,
    /// Active file extensions for scanning (e.g., ["docx", "pdf"]).
    /// Set via UI or CLI, passed explicitly to scanner to avoid env-var race.
    pub scan_extensions: Vec<String>,
}

impl AppState {
    pub fn new() -> Self {
        let settings = AppSettings::default();
        Self {
            current_path: settings.scan_path.clone(),
            settings,
            found_files: Vec::new(),
            search_query: String::new(),
            search_results: Vec::new(),
            results_offset: 0,
            results_limit: 100,
            status: SearchStatus::Idle,
            error: None,
            skipped_oversized_files: 0,
            skipped_unreadable_entries: 0,
            scan_extensions: vec!["docx".to_string(), "pdf".to_string()],
        }
    }

    pub fn set_scan_path(&mut self, path: PathBuf) {
        self.settings.scan_path = path.clone();
        self.current_path = path;
        self.found_files.clear();
        self.reset_search();
    }

    pub fn reset_search(&mut self) {
        self.search_results.clear();
        self.results_offset = 0;
        self.status = SearchStatus::Idle;
        self.error = None;
    }

    pub fn get_visible_results(&self) -> &[SearchMatch] {
        let end = self
            .results_offset
            .saturating_add(self.results_limit)
            .min(self.search_results.len());
        &self.search_results[self.results_offset.min(end)..end]
    }

    pub fn has_more_results(&self) -> bool {
        self.results_offset.saturating_add(self.results_limit) < self.search_results.len()
    }

    pub fn load_more(&mut self) {
        self.results_offset = self
            .results_offset
            .saturating_add(self.results_limit)
            .min(self.search_results.len());
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_never_exposes_out_of_bounds_slice() {
        let mut state = AppState::new();
        state.results_limit = 2;
        state.results_offset = 100;
        assert!(state.get_visible_results().is_empty());
        assert!(!state.has_more_results());
    }

    #[test]
    fn load_more_saturates_at_result_count() {
        let mut state = AppState::new();
        state.results_limit = 2;
        state.search_results = (0..3)
            .map(|index| SearchMatch {
                file_relative_path: format!("{index}.txt"),
                file_absolute_path: PathBuf::from(format!("{index}.txt")),
                contexts: Vec::new(),
                occurrences: Vec::new(),
            })
            .collect();
        state.load_more();
        state.load_more();
        assert_eq!(state.results_offset, 3);
        assert!(!state.has_more_results());
    }

    #[test]
    fn changing_scan_path_clears_old_index_and_results() {
        let mut state = AppState::new();
        state.found_files.push(FileEntry {
            relative_path: "old.txt".to_owned(),
            absolute_path: PathBuf::from("old.txt"),
        });
        state.search_query = "old".to_owned();
        state.set_scan_path(PathBuf::from("/tmp/new"));
        assert!(state.found_files.is_empty());
        assert!(state.search_results.is_empty());
        assert_eq!(state.current_path, PathBuf::from("/tmp/new"));
    }
}
