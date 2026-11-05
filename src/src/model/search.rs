use crate::model::extractor::extract_text;
use crate::model::state::{FileEntry, SearchMatch, SearchOccurrence};

const CONTEXT_CHARS: usize = 20;

#[derive(Debug, Clone)]
struct FoldedText {
    chars: Vec<char>,
    source_spans: Vec<(usize, usize)>,
}

impl FoldedText {
    fn new(text: &str) -> Self {
        let mut chars = Vec::new();
        let mut source_spans = Vec::new();
        for (start, source_char) in text.char_indices() {
            let end = start + source_char.len_utf8();
            for folded_char in source_char.to_lowercase() {
                chars.push(folded_char);
                source_spans.push((start, end));
            }
        }
        Self { chars, source_spans }
    }
}
pub fn search_files(files: &[FileEntry], query: &str) -> anyhow::Result<Vec<SearchMatch>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let folded_query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if folded_query.is_empty() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    for file in files {
        let text = match extract_text(&file.absolute_path) {
            Ok(text) => text,
            Err(error) => {
                tracing::warn!(
                    path = %file.relative_path,
                    error = %error,
                    "Skipping file because text extraction failed"
                );
                continue;
            }
        };

        let occurrences = find_occurrences_with_context(&text, &folded_query);
        if occurrences.is_empty() {
            continue;
        }

        results.push(SearchMatch {
            file_relative_path: file.relative_path.clone(),
            file_absolute_path: file.absolute_path.clone(),
            contexts: occurrences.iter().map(|item| item.snippet.clone()).collect(),
            occurrences,
        });
    }
    Ok(results)
}

fn find_occurrences_with_context(text: &str, folded_query: &[char]) -> Vec<SearchOccurrence> {
    let folded_text = FoldedText::new(text);
    if folded_query.len() > folded_text.chars.len() {
        return Vec::new();
    }

    let char_boundaries: Vec<usize> = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect();
    let mut occurrences = Vec::new();
    let mut search_from = 0;

    while search_from + folded_query.len() <= folded_text.chars.len() {
        let found = (search_from..=folded_text.chars.len() - folded_query.len())
            .find(|start| folded_text.chars[*start..*start + folded_query.len()] == *folded_query);
        let Some(folded_start) = found else {
            break;
        };

        let folded_end = folded_start + folded_query.len();
        let source_start = folded_text.source_spans[folded_start].0;
        let source_end = folded_text.source_spans[folded_end - 1].1;
        let starts_at_source_boundary = folded_start == 0
            || folded_text.source_spans[folded_start - 1].1 != source_start;
        let ends_at_source_boundary = folded_end == folded_text.source_spans.len()
            || folded_text.source_spans[folded_end].0 != source_end;
        if !starts_at_source_boundary || !ends_at_source_boundary {
            search_from = folded_start + 1;
            continue;
        }

        let start_char = char_boundaries.binary_search(&source_start).unwrap();
        let end_char = char_boundaries.binary_search(&source_end).unwrap();
        let context_start = char_boundaries[start_char.saturating_sub(CONTEXT_CHARS)];
        let context_end = char_boundaries[(end_char + CONTEXT_CHARS).min(char_boundaries.len() - 1)];
        let before = text[context_start..source_start].to_owned();
        let matched = text[source_start..source_end].to_owned();
        let after = text[source_end..context_end].to_owned();

        occurrences.push(SearchOccurrence {
            snippet: format!("{before}{matched}{after}"),
            before,
            matched,
            after,
        });
        search_from = folded_start + folded_query.len();
    }

    occurrences
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn search_text(text: &str, query: &str) -> Vec<SearchOccurrence> {
        let folded_query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
        find_occurrences_with_context(text, &folded_query)
    }

    #[test]
    fn matches_without_case_sensitivity() {
        let occurrences = search_text("A Searchable word", "search");
        assert_eq!(occurrences.len(), 1);
        assert_eq!(occurrences[0].matched, "Search");
    }

    #[test]
    fn handles_unicode_offsets_and_context() {
        let text = "Привет 🌍, МИР!";
        let occurrences = search_text(text, "мир");
        assert_eq!(occurrences.len(), 1);
        assert_eq!(occurrences[0].matched, "МИР");
        assert_eq!(occurrences[0].snippet, text);
    }

    #[test]
    fn creates_context_limited_to_twenty_characters() {
        let text = format!("{}MATCH{}", "a".repeat(30), "b".repeat(30));
        let occurrences = search_text(&text, "match");
        assert_eq!(occurrences.len(), 1);
        assert_eq!(occurrences[0].before.chars().count(), CONTEXT_CHARS);
        assert_eq!(occurrences[0].after.chars().count(), CONTEXT_CHARS);
    }

    #[test]
    fn finds_multiple_non_overlapping_matches() {
        assert_eq!(search_text("test one test two test", "test").len(), 3);
        assert_eq!(search_text("banana", "ana").len(), 1);
    }

    #[test]
    fn empty_query_has_no_matches() {
        assert!(search_text("some text", "").is_empty());
    }

    #[test]
    fn search_skips_files_with_extraction_errors() {
        let root = tempdir().unwrap();
        let unsupported = root.path().join("bad.odt");
        fs::write(&unsupported, "irrelevant").unwrap();
        let results = search_files(
            &[FileEntry {
                relative_path: "bad.odt".to_owned(),
                absolute_path: unsupported,
            }],
            "needle",
        )
        .unwrap();
        assert!(results.is_empty());
    }
}
