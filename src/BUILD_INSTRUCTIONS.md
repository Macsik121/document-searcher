# Document Searcher - Build & Test Instructions

## Prerequisites (on Alt Linux / WSL / Linux VM)

### System Dependencies

```bash
# On Alt Linux (or Fedora/RHEL-based):
sudo dnf install -y \
    gcc gcc-c++ make cmake \
    gtk4-devel libadwaita-devel \
    blueprint-compiler glib2-devel \
    tesseract-devel leptonica-devel \
    openssl-devel pkgconfig \
    flatpak flatpak-builder \
    cargo rustup

# On Ubuntu/Debian:
sudo apt install -y \
    build-essential cmake \
    libgtk-4-dev libadwaita-1-dev \
    blueprint-compiler libglib2.0-dev \
    libtesseract-dev libleptonica-dev \
    libssl-dev pkg-config \
    flatpak flatpak-builder \
    cargo rustup
```

### Rust Toolchain

```bash
rustup install 1.78.0
rustup default 1.78.0
rustup component add rustfmt clippy
```

### Tesseract Language Data

```bash
# Install language data for OCR
sudo dnf install -y tesseract-langpack-eng tesseract-langpack-rus
# OR manually download to /usr/share/tessdata/
```

---

## Building Natively (Development)

### 1. Clone/Navigate to Project

```bash
cd document_searcher
```

### 2. Build Blueprint Resources

```bash
# Compile .blp to .ui
blueprint-compiler compile --output data/resources/window.ui data/resources/window.blp

# Compile GResource
glib-compile-resources --target data/resources/org.example.DocumentSearcher.gresource \
    --sourcedir data/resources \
    data/resources/gresource.xml
```

### 3. Build with Cargo

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run
cargo run
# OR
./target/release/document_searcher
```

---

## Building Flatpak (Production)

### 1. Generate Cargo Sources for Offline Build

```bash
# From project root
cargo generate-lockfile
cargo install cargo-flatpak-generator
cargo flatpak-generator Cargo.lock -o flatpak/generated-sources.json
```

### 2. Build Flatpak

```bash
# Clean build
flatpak-builder --force-clean --install-deps-from=flathub \
    flatpak/build-dir flatpak/org.example.DocumentSearcher.json

# Install locally
flatpak-builder --force-clean --install-deps-from=flathub --user \
    --install flatpak/build-dir flatpak/org.example.DocumentSearcher.json
```

### 3. Run Flatpak

```bash
flatpak run org.example.DocumentSearcher
```

### 4. Create Flatpak Bundle (for distribution)

```bash
flatpak build-bundle --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
    flatpak/build-dir document_searcher.flatpak org.example.DocumentSearcher
```

---

## Testing Checklist

### Functional Tests

| Test | Expected Result |
|------|-----------------|
| Launch app | Window opens with "Document Searcher" title |
| Default folder | Shows "Folder: /home/documents" |
| Change Folder button | Opens portal file chooser |
| Select folder | Updates folder label, scans files |
| File count label | Shows "N files found" |
| File list | Shows relative paths (e.g., `presentations/day_of_opened_doors.pdf`) |
| Search input | Placeholder: "Enter the search string..." |
| Type in search | After 300ms debounce, spinner appears |
| Search completes | Green "Search complete" indicator |
| Results list | Numbered items with bold file names |
| Click file link | Opens in nautilus/file manager |
| Show more button | Loads 100 more results, reappears |
| Context preview | Shows `...20 before + match + 20 after...` |

### Edge Cases

- Empty folder → shows "0 files found"
- No matches → empty results list
- Very large files → doesn't crash (streaming read)
- Special characters in filenames → handled correctly
- Cancel folder selection → keeps previous folder
- Rapid typing → only searches after 300ms pause

---

## Troubleshooting

### Blueprint Compilation Fails

```bash
# Ensure blueprint-compiler is installed
blueprint-compiler --version
# Should be 0.9.x
```

### GResource Not Found

```bash
# Check file exists
ls -la data/resources/org.example.DocumentSearcher.gresource

# Re-run build.rs manually
cargo build  # This runs build.rs automatically
```

### Tesseract Not Found

```bash
# Verify installation
tesseract --version
# Check tessdata
ls /usr/share/tessdata/eng.traineddata
```

### Flatpak Build Fails

```bash
# Check generated-sources.json exists
ls -la flatpak/generated-sources.json

# Clean and rebuild
rm -rf flatpak/build-dir
flatpak-builder --force-clean ...
```

### Portal Errors (File Chooser)

- Ensure `xdg-desktop-portal-gtk` is installed
- Run with `--filesystem=host` temporarily for debugging (remove for production)

---

## Project Structure Reference

```
document_searcher/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── build.rs
├── data/
│   ├── resources/
│   │   ├── window.blp
│   │   ├── window.ui (generated)
│   │   ├── gresource.xml
│   │   ├── styles.css
│   │   └── org.example.DocumentSearcher.gresource (generated)
│   ├── org.example.DocumentSearcher.desktop
│   └── org.example.DocumentSearcher.metainfo.xml
├── flatpak/
│   ├── org.example.DocumentSearcher.json
│   └── generated-sources.json
└── src/
    ├── main.rs
    ├── app.rs
    ├── utils.rs
    ├── model/
    │   ├── mod.rs
    │   ├── state.rs
    │   ├── actions.rs
    │   ├── scanner.rs
    │   ├── extractor.rs
    │   └── search.rs
    └── widgets/
        ├── mod.rs
        ├── sidebar.rs
        ├── search_bar.rs
        └── results_list.rs
```

---

## Development Workflow

1. **Edit UI**: Modify `data/resources/window.blp`
2. **Rebuild resources**: Run `cargo build` (build.rs handles this)
3. **Edit logic**: Modify files in `src/model/` and `src/widgets/`
4. **Test**: `cargo run`
5. **Format**: `cargo fmt`
6. **Lint**: `cargo clippy`

---

## ALT Linux Specific Notes

- Use `org.freedesktop.Platform//23.08` runtime (ALT's current LTS)
- Ensure `.desktop` file has correct `Categories=Utility;TextTools;Office;`
- MIME types in `.desktop` match supported extensions
- AppStream metadata includes Russian localization if needed
- SELinux: Flatpak sandbox handles this automatically