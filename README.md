# Document Searcher

Desktop application for searching documents on ALT Linux. The project is being developed in stages: the document-search core and a small CLI can be exercised on Windows, while the GTK/Libadwaita interface, portals, OCR and Flatpak packaging require Linux (ultimately the ALT Linux VM).

## Core development on Windows

Install Rust stable (the repository pins Rust 1.78.0 in `src/rust-toolchain.toml`). Open a new PowerShell after installation so Cargo is on `PATH`. If it is not, use the temporary PATH setup shown below.

Run commands from the repository root with `--manifest-path`, or change directory to `src/` first:

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path
Set-Location .\src
cargo fmt --check
cargo test --lib --no-default-features
cargo clippy --lib --tests --no-default-features -- -D warnings
cargo run --no-default-features -- scan <directory>
cargo run --no-default-features -- search <directory> <query>
```

Equivalent commands from the repository root:

```powershell
cargo test --manifest-path .\src\Cargo.toml --lib --no-default-features
cargo run --manifest-path .\src\Cargo.toml --no-default-features -- scan <directory>
cargo run --manifest-path .\src\Cargo.toml --no-default-features -- search <directory> <query>
```

The default build excludes GTK and OCR native dependencies. It checks scanning, UTF-8 text/DOCX/PDF text-layer extraction, and Unicode-aware searching. Image OCR and scanned-PDF OCR require the `ocr` feature, Tesseract/Leptonica, language data, and `pdftoppm`; those should be validated in Linux. The GTK application is not currently a Windows target because GTK/Libadwaita/portal dependencies are Linux desktop dependencies.

## Linux / ALT Linux validation

From `src/`, validate the core first:

```sh
cargo fmt --check
cargo test --lib
cargo clippy --lib --tests -- -D warnings
```

The GUI build additionally requires GTK4, Libadwaita, Blueprint Compiler, GLib development tools, and the portal. Flatpak build instructions are in [`src/BUILD_INSTRUCTIONS.md`](src/BUILD_INSTRUCTIONS.md); the manifest and desktop application still need final integration and end-to-end verification.

## Specifications

See [`docs/specs/`](docs/specs/) for the subject-area analysis, requirements, and system design.

## Current development status

The core scanner, extractors, Unicode-aware matcher, Tokio worker, and Windows-friendly CLI are under active development. The GTK/Relm4 application, portal-based folder selection, GSettings persistence, and Flatpak packaging remain unfinished; do not treat the current CLI as the final desktop application.
