# Document Searcher

Document Searcher is a Rust document-search application for ALT Linux. The
feature-free core and CLI can be exercised on Windows; the GTK/Libadwaita UI,
portals, GSettings runtime, OCR, and Flatpak are Linux-only and must be
validated in the ALT Linux VM.

## Windows core workflow

The default feature set is intentionally empty. From the repository root, use
only `--no-default-features` on Windows:

```powershell
cargo fmt --check --manifest-path .\src\Cargo.toml
cargo test --manifest-path .\src\Cargo.toml --lib --no-default-features
cargo clippy --manifest-path .\src\Cargo.toml --lib --tests --no-default-features -- -D warnings
cargo run --manifest-path .\src\Cargo.toml --no-default-features -- scan <directory>
cargo run --manifest-path .\src\Cargo.toml --no-default-features -- search <directory> <query>
```

The CLI scans supported files, reports skipped oversized/unreadable entries,
and searches extracted text with Unicode-aware case-insensitive matching and up
to 20 characters of context on each side. The core includes text, DOCX, and
PDF text-layer extraction. Image and scanned-PDF OCR requires the Linux `ocr`
/ `desktop` build, Tesseract, Leptonica, `pdftoppm`, and matching tessdata.

Do not run the `desktop` feature, native GUI smoke tests, OCR integration, or
Flatpak commands on Windows. A successful core check does not verify GTK,
portal, GSettings, OCR, or Flatpak behavior.

## Linux / ALT Linux workflow

The Linux desktop build requires GTK4, Libadwaita, GLib development tools,
Tesseract/Leptonica, `pdftoppm`, pkg-config, and the ashpd portal runtime.
Blueprint Compiler is not required: the UI is built procedurally in
`src/src/app.rs`. During a Linux desktop build, `build.rs` automatically runs
`glib-compile-resources` for `data/resources/gresource.xml` and
`glib-compile-schemas` for the GSettings schema. The CSS resource is embedded
in the binary, so no `.gresource` installation step is needed.

From `src/` in the ALT Linux VM:

```sh
cargo fmt --check
cargo test --lib --no-default-features
cargo clippy --lib --tests --no-default-features -- -D warnings
cargo build --release --features desktop
cargo run --release --features desktop
```

The native desktop checks must cover startup, embedded CSS, default
`/home/documents`, GSettings `scan-path` persistence, portal folder selection,
scan/search, pagination, cancellation, and GIO file opening. Record them
separately from Windows core validation.

## Flatpak workflow (ALT Linux VM only)

The sole manifest is
[`src/flatpak/org.gtk.DocumentSearcher.json`](src/flatpak/org.gtk.DocumentSearcher.json).
After the lockfile is stable, generate its offline Cargo sources in the Linux
VM; do not create or claim this generated file on Windows:

```sh
cd src
cargo flatpak-generator --output flatpak/generated-sources.json
flatpak-builder --force-clean --install-deps-from=flathub \
  flatpak/build-dir flatpak/org.gtk.DocumentSearcher.json
flatpak-builder --force-clean --user --install \
  flatpak/build-dir flatpak/org.gtk.DocumentSearcher.json
flatpak run org.gtk.DocumentSearcher
```

The manifest installs desktop metadata and the GSettings schema. The
GResource is embedded in the binary and is not copied separately. Verify the
portal permissions, schema lookup, CSS lookup, OCR runtime, and file opening
inside the Flatpak before treating packaging as complete.

The extractor default historically uses `eng+rus`, while the current manifest
sets `DOCUMENT_SEARCHER_OCR_LANG=eng` and supplies English tessdata. This
mismatch is documented and must be resolved only after a Linux OCR check.

## Specifications and architecture

- [Specifications](docs/specs/)
- [Architecture documentation](docs/architecture/)
- [ALT Linux run guide](docs/ALT-LINUX-RUN.md)
- [Build instructions](src/BUILD_INSTRUCTIONS.md)

`docs/specs/requirements-spec.md` is the user-owned requirements source and is
not rewritten by implementation work.

## Current status

The scanner, extractors, Unicode-aware matcher, Tokio worker, and CLI are
available for core checks. Desktop integration is implemented in the current
source path but remains unverified until the ALT Linux VM checks are run.
