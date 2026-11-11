use std::env;
use std::path::Path;
use std::process::ExitCode;

use document_searcher::model;
use model::scanner::{scan_directory_with_extensions, scan_directory_with_report};
use model::search::search_files_streaming;

#[cfg(feature = "desktop")]
#[path = "app.rs"]
mod app;

#[cfg(all(feature = "windows-desktop", not(feature = "desktop")))]
#[path = "app_win.rs"]
mod app_win;

fn print_help(program: &str) {
    println!(
        "Document Searcher (.docx & .pdf)\n\n\
         Usage:\n\
         \x20 {program} [directory]                       Open Desktop UI (in directory or current folder)\n\
         \x20 {program} gui [directory]                   Open Desktop UI in specified directory\n\
         \x20 {program} scan <directory> [--ext docx,pdf] Scan directory in console\n\
         \x20 {program} search <directory> <query>        Search documents in console\n\n\
         By default, only .docx and .pdf files are indexed."
    );
}

fn parse_ext_flag(args: &mut Vec<String>) -> anyhow::Result<Option<Vec<String>>> {
    if let Some(pos) = args.iter().position(|arg| arg == "--ext") {
        args.remove(pos);
        let Some(raw) = args.get(pos).cloned() else {
            anyhow::bail!("--ext requires a comma-separated list of extensions (e.g. docx,pdf)");
        };
        args.remove(pos);
        let list: Vec<String> = raw
            .split(',')
            .map(|item| item.trim().trim_start_matches('.').to_owned())
            .filter(|item| !item.is_empty())
            .collect();
        if list.is_empty() {
            anyhow::bail!("--ext list cannot be empty");
        }
        return Ok(Some(list));
    }
    Ok(None)
}

fn print_scan(root: &Path, extensions: Option<&[String]>) -> anyhow::Result<()> {
    let report = match extensions {
        Some(ext) => scan_directory_with_extensions(root, ext)?,
        None => scan_directory_with_report(root)?,
    };
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

fn print_search(root: &Path, query: &str, extensions: Option<&[String]>) -> anyhow::Result<()> {
    let report = match extensions {
        Some(ext) => scan_directory_with_extensions(root, ext)?,
        None => scan_directory_with_report(root)?,
    };
    // Stream matches to stdout as each file finishes searching, instead of
    // buffering the entire corpus and printing only at the end. The header is
    // printed first so the user sees that search has started; the trailing
    // total is printed once the core reports completion.
    println!("Searching {query:?} across {} files…", report.files.len());
    let mut stdout = std::io::BufWriter::new(std::io::stdout());
    use std::io::Write;
    let reported = search_files_streaming(&report.files, query, |result| {
        writeln!(stdout, "\n{}", result.file_relative_path)?;
        for occurrence in &result.occurrences {
            let page_info = occurrence
                .page
                .map(|page| format!("  (page {page})"))
                .unwrap_or_default();
            writeln!(stdout, "{page_info}  {}", occurrence.snippet)?;
        }
        stdout.flush()?;
        Ok(())
    })?;
    stdout.flush()?;
    println!("\nMatches for {query:?}: {reported} files");
    Ok(())
}

fn run_cli_or_gui() -> anyhow::Result<()> {
    let mut args: Vec<String> = env::args().collect();
    let program = if args.is_empty() {
        "document-searcher".to_owned()
    } else {
        args.remove(0)
    };

    let extensions = parse_ext_flag(&mut args)?;
    if let Some(ext_list) = &extensions {
        env::set_var("DOCUMENT_SEARCHER_EXTENSIONS", ext_list.join(","));
    }

    if args.is_empty() {
        #[cfg(feature = "desktop")]
        {
            app::run();
            return Ok(());
        }
        #[cfg(all(feature = "windows-desktop", not(feature = "desktop")))]
        {
            app_win::run();
            return Ok(());
        }
        #[cfg(not(any(feature = "desktop", feature = "windows-desktop")))]
        {
            print_help(&program);
            return Ok(());
        }
    }

    match args[0].as_str() {
        "scan" => {
            if args.len() != 2 {
                print_help(&program);
                anyhow::bail!("scan requires a <directory>");
            }
            print_scan(Path::new(&args[1]), extensions.as_deref())
        }
        "search" => {
            if args.len() != 3 {
                print_help(&program);
                anyhow::bail!("search requires <directory> and <query>");
            }
            print_search(Path::new(&args[1]), &args[2], extensions.as_deref())
        }
        "gui" => {
            #[cfg(all(feature = "windows-desktop", not(feature = "desktop")))]
            {
                if let Some(dir) = args.get(1) {
                    env::set_var("DOCUMENT_SEARCHER_INITIAL_DIR", dir);
                }
                app_win::run();
                Ok(())
            }
            #[cfg(feature = "desktop")]
            {
                app::run();
                Ok(())
            }
            #[cfg(not(any(feature = "desktop", feature = "windows-desktop")))]
            {
                anyhow::bail!("GUI requires --features windows-desktop");
            }
        }
        "--help" | "-h" => {
            print_help(&program);
            Ok(())
        }
        other => {
            let candidate_path = Path::new(other);
            if candidate_path.is_dir() {
                #[cfg(all(feature = "windows-desktop", not(feature = "desktop")))]
                {
                    env::set_var("DOCUMENT_SEARCHER_INITIAL_DIR", other);
                    app_win::run();
                    return Ok(());
                }
            }
            print_help(&program);
            anyhow::bail!("unknown command or non-existent directory: {other}");
        }
    }
}

fn main() -> ExitCode {
    match run_cli_or_gui() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
