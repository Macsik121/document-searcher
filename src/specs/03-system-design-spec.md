\# Stage 3: System Design \& Architecture Blueprint

\*\*Project:\*\* Document Searcher

\*\*Architecture Pattern:\*\* Async MVVM (Model-View-ViewModel)

\*\*Document Status:\*\* Finalized (Extended Edition)



\## 1. System Architecture Overview

To satisfy non-blocking UI constraints (NFR-1), the application utilizes a dual-runtime architecture:

1\.  \*\*GLib/GTK Main Loop (Relm4):\*\* Synchronous, handles widget rendering, state mutation, and user interactions.

2\.  \*\*Tokio Async Runtime:\*\* Asynchronous, handles disk I/O, heavy CPU tasks (OCR), and file parsing via background worker threads.



Communication between these runtimes is strictly unidirectional and message-driven via `tokio::sync::mpsc` (UI -> Worker) and `relm4::ComponentSender` (Worker -> UI).



\## 2. Module Specifications



\### 2.1 View Module (UI \& Layout)

\*   \*\*Technology:\*\* Blueprint (`.blp`), GTK4, Libadwaita.

\*   \*\*Structure:\*\*

&#x20;   \*   `AppWindow`: Main container (`AdwApplicationWindow`).

&#x20;   \*   `FilesFactory`: Relm4 `FactoryComponent` rendering the left-pane list of scanned files.

&#x20;   \*   `ResultsFactory`: Relm4 `FactoryComponent` rendering the right-pane list of formatted search occurrences.



\### 2.2 ViewModel / State Module (Single Source of Truth)

\*   \*\*Core Struct:\*\* `AppState`

\*   \*\*Constraint:\*\* The UI state must never rely on integer indices (`row.index()`) for mapping to data, as background tasks can mutate data while the UI renders.

\*   \*\*Schema:\*\*

&#x20;   ```rust

&#x20;   pub struct AppState {

&#x20;       pub settings: AppSettings, // Persisted via GSettings

&#x20;       pub found\_documents: HashMap<PathBuf, FoundDocument>, // Primary Key: PathBuf

&#x20;       pub matched\_documents: Vec<MatchedDocument>,

&#x20;       pub search\_status: SearchStatus, // enum: Idle, Searching, Success, Error(String)

&#x20;       pub search\_query: String,

&#x20;   }

&#x20;   ```



\### 2.3 Extractor Module (Strategy Pattern)

Handles text extraction based on file MIME type, resolving the OCR misuse risk (identified in Stage 1).

\*   \*\*`extract\_native\_text()`\*\*: Uses `std::fs` for `.txt`, and `docx-rs` for `.docx`. Fast, CPU-light.

\*   \*\*`extract\_pdf()`\*\*: Uses `pdf-extract`. If no text layer is found (scanned PDF), cascades to `extract\_ocr()`.

\*   \*\*`extract\_ocr()`\*\*: Uses `leptess` (Tesseract). Slow, CPU-heavy. Processes `.png`, `.jpg`, `.tiff`.



\### 2.4 Worker Module (Concurrency \& IPC)

This module acts as the backend service. It receives `WorkerMsg` commands from the UI.

\*   \*\*Debouncer Implementation:\*\* When `SearchQueryChanged` is received, the worker spawns a `tokio::time::sleep(300ms)` task.

\*   \*\*Cancellation Token:\*\* If a new keystroke arrives before 300ms expires, a `tokio\_util::sync::CancellationToken` is fired, aborting the sleeping task and preventing the OCR pipeline from starting.



\## 3. Data Flow \& Sequence Diagram (Search Execution)



1\.  \*\*User Input:\*\* User types "Alpha".

2\.  \*\*ViewModel:\*\* Relm4 fires `AppMsg::SearchQueryChanged("Alpha")`. Updates `state.query`.

3\.  \*\*IPC:\*\* ViewModel sends `WorkerMsg::ExecuteSearch("Alpha")` to the Tokio Worker via `async\_channel`.

4\.  \*\*Worker (Debounce):\*\* Worker cancels any previous search task. Starts 300ms sleep.

5\.  \*\*Worker (Processing):\*\* Sleep finishes. Worker requests `CancellationToken`. Iterates over `found\_documents`.

6\.  \*\*Worker (Extraction):\*\* Calls Extractor Module. If the file is an image, CPU blocks inside `spawn\_blocking` (OCR).

7\.  \*\*Worker (Yield):\*\* After each file, Worker checks the `CancellationToken`. If user typed again, it aborts immediately.

8\.  \*\*Worker -> UI:\*\* If a match is found, Worker sends `AppMsg::SearchResult(MatchedDocument)` back to Relm4.

9\.  \*\*ViewModel:\*\* Pushes `MatchedDocument` to `state.matched\_documents` and updates `ResultsFactory`.



\## 4. Error Handling \& Resilience Matrix



| Failure Scenario | System Response | UI Feedback |

| :--- | :--- | :--- |

| \*\*Encrypted/Corrupted PDF\*\* | Extractor catches `Err`. Logs to `tracing`. Skips file. | None (Silent skip to prevent UX spam). |

| \*\*File > 50MB detected\*\* | Scanner skips file before adding to `found\_documents`. | Toast notification: "Skipped X files (oversized)". |

| \*\*Missing Tesseract Data\*\* | Worker panics on init. Caught by Tokio. | Search spinner stops. Error Icon displays. Toast: "OCR Engine failed to load." |

| \*\*Sandbox Permission Denied\*\*| `xdg-desktop-portal` rejects access. | Toast: "Permission to access folder denied." |



\## 5. Deployment Architecture (Flatpak Sandboxing)

To satisfy offline reproducibility (NFR-2) on ALT Linux, the deployment architecture bypasses the host filesystem entirely.

\*   \*\*Runtime:\*\* `org.freedesktop.Platform` (v23.08+).

\*   \*\*C/C++ Build Isolation:\*\* The manifest explicitly downloads and compiles `Leptonica` and `Tesseract` via `autotools` modules.

\*   \*\*Asset Isolation:\*\* `eng.traineddata` is downloaded directly into the Flatpak virtual filesystem (`/app/share/tessdata`).

\*   \*\*IPC Permissions:\*\* `--talk-name=org.freedesktop.portal.Desktop` is injected into the manifest to allow `gio::AppInfo::launch\_default\_for\_uri` to open files via the host's Nautilus/File Manager.

