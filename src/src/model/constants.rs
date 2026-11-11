/// Extensions that the scanner and extractor can process.
pub const ALL_SUPPORTED_EXTENSIONS: &[&str] = &[
    "docx", "pdf", "txt", "md", "rs", "toml", "json", "xml", "html", "css", "js", "ts",
];

/// Extensions handled as decoded plain text rather than a document container.
pub const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "rs", "toml", "json", "xml", "html", "css", "js", "ts",
];

pub const DEFAULT_EXTENSIONS: &[&str] = &["docx", "pdf"];
