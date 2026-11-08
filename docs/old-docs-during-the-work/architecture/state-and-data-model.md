# State and data model

## Runtime state

```mermaid
stateDiagram-v2
    [*] --> Idle
    Idle --> Scanning: WorkerEvent::ScanStarted
    Scanning --> Idle: ScanCompleted
    Scanning --> Error: ScanError
    Idle --> Searching: non-empty QueryChanged
    Searching --> Searching: SearchProgress
    Searching --> Success: SearchCompleted with matches/query
    Searching --> Idle: empty query or cancellation
    Searching --> Error: SearchError
    Success --> Searching: new non-empty query
    Success --> Idle: empty query
    Error --> Scanning: new scan
    Error --> Searching: new non-empty query
    Error --> Idle: empty query/cancel
```

`AppState` owns the current path, indexed `FileEntry` values, query, result
list, pagination offset/limit, `SearchStatus`, error text, and scan skip
counters. It is mutated by the GLib main-loop event handler. `RuntimeBridge`
and `Worker` own the asynchronous boundary; they do not directly mutate GTK
widgets.

## In-memory data

| Type | Fields and purpose |
| --- | --- |
| `FileEntry` | `relative_path` for display and `absolute_path` for filesystem operations |
| `SearchOccurrence` | `before`, exact `matched`, `after`, and combined `snippet` |
| `SearchMatch` | file paths, occurrence list, and compatibility `contexts` snippets |
| `SearchStatus` | `Idle`, `Scanning`, `Searching`, `Success`, or `Error` |
| `WorkerCommand` | `Scan { root }`, `QueryChanged { query }`, or `Shutdown` |
| `WorkerEvent` | scan lifecycle/progress/errors and search lifecycle/progress/results/cancellation/errors |

The scanner filters hidden entries, does not follow symlinks, enforces the 50
MiB file limit, and sorts relative paths. The search layer performs
case-insensitive Unicode-aware matching and limits each side of a context to
20 characters. The worker adds the 300 ms debounce, cancellation token, and
generation guards.

## Persistence boundary

There is no database, disk index, or persistent result entity, so no ERD is
needed. The file index and search results are rebuilt in memory. GSettings
persists only the selected `scan-path` string in the
`org.gtk.DocumentSearcher` schema. The embedded CSS GResource is build output
inside the desktop binary, not application state.

## Lifecycle

A scan replaces the worker's indexed file list only for the current scan
generation. A newer query cancels and aborts the prior search task. The UI
ignores stale completion events whose query no longer matches its current
search query.
