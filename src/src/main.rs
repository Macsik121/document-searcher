use document_searcher::model;
use std::env;
use std::path::Path;
use std::process::ExitCode;

use model::scanner::scan_directory_with_report;
use model::search::search_files;

fn print_help(program: &str) {
    println!(
        "Document Searcher\n\n\
         Usage:\n  {program} scan <directory>\n  {program} search <directory> <query>\n\n\
         The Windows build runs the document-search core without the GTK/Flatpak UI.\n\
         Supported formats: txt, md, rs, toml, json, xml, html, css, js, ts, pdf,\n\
         docx, png, jpg, jpeg, tiff, bmp."
    );
}

fn print_scan(root: &Path) -> anyhow::Result<()> {
    let report = scan_directory_with_report(root)?;
    println!("Folder: {}", root.display());
    println!("Files found: {}", report.files.len());
    if report.oversized_files > 0 {
        println!("Skipped oversized files: {}", report.oversized_files);
    }
    if report.unreadable_entries > 0 {
        println!("Skipped unreadable entries: {}", report.unreadable_entries);
    }
    for file in report.files {
        println!("{}", file.relative_path);
    }
    Ok(())
}

fn print_search(root: &Path, query: &str) -> anyhow::Result<()> {
    let report = scan_directory_with_report(root)?;
    let results = search_files(&report.files, query)?;
    println!("Matches for {query:?}: {} files", results.len());
    for result in results {
        println!("\n{}", result.file_relative_path);
        for occurrence in result.occurrences {
            println!("  {}", occurrence.snippet);
        }
    }
    Ok(())
}

fn run() -> anyhow::Result<()> {
    let mut args = env::args();
    let program = args.next().unwrap_or_else(|| "document-searcher".to_owned());
    let Some(command) = args.next() else {
        print_help(&program);
        return Ok(());
    };

    match command.as_str() {
        "scan" => {
            let Some(directory) = args.next() else {
                print_help(&program);
                anyhow::bail!("scan requires a directory");
            };
            if args.next().is_some() {
                print_help(&program);
                anyhow::bail!("scan accepts exactly one directory");
            }
            print_scan(Path::new(&directory))
        }
        "search" => {
            let (Some(directory), Some(query)) = (args.next(), args.next()) else {
                print_help(&program);
                anyhow::bail!("search requires a directory and a query");
            };
            if args.next().is_some() {
                print_help(&program);
                anyhow::bail!("search accepts exactly one directory and one query");
            }
            print_search(Path::new(&directory), &query)
        }
        "--help" | "-h" => {
            print_help(&program);
            Ok(())
        }
        other => {
            print_help(&program);
            anyhow::bail!("unknown command: {other}");
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
