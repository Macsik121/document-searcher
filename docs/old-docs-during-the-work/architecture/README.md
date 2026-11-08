# Architecture overview

Document Searcher has two entry points behind Cargo feature gates:

- **Core/CLI**: the default, feature-free build exposes scanning, extraction,
  Unicode-aware search, and the `scan`/`search` commands. It is the scope that
  can be exercised on Windows with `--no-default-features`.
- **Desktop**: the `desktop` feature adds GTK4/Libadwaita, ashpd portal
  integration, GSettings, OCR, and a procedural UI. It requires a Linux
  GTK/GLib toolchain and is validated in the ALT Linux VM.
- **Flatpak**: `src/flatpak/org.gtk.DocumentSearcher.json` packages the Linux
  desktop binary and native OCR runtime. The CSS GResource is embedded in the
  binary; the manifest installs metadata and the compiled GSettings schema but
  does not install a standalone `.gresource` file.

The runnable architecture is documented in:

- [Component architecture](component-architecture.md)
- [Scan and search sequence](scan-search-sequence.md)
- [User flow](user-flow.md)
- [Deployment](deployment.md)
- [State and data model](state-and-data-model.md)
- [Assumptions and gaps](assumptions-gaps.md)

## Verification matrix

| Area | Windows core | ALT Linux VM |
| --- | --- | --- |
| Scanner, extraction without OCR, search, worker tests | Supported with `--no-default-features` | Also available |
| `cargo fmt` and core Clippy | Supported with `--no-default-features` | Also available |
| CLI `scan`/`search` smoke | Supported with `--no-default-features` | Also available |
| GTK/Libadwaita desktop build | **Not run** | Required |
| Portal folder chooser and file opening | **Not run** | Required |
| GSettings schema discovery/persistence | **Not run** | Required |
| Tesseract image/scanned-PDF OCR | **Not run** | Required |
| Cargo source generation and Flatpak build/run | **Not run** | Required |

Windows core checks do not verify native GUI, portal, GSettings, OCR, or
Flatpak behavior.
