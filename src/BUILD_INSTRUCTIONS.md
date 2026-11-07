# Document Searcher build and test instructions

## Windows: core only

Windows checks must use `--no-default-features`; do not enable `desktop` or
`ocr` on Windows. From the repository root:

```powershell
cargo fmt --check --manifest-path .\src\Cargo.toml
cargo test --manifest-path .\src\Cargo.toml --lib --no-default-features
cargo clippy --manifest-path .\src\Cargo.toml --lib --tests --no-default-features -- -D warnings
cargo run --manifest-path .\src\Cargo.toml --no-default-features -- scan <directory>
cargo run --manifest-path .\src\Cargo.toml --no-default-features -- search <directory> <query>
```

These checks cover the feature-free CLI and core only. They do not validate
GTK/Libadwaita, portal behavior, GSettings runtime, OCR, or Flatpak. If Windows
policy blocks Cargo build scripts, record the exact error and rerun the checks
in the ALT Linux VM rather than marking them passed.

## Linux native prerequisites

The desktop build needs a Linux environment with Rust, a C toolchain,
`pkg-config`, GTK4/Libadwaita development packages, GLib tools including
`glib-compile-resources` and `glib-compile-schemas`, Tesseract/Leptonica,
`pdftoppm`, and a desktop portal runtime. Package names vary between ALT Linux
releases; use the package names available in the target VM.

Blueprint Compiler is not required. The runnable UI is constructed
procedurally in `src/src/app.rs`. On a Linux desktop build, `build.rs`
automatically compiles `data/resources/gresource.xml` and the GSettings schema.
The compiled CSS resource is embedded into the application binary at build
time; the runtime does not load it from Cargo's `OUT_DIR` and Flatpak does not
install a separate `.gresource` file.

From `src/` in the ALT Linux VM:

```sh
cargo fmt --check
cargo test --lib --no-default-features
cargo clippy --lib --tests --no-default-features -- -D warnings
cargo build --release --features desktop
cargo run --release --features desktop
```

The schema is compiled into the build output for development. For installed
packages it is installed under `/app/share/glib-2.0/schemas` and compiled by
the Flatpak manifest. Check schema discovery, embedded CSS lookup, portal
folder selection/cancellation, scan/search, pagination, GIO file opening, and
OCR separately in the Linux environment.

## Flatpak build (ALT Linux VM only)

The single current manifest is
`flatpak/org.gtk.DocumentSearcher.json`. After dependency versions and
`Cargo.lock` are stable, generate offline Cargo sources in the Linux VM:

```sh
cd src
cargo flatpak-generator --output flatpak/generated-sources.json
flatpak-builder --force-clean --install-deps-from=flathub \
  flatpak/build-dir flatpak/org.gtk.DocumentSearcher.json
flatpak-builder --force-clean --user --install \
  flatpak/build-dir flatpak/org.gtk.DocumentSearcher.json
flatpak run org.gtk.DocumentSearcher
```

Do not hand-create `generated-sources.json` on Windows or report the Flatpak as
verified until `flatpak-builder` and `flatpak run` have completed in the VM.
The manifest installs the desktop entry, AppStream metadata, and schema; CSS
is embedded in the binary. The manifest currently supplies English tessdata
and sets `DOCUMENT_SEARCHER_OCR_LANG=eng`, while the extractor fallback is
`eng+rus`; resolve that mismatch only after a Linux OCR check.

## Linux smoke checklist

- Launch the native desktop app and Flatpak app.
- Confirm CSS loads from the embedded GResource.
- Confirm default path `/home/documents`, then select and persist another path.
- Confirm folder chooser cancellation preserves the prior selection.
- Scan and search a folder, exercise cancellation and pagination, and open a
  result through GIO/portal integration.
- Verify image and scanned-PDF OCR against the installed tessdata.
- Confirm schema lookup and Flatpak filesystem/portal permissions.

## Current source layout

```text
src/
  Cargo.toml
  Cargo.lock
  build.rs
  data/
    org.gtk.DocumentSearcher.desktop
    org.gtk.DocumentSearcher.gschema.xml
    org.gtk.DocumentSearcher.metainfo.xml
    resources/
      gresource.xml
      styles.css
  flatpak/
    org.gtk.DocumentSearcher.json
    generated-sources.json  # generated in the Linux VM
  src/
    main.rs
    app.rs
    lib.rs
    model/
      actions.rs
      extractor.rs
      scanner.rs
      search.rs
      state.rs
      worker.rs
```
