# Scan and search sequence

```mermaid
sequenceDiagram
    participant UI as GLib main loop / build_ui()
    participant B as RuntimeBridge
    participant W as Tokio Worker
    participant S as scanner.rs
    participant E as extractor.rs
    participant Q as search.rs

    UI->>B: WorkerCommand::Scan { root }
    B->>W: bounded mpsc command
    W->>W: cancel prior scan/search; increment scan generation
    W-->>UI: WorkerEvent::ScanStarted
    W->>S: scan_directory_with_report(root)
    S-->>W: files + skipped counters
    W->>W: generation guard before publishing
    W-->>UI: WorkerEvent::ScanCompleted

    UI->>B: WorkerCommand::QueryChanged { query }
    B->>W: bounded mpsc command
    W->>W: cancel previous search; increment generation
    W->>W: wait 300 ms debounce or cancellation
    W-->>UI: WorkerEvent::SearchStarted
    loop each indexed file
        W->>W: acquire extraction semaphore
        W->>E: spawn_blocking extract_text(file)
        E-->>Q: extracted text
        Q-->>W: SearchMatch occurrences + context
        W->>W: cancellation and generation guards
        W-->>UI: WorkerEvent::SearchProgress
    end
    W-->>UI: WorkerEvent::SearchCompleted

    UI->>B: newer QueryChanged
    B->>W: replacement command
    W->>W: CancellationToken.cancel + abort old task
    W-->>UI: only current generation may publish completion
```

`SEARCH_DEBOUNCE` is 300 ms. Search extraction is blocking work moved away
from the GLib loop with `spawn_blocking`; a semaphore currently limits active
extraction. Cancellation is checked during debounce, before and after each
file, and while sending events. Generation counters prevent stale scan or
search results from reaching the UI after a newer command.

The scan task updates the worker's in-memory file list only when its generation
is still current. The UI receives events through the GLib main context and
updates `AppState` and widgets there.
