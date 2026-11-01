\# Software Requirements Specification (SRS)

\*\*Project:\*\* Document Searcher

\*\*Target Platform:\*\* ALT Linux (via Flatpak)

\*\*Architecture Pattern:\*\* MVVM (Model-View-ViewModel)



\## 1. Project Overview \& Scope

The Document Searcher is a native desktop application designed to perform deep, contextual substring searches across various local document formats. The MVP (Minimum Viable Product) will operate entirely offline, utilizing a sandboxed Flatpak environment. It prioritizes non-blocking UI responsiveness, asynchronous file processing, and unified text extraction strategies (native extraction for text-based files, OCR for images/scanned documents).



\## 2. Technology Stack \& Architecture

\*   \*\*Language:\*\* Rust (stable)

\*   \*\*UI Framework:\*\* GTK4, Libadwaita

\*   \*\*Architecture Framework:\*\* Relm4 (using `Component` trait for widget mutation)

\*   \*\*UI Layout:\*\* Blueprint (`.blp`) compiled to XML via `build.rs`

\*   \*\*Concurrency:\*\* `tokio` (async/await) with multithreading and `async-channel` for MVVM message passing.

\*   \*\*Text Extraction:\*\* `docx-rs` (.docx), `pdf-extract`/`lopdf` (.pdf), `leptess` (Tesseract Rust bindings for OCR).

\*   \*\*Distribution:\*\* Flatpak (offline build capability required).



\## 3. Functional Requirements (FR)



\*\*FR-1: Directory Management \& Sandboxing\*\*

\*   \*\*FR-1.1:\*\* The application must launch with a default scan directory of `/home/documents`.

\*   \*\*FR-1.2:\*\* The application must provide a UI button to change the target directory.

\*   \*\*FR-1.3:\*\* Directory selection must utilize the Flatpak `FileChooser` Portal (`xdg-desktop-portal`) to request user permission for directory access outside the sandbox.

\*   \*\*FR-1.4:\*\* The chosen directory path must be persisted across sessions using GSettings (`org.gtk.DocumentSearcher`).



\*\*FR-2: File Indexing \& Text Extraction\*\*

\*   \*\*FR-2.1:\*\* Upon directory selection, the system must recursively scan for supported formats: `.txt`, `.md`, `.rs`, `.toml`, `.json`, `.xml`, `.html`, `.css`, `.js`, `.ts`, `.pdf`, `.docx`, `.png`, `.jpg`, `.jpeg`, `.tiff`, `.bmp`.

\*   \*\*FR-2.2:\*\* The scanner must explicitly ignore hidden files/directories (starting with `.`) and must not follow symlinks to prevent infinite loops.

\*   \*\*FR-2.3:\*\* The scanner must skip any single file exceeding a 50MB size limit to prevent memory exhaustion.

\*   \*\*FR-2.4 (Extraction Strategy):\*\* Native text extraction must be used for text and office files. OCR (via Tesseract) must only be used as a primary strategy for image formats, and as a fallback strategy for PDFs lacking a native text layer.



\*\*FR-3: Search Execution \& Concurrency\*\*

\*   \*\*FR-3.1:\*\* The search algorithm must perform case-insensitive substring matching.

\*   \*\*FR-3.2:\*\* Search execution must be debounced by 300 milliseconds. Typing within this window resets the timer; execution only begins after the user pauses.

\*   \*\*FR-3.3:\*\* The search subsystem must support task cancellation. If a new search triggers while a previous search is processing, the active `tokio` worker task must be immediately aborted via a cancellation token to free CPU threads.

\*   \*\*FR-3.4:\*\* The application must format occurrences into a snippet containing up to 20 characters of preceding context, the exact matched substring, and up to 20 characters of succeeding context.



\*\*FR-4: User Interface (UI)\*\*

\*   \*\*FR-4.1:\*\* The UI must implement a 50/50 vertical split. 

&#x20;   \*   \*Left Pane:\* Displays the current absolute folder path, total recursive file count, and a scrollable list of found files formatted as relative paths.

&#x20;   \*   \*Right Pane:\* Implements a 50/50 horizontal split. Top section contains the search input. Bottom section contains a scrollable list of search results.

\*   \*\*FR-4.2:\*\* Search state must be visually communicated. A spinner must be visible during active scanning/searching. A success icon (green check) must appear upon completion. An error icon must appear upon failure.

\*   \*\*FR-4.3:\*\* In the results pane, the relative path of the matched file must be rendered in bold as a clickable hyperlink.

\*   \*\*FR-4.4:\*\* The matched substring within the context snippet must be visually highlighted (e.g., bolded or colored).



\*\*FR-5: System Integration \& Interactions\*\*

\*   \*\*FR-5.1:\*\* Clicking a file in either the Left Pane (File List) or Right Pane (Results List) must trigger a system command to open the file in the OS default application (e.g., Nautilus).

\*   \*\*FR-5.2:\*\* Row activation events must be resolved using deterministic identifiers (absolute `PathBuf` or UUID) mapped to the data model, rather than relying on UI list indices (`row.index()`), to prevent desynchronization during asynchronous updates.

\*   \*\*FR-5.3:\*\* File launching must be executed via `gio::AppInfo::launch\_default\_for\_uri` to ensure compatibility with Flatpak Portals.



\## 4. Non-Functional Requirements (NFR)



\*   \*\*NFR-1 (Performance):\*\* The main GTK event loop must never block. All I/O operations (scanning, extraction, OCR) must execute on a separate Tokio background thread.

\*   \*\*NFR-2 (Build Reproducibility):\*\* The Flatpak manifest must encapsulate all dependencies. Tesseract, Leptonica, and the required language data (`tessdata/eng.traineddata`) must be compiled from source or downloaded via manifest modules, rather than mapped from the host filesystem.

\*   \*\*NFR-3 (Resilience):\*\* Corrupted files, encrypted PDFs, or unsupported encoding must not crash the worker thread. The extractor must log the failure, gracefully skip the file, and continue the loop.

