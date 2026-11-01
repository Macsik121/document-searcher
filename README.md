# document-searcher

The Document Searcher App for Alt Linux with graphical interface designed to solve my mother's industrial case.



\### Validation Step (Your Next Steps)



(all of the necessary files are edited)



You now have the structural perimeter of the application built. Run the following commands in your terminal to validate the sandbox:



1\. `cargo generate-lockfile` (This ensures you have a `Cargo.lock` which is required for flatpak Rust builds).

2\. `cargo install cargo-flatpak-generator` (If you haven't already).

3\. `cargo flatpak-generator --output flatpak/generated-sources.json` (This parses your `Cargo.lock` and creates a list of crates for Flatpak to download offline).

4\. \*\*Link the generated sources:\*\* Add `{ "type": "file", "path": "generated-sources.json" }` into the `sources` array of the `document-searcher` module in your JSON manifest.

5\. \*\*Build the container:\*\* `flatpak-builder --force-clean --user build-dir flatpak/org.gtk.DocumentSearcher.json`



Once that successfully compiles, the C-dependencies are locked in. We can then confidently write the \*\*Tokio Worker \& Debouncer\*\* (the backend logic) knowing the OCR extraction calls will safely compile.

