# User flow

```mermaid
flowchart TD
    Start([Start desktop application]) --> Settings{GSettings schema available?}
    Settings -- yes --> Restore[Read scan-path]
    Settings -- no --> Default[Use default /home/documents]
    Restore --> Scan[Send WorkerCommand::Scan]
    Default --> Scan
    Scan --> ScanStatus[Show scanning status]
    ScanStatus --> ScanResult{Scan result}
    ScanResult -- error --> ScanError[Show error status]
    ScanResult -- success --> Indexed[Render file list and counts]

    Indexed --> Choose{Change folder?}
    Choose -- yes --> Portal[Open ashpd FileChooser portal]
    Portal --> Cancel{Cancelled?}
    Cancel -- yes --> Indexed
    Cancel -- no --> Persist[Set GSettings scan-path]
    Persist --> Scan
    Choose -- no --> Query[User changes search entry]

    Query --> Empty{Query empty?}
    Empty -- yes --> Clear[Clear results and show Ready]
    Empty -- no --> Debounce[Wait 300 ms in Worker]
    Debounce --> Search[Extract and search indexed files]
    Search --> SearchResult{Search result}
    SearchResult -- error --> SearchError[Show error status]
    SearchResult -- success --> Results[Render up to 100 results]
    Results --> More{Show more?}
    More -- yes --> Page[Advance in-memory pagination]
    Page --> Results
    More -- no --> Open{Open a file?}
    Open -- yes --> GIO[gio::AppInfo::launch_default_for_uri]
    Open -- no --> Query
    GIO --> Query
```

The initial path is `/home/documents` unless a valid `scan-path` setting is
restored. Folder selection is mediated by the portal; cancellation keeps the
previous path and index. A successful selection persists the path and starts a
new scan. Search results, pagination offset, and the indexed file list are
memory-only.

Errors from scanning, extraction, the portal, or GIO are shown in the status
area. A new query cancels the prior debounced/active search; an empty query
clears results without starting extraction.
