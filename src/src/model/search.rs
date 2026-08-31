use std::path::Path;
use crate::model::state::{FileEntry, SearchMatch};
use crate::model::extractor::extract_text;

const CONTEXT_CHARS: usize = 20;

pub fn search_files(files: &[FileEntry], query: &str) -> anyhow::Result<Vec<SearchMatch>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    for file_entry in files {
        let text = match extract_text(&file_entry.absolute_path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Failed to extract text from {}: {}", file_entry.relative_path, e);
                continue;
            }
        };

        let matches = find_matches_with_context(&text, &query_lower);
        if !matches.is_empty() {
            results.push(SearchMatch {
                file_relative_path: file_entry.relative_path.clone(),
                contexts: matches,
            });
        }
    }

    Ok(results)
}

fn find_matches_with_context(text: &str, query: &str) -> Vec<String> {
    let text_lower = text.to_lowercase();
    let mut contexts = Vec::new();
    let mut start = 0;

    while let Some(pos) = text_lower[start..].find(query) {
        let actual_pos = start + pos;
        let match_end = actual_pos + query.len();

        // Get 20 chars before
        let context_start = actual_pos.saturating_sub(CONTEXT_CHARS);
        let before = &text[context_start..actual_pos];

        // Get 20 chars after
        let context_end = (match_end + CONTEXT_CHARS).min(text.len());
        let after = &text[match_end..context_end];

        let matched_text = &text[actual_pos..match_end];

        let context = format!("...{}{}{}", before, matched_text, after);
        contexts.push(context);

        start = match_end;
    }

    contexts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::state::FileEntry;
    use std::path::PathBuf;

    #[test]
    fn test_find_matches_with_context() {
        let text = "This is a test document with some searchable content for testing.";
        let query = "search";

        let contexts = find_matches_with_context(text, query);
        assert_eq!(contexts.len(), 1);
        assert!(contexts[0].contains("search"));
        assert!(contexts[0].contains("..."));
    }

    #[test]
    fn test_multiple_matches() {
        let text = "test one test two test three";
        let query = "test";

        let contexts = find_matches_with_context(text, query);
        assert_eq!(contexts.len(), 3);
    }

    #[test]
    fn test_context_length() {
        let text = "A very long text with a match in the middle of it all";
        let query = "match";

        let contexts = find_matches_with_context(text, query);
        assert_eq!(contexts.len(), 1);

        // Should have approximately 20 chars before and after
        let context = &contexts[0];
        let match_pos = context.find("match").unwrap();
        assert!(match_pos <= CONTEXT_CHARS + 3); // +3 for "..."
    }
}