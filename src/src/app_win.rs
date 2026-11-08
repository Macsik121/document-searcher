//! Windows desktop application.
//!
//! Mirrors `app.rs` by running `Worker` on a dedicated Tokio runtime thread via
//! `RuntimeBridge` and mutating `AppState` through `WorkerCommand`/`WorkerEvent`.
//! State mutations and `WorkerCommand` dispatch are serialized through a single
//! `Controller` monitor to prevent race conditions.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::mpsc::{self as std_mpsc, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::runtime::Builder;
use tokio::sync::mpsc;

use crate::model::actions::{WorkerCommand, WorkerEvent};
use crate::model::extractor::normalize_platform_path;
use crate::model::state::{AppState, FileEntry, SearchMatch, SearchStatus};
use crate::model::worker::Worker;

const HTTP_WORKER_THREADS: usize = 4;
const HTTP_QUEUE_CAPACITY: usize = 32;
const MAX_REQUEST_BYTES: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Worker bridge: dedicated Tokio runtime on its own thread (mirrors app.rs).
// ---------------------------------------------------------------------------

struct RuntimeBridge {
    commands: mpsc::Sender<WorkerCommand>,
    runtime_thread: Option<thread::JoinHandle<()>>,
}

impl RuntimeBridge {
    fn start() -> (Self, mpsc::Receiver<WorkerEvent>) {
        let (command_tx, command_rx) = mpsc::channel(32);
        let (event_tx, event_rx) = mpsc::channel(64);

        let runtime_thread = thread::Builder::new()
            .name("document-searcher-worker".to_owned())
            .spawn(move || {
                let runtime = match Builder::new_multi_thread()
                    .enable_all()
                    .thread_name("document-searcher-worker-task")
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        tracing::error!(%error, "Could not create Tokio runtime");
                        return;
                    }
                };
                runtime.block_on(Worker::default().run(command_rx, event_tx));
            })
            .expect("failed to start worker runtime thread");

        (
            Self {
                commands: command_tx,
                runtime_thread: Some(runtime_thread),
            },
            event_rx,
        )
    }

    fn send(&self, command: WorkerCommand) {
        match self.commands.try_send(command) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(command)) => {
                let sender = self.commands.clone();
                thread::spawn(move || {
                    if let Err(error) = sender.blocking_send(command) {
                        tracing::warn!(%error, "Worker command queue is unavailable");
                    }
                });
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!("Worker command queue is closed");
            }
        }
    }
}

impl Drop for RuntimeBridge {
    fn drop(&mut self) {
        let commands = self.commands.clone();
        if let Some(runtime_thread) = self.runtime_thread.take() {
            thread::spawn(move || {
                if let Err(error) = commands.blocking_send(WorkerCommand::Shutdown) {
                    tracing::warn!(%error, "Worker command queue closed during shutdown");
                }
                if let Err(error) = runtime_thread.join() {
                    tracing::error!(?error, "Worker runtime thread panicked");
                }
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Settings persistence (%APPDATA%\DocumentSearcher\settings.conf).
// ---------------------------------------------------------------------------

fn settings_file_path() -> Option<PathBuf> {
    let mut base = if let Some(appdata) = std::env::var_os("APPDATA") {
        PathBuf::from(appdata)
    } else {
        let profile = std::env::var_os("USERPROFILE")?;
        PathBuf::from(profile).join(".config")
    };
    base.push("DocumentSearcher");
    base.push("settings.conf");
    Some(base)
}

fn load_persisted_scan_path() -> Option<PathBuf> {
    let config_path = settings_file_path()?;
    let content = fs::read_to_string(config_path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(normalize_platform_path(Path::new(trimmed)))
}

fn persist_scan_path(path: &Path) {
    let Some(config_path) = settings_file_path() else {
        return;
    };
    if let Some(parent) = config_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            tracing::warn!(%error, "Could not create settings directory");
            return;
        }
    }
    let clean = normalize_platform_path(path);
    if let Err(error) = fs::write(&config_path, clean.to_string_lossy().as_bytes()) {
        tracing::warn!(%error, "Could not persist scan-path to settings file");
    }
}

fn resolve_initial_state() -> AppState {
    let mut state = AppState::new();

    if let Ok(cli_dir) = std::env::var("DOCUMENT_SEARCHER_INITIAL_DIR") {
        let path = PathBuf::from(cli_dir);
        if let Ok(canonical) = path.canonicalize() {
            let clean = normalize_platform_path(&canonical);
            persist_scan_path(&clean);
            state.set_scan_path(clean);
            return state;
        }
    }

    if let Some(saved_path) = load_persisted_scan_path() {
        if saved_path.is_dir() {
            state.set_scan_path(saved_path);
            return state;
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        if cwd.is_dir() {
            let clean = normalize_platform_path(&cwd);
            state.set_scan_path(clean);
            return state;
        }
    }

    state
}

// ---------------------------------------------------------------------------
// Native Win32 Shell integration (Folder Picker & Default App File Opener).
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod win32 {
    use std::ffi::{OsStr, OsString};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};
    use std::ptr;

    type Hwnd = isize;
    type Lparam = isize;
    type Lpitemidlist = *mut core::ffi::c_void;
    type Uint = u32;

    const BIF_RETURNONLYFSDIRS: Uint = 0x0000_0001;
    const BIF_NEWDIALOGSTYLE: Uint = 0x0000_0040;
    const BFFM_INITIALIZED: Uint = 1;
    const BFFM_SETSELECTIONW: Uint = 0x0400 + 103;
    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const MAX_PATH: usize = 260;
    const SW_SHOWNORMAL: i32 = 1;

    #[repr(C)]
    struct BrowseInfoW {
        hwnd_owner: Hwnd,
        pidl_root: Lpitemidlist,
        psz_display_name: *mut u16,
        lpsz_title: *const u16,
        ul_flags: Uint,
        lpfn: Option<unsafe extern "system" fn(Hwnd, Uint, Lparam, Lparam) -> i32>,
        l_param: Lparam,
        i_image: i32,
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHBrowseForFolderW(lpbi: *mut BrowseInfoW) -> Lpitemidlist;
        fn SHGetPathFromIDListW(pidl: Lpitemidlist, psz_path: *mut u16) -> i32;
        fn ShellExecuteW(
            hwnd: Hwnd,
            lp_operation: *const u16,
            lp_file: *const u16,
            lp_parameters: *const u16,
            lp_directory: *const u16,
            n_show_cmd: i32,
        ) -> isize;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(pv_reserved: *mut core::ffi::c_void, dw_co_init: u32) -> i32;
        fn CoUninitialize();
        fn CoTaskMemFree(pv: *mut core::ffi::c_void);
    }

    #[link(name = "user32")]
    extern "system" {
        fn SendMessageW(hwnd: Hwnd, msg: Uint, w_param: usize, l_param: Lparam) -> isize;
        fn SetForegroundWindow(hwnd: Hwnd) -> i32;
    }

    fn to_wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    unsafe extern "system" fn browse_callback(
        hwnd: Hwnd,
        msg: Uint,
        _l_param: Lparam,
        lp_data: Lparam,
    ) -> i32 {
        if msg == BFFM_INITIALIZED {
            let _ = SetForegroundWindow(hwnd);
            if lp_data != 0 {
                let _ = SendMessageW(hwnd, BFFM_SETSELECTIONW, 1, lp_data);
            }
        }
        0
    }

    pub fn pick_folder(initial_dir: &Path) -> Option<PathBuf> {
        let title = to_wide(OsStr::new("Select a folder to search"));
        let initial = if initial_dir.is_dir() {
            Some(to_wide(initial_dir.as_os_str()))
        } else {
            None
        };
        let l_param = match &initial {
            Some(wide) => wide.as_ptr() as Lparam,
            None => 0,
        };
        let mut display_name = [0u16; MAX_PATH];

        unsafe {
            let hr = CoInitializeEx(ptr::null_mut(), COINIT_APARTMENTTHREADED);
            let should_uninit = hr >= 0;

            let mut bi = BrowseInfoW {
                hwnd_owner: 0,
                pidl_root: ptr::null_mut(),
                psz_display_name: display_name.as_mut_ptr(),
                lpsz_title: title.as_ptr(),
                ul_flags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
                lpfn: Some(browse_callback),
                l_param,
                i_image: 0,
            };

            let pidl = SHBrowseForFolderW(&mut bi);
            if pidl.is_null() {
                if should_uninit {
                    CoUninitialize();
                }
                return None;
            }

            let mut path_buf = [0u16; MAX_PATH];
            let ok = SHGetPathFromIDListW(pidl, path_buf.as_mut_ptr());
            CoTaskMemFree(pidl);
            if should_uninit {
                CoUninitialize();
            }

            if ok == 0 {
                return None;
            }

            let len = path_buf
                .iter()
                .position(|&ch| ch == 0)
                .unwrap_or(path_buf.len());
            Some(PathBuf::from(OsString::from_wide(&path_buf[..len])))
        }
    }

    pub fn open_with_default_app(path: &Path) -> anyhow::Result<()> {
        let operation = to_wide(OsStr::new("open"));
        let target = to_wide(path.as_os_str());
        let code = unsafe {
            ShellExecuteW(
                0,
                operation.as_ptr(),
                target.as_ptr(),
                ptr::null(),
                ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if code <= 32 {
            anyhow::bail!(
                "Could not open {} (ShellExecuteW code {code})",
                path.display()
            );
        }
        Ok(())
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            lp_file_name: *const u16,
            dw_desired_access: u32,
            dw_share_mode: u32,
            lp_security_attributes: *mut core::ffi::c_void,
            dw_creation_disposition: u32,
            dw_flags_and_attributes: u32,
            h_template_file: isize,
        ) -> isize;
        fn SetStdHandle(n_std_handle: u32, h_handle: isize) -> i32;
    }

    pub fn silence_stdout_spam() {
        const GENERIC_WRITE: u32 = 0x4000_0000;
        const FILE_SHARE_READ_WRITE: u32 = 0x0000_0001 | 0x0000_0002;
        const OPEN_EXISTING: u32 = 3;
        const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;
        const INVALID_HANDLE_VALUE: isize = -1;

        let nul = to_wide(OsStr::new("NUL"));
        unsafe {
            let handle = CreateFileW(
                nul.as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_READ_WRITE,
                ptr::null_mut(),
                OPEN_EXISTING,
                0,
                0,
            );
            if handle != INVALID_HANDLE_VALUE && handle != 0 {
                let _ = SetStdHandle(STD_OUTPUT_HANDLE, handle);
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod win32 {
    use std::fs::OpenOptions;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    #[cfg(unix)]
    extern "C" {
        fn dup2(oldfd: i32, newfd: i32) -> i32;
    }

    /// Redirects `stdout` to `/dev/null` so that the chatty `pdf-extract` crate
    /// does not flood the embedded browser console. `tracing` logs still reach
    /// `stderr`. Console `scan`/`search` commands finish before `run()` calls
    /// this, so their output is preserved.
    pub fn silence_stdout_spam() {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            if let Ok(dev_null) = OpenOptions::new().write(true).open("/dev/null") {
                unsafe {
                    let _ = dup2(dev_null.as_raw_fd(), 1);
                }
            }
        }
    }

    /// Tries each installed graphical folder dialog in turn (zenity → kdialog
    /// → yad) and returns the first user selection. Returns `None` on cancel or
    /// when no dialog helper is available (the UI's manual path field covers
    /// that case).
    pub fn pick_folder(initial_dir: &Path) -> Option<PathBuf> {
        let start_dir = if initial_dir.is_dir() {
            format!("{}/", initial_dir.display())
        } else {
            String::new()
        };

        // 1. zenity — standard on GNOME / MATE / Xfce / ALT Linux Workstation / Ubuntu.
        let mut zenity_cmd = Command::new("zenity");
        zenity_cmd.args([
            "--file-selection",
            "--directory",
            "--title=Select a folder to search",
        ]);
        if !start_dir.is_empty() {
            zenity_cmd.arg(format!("--filename={start_dir}"));
        }
        if let Ok(output) = zenity_cmd.stderr(Stdio::null()).output() {
            if output.status.success() {
                let selected = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if !selected.is_empty() {
                    return Some(PathBuf::from(selected));
                }
            }
            return None;
        }

        // 2. kdialog — standard on KDE Plasma / ALT Linux KWorkstation / Kubuntu.
        let mut kdialog_cmd = Command::new("kdialog");
        kdialog_cmd.arg("--getexistingdirectory");
        if initial_dir.is_dir() {
            kdialog_cmd.arg(initial_dir);
        } else {
            kdialog_cmd.arg(".");
        }
        kdialog_cmd.args(["--title", "Select a folder to search"]);
        if let Ok(output) = kdialog_cmd.stderr(Stdio::null()).output() {
            if output.status.success() {
                let selected = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if !selected.is_empty() {
                    return Some(PathBuf::from(selected));
                }
            }
            return None;
        }

        // 3. yad — lightweight GTK dialog.
        if let Ok(output) = Command::new("yad")
            .args(["--file", "--directory", "--title=Select a folder to search"])
            .stderr(Stdio::null())
            .output()
        {
            if output.status.success() {
                let selected = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if !selected.is_empty() {
                    return Some(PathBuf::from(selected));
                }
            }
        }

        None
    }

    /// Opens a document in the OS default application via `xdg-open`, falling
    /// back to `gio open` (Nautilus-managed desktops).
    pub fn open_with_default_app(path: &Path) -> anyhow::Result<()> {
        if Command::new("xdg-open")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            return Ok(());
        }

        if Command::new("gio")
            .args(["open"])
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            return Ok(());
        }

        anyhow::bail!("Could not open {} via xdg-open or gio open", path.display())
    }
}

fn open_file(path: &Path) -> anyhow::Result<()> {
    let clean = normalize_platform_path(path);
    if !clean.is_file() {
        anyhow::bail!("File does not exist: {}", clean.display());
    }
    win32::open_with_default_app(&clean)
}

// ---------------------------------------------------------------------------
// Application UI state holder & synchronized Controller monitor.
// ---------------------------------------------------------------------------

struct WinUiState {
    state: AppState,
    status_text: String,
    version: u64,
    last_query_seq: u64,
    folder_dialog_open: bool,
}

impl WinUiState {
    fn new(initial_state: AppState) -> Self {
        Self {
            state: initial_state,
            status_text: "Ready".to_owned(),
            version: 1,
            last_query_seq: 0,
            folder_dialog_open: false,
        }
    }

    fn bump(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    fn is_indexed_file(&self, candidate: &Path) -> bool {
        let normalized_candidate = normalize_platform_path(candidate);
        let candidate_str = normalized_candidate.to_string_lossy();
        self.state.found_files.iter().any(|entry| {
            let entry_norm = normalize_platform_path(&entry.absolute_path);
            entry_norm == normalized_candidate
                || entry_norm
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&candidate_str)
        })
    }

    fn set_status(&mut self, status: SearchStatus, detail: Option<&str>) {
        self.state.status = status;
        let text = match status {
            SearchStatus::Idle => detail.unwrap_or("Ready"),
            SearchStatus::Scanning => detail.unwrap_or("Scanning…"),
            SearchStatus::Searching => detail.unwrap_or("Searching…"),
            SearchStatus::Success => detail.unwrap_or("Search complete"),
            SearchStatus::Error => detail.unwrap_or("An error occurred"),
        };
        self.status_text = text.to_owned();
        self.bump();
    }

    fn set_error(&mut self, error: String) {
        self.state.error = Some(error.clone());
        self.set_status(SearchStatus::Error, Some(&error));
    }

    fn scan_started(&mut self) {
        self.state.status = SearchStatus::Scanning;
        self.state.error = None;
        self.state.found_files.clear();
        self.state.search_results.clear();
        self.state.results_offset = 0;
        self.set_status(SearchStatus::Scanning, Some("Scanning…"));
    }

    fn scan_completed(&mut self, files: Vec<FileEntry>, oversized: usize, unreadable: usize) {
        self.state.skipped_oversized_files = oversized;
        self.state.skipped_unreadable_entries = unreadable;
        self.state.found_files = files
            .into_iter()
            .map(|entry| FileEntry {
                relative_path: entry.relative_path,
                absolute_path: normalize_platform_path(&entry.absolute_path),
            })
            .collect();
        self.state.status = SearchStatus::Idle;
        let summary = match (oversized, unreadable) {
            (0, 0) => format!("Ready — {} files indexed", self.state.found_files.len()),
            _ => format!(
                "Ready — {} files indexed; skipped {} oversized and {} unreadable entries",
                self.state.found_files.len(),
                oversized,
                unreadable
            ),
        };
        self.set_status(SearchStatus::Idle, Some(&summary));
    }

    fn accept_query_edit(&mut self, query: String, seq: Option<u64>) -> bool {
        if let Some(seq_num) = seq {
            if seq_num < self.last_query_seq {
                return false;
            }
            self.last_query_seq = seq_num;
        }

        self.state.search_query = query.clone();
        self.state.search_results.clear();
        self.state.results_offset = 0;
        self.state.error = None;

        if self.state.status == SearchStatus::Scanning {
            self.bump();
            return true;
        }

        if query.trim().is_empty() {
            let count = self.state.found_files.len();
            self.set_status(
                SearchStatus::Idle,
                Some(&format!("Ready — {count} files indexed")),
            );
        } else {
            self.set_status(SearchStatus::Searching, Some("Waiting for input…"));
        }
        true
    }

    fn search_started(&mut self, query: &str) {
        if self.state.status == SearchStatus::Scanning {
            return;
        }
        if self.state.search_query != query || query.trim().is_empty() {
            return;
        }
        self.state.search_results.clear();
        self.state.results_offset = 0;
        self.state.error = None;
        self.set_status(SearchStatus::Searching, Some("Searching…"));
    }

    fn search_progress(&mut self, completed: usize, total: usize) {
        if self.state.status == SearchStatus::Scanning || self.state.search_query.trim().is_empty()
        {
            return;
        }
        let progress = format!("Searching… {completed}/{total}");
        self.set_status(SearchStatus::Searching, Some(&progress));
    }

    fn search_completed(&mut self, query: String, results: Vec<SearchMatch>) {
        if self.state.search_query != query {
            return;
        }
        self.state.search_results = results
            .into_iter()
            .map(|item| SearchMatch {
                file_relative_path: item.file_relative_path,
                file_absolute_path: normalize_platform_path(&item.file_absolute_path),
                contexts: item.contexts,
                occurrences: item.occurrences,
            })
            .collect();
        self.state.results_offset = 0;
        self.state.error = None;

        if query.trim().is_empty() {
            if self.state.status == SearchStatus::Scanning {
                self.bump();
                return;
            }
            let count = self.state.found_files.len();
            self.set_status(
                SearchStatus::Idle,
                Some(&format!("Ready — {count} files indexed")),
            );
        } else {
            let count = self.state.search_results.len();
            self.set_status(
                SearchStatus::Success,
                Some(&format!("Search complete — {count} matching files")),
            );
        }
    }

    fn load_more(&mut self) {
        self.state.load_more();
        self.bump();
    }
}

/// Synchronizes UI state mutations and `WorkerCommand` dispatch under a single
/// lock so commands are always delivered to `Worker` in the exact order of
/// state transitions.
struct Controller {
    ui: WinUiState,
    bridge: RuntimeBridge,
}

impl Controller {
    fn submit_query(&mut self, query: String, seq: Option<u64>) {
        if self.ui.accept_query_edit(query.clone(), seq) {
            self.bridge.send(WorkerCommand::QueryChanged { query });
        }
    }

    fn start_folder_scan(&mut self, folder: PathBuf) {
        let clean_folder = normalize_platform_path(&folder);
        self.ui.folder_dialog_open = false;
        self.ui.state.search_query.clear();
        self.ui.state.set_scan_path(clean_folder.clone());
        self.ui.scan_started();
        self.bridge.send(WorkerCommand::Scan { root: clean_folder });
    }
}

// ---------------------------------------------------------------------------
// JSON serialization & session token generation.
// ---------------------------------------------------------------------------

fn generate_session_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let stack_addr = &nanos as *const u128 as usize;
    format!("{nanos:032x}{pid:08x}{stack_addr:016x}")
}

fn json_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

fn serialize_state_json(ui: &WinUiState) -> String {
    let status_name = match ui.state.status {
        SearchStatus::Idle => "idle",
        SearchStatus::Scanning => "scanning",
        SearchStatus::Searching => "searching",
        SearchStatus::Success => "success",
        SearchStatus::Error => "error",
    };

    let current_page = ui.state.get_visible_results();
    let total_results = ui.state.search_results.len();
    let visible_end = if current_page.is_empty() {
        0
    } else {
        ui.state
            .results_offset
            .saturating_add(current_page.len())
            .min(total_results)
    };
    let visible_results = &ui.state.search_results[..visible_end];

    let mut json = String::with_capacity(4096);
    json.push('{');
    json.push_str(&format!("\"version\":{},", ui.version));
    json.push_str(&format!(
        "\"currentPath\":\"{}\",",
        json_escape(&normalize_platform_path(&ui.state.current_path).to_string_lossy())
    ));
    json.push_str(&format!(
        "\"searchQuery\":\"{}\",",
        json_escape(&ui.state.search_query)
    ));
    json.push_str(&format!("\"fileCount\":{},", ui.state.found_files.len()));
    json.push_str(&format!(
        "\"skippedOversized\":{},",
        ui.state.skipped_oversized_files
    ));
    json.push_str(&format!(
        "\"skippedUnreadable\":{},",
        ui.state.skipped_unreadable_entries
    ));
    json.push_str(&format!("\"status\":\"{status_name}\","));
    json.push_str(&format!(
        "\"statusText\":\"{}\",",
        json_escape(&ui.status_text)
    ));
    json.push_str(&format!("\"totalResults\":{total_results},"));
    json.push_str(&format!("\"hasMore\":{},", ui.state.has_more_results()));
    json.push_str(&format!("\"dialogOpen\":{},", ui.folder_dialog_open));

    json.push_str("\"files\":[");
    for (idx, file) in ui.state.found_files.iter().enumerate() {
        if idx > 0 {
            json.push(',');
        }
        json.push_str(&format!(
            "{{\"rel\":\"{}\",\"abs\":\"{}\"}}",
            json_escape(&file.relative_path),
            json_escape(&file.absolute_path.to_string_lossy())
        ));
    }
    json.push_str("],\"results\":[");

    for (idx, result) in visible_results.iter().enumerate() {
        if idx > 0 {
            json.push(',');
        }
        json.push_str(&format!(
            "{{\"num\":{},\"rel\":\"{}\",\"abs\":\"{}\",\"occurrences\":[",
            idx + 1,
            json_escape(&result.file_relative_path),
            json_escape(&result.file_absolute_path.to_string_lossy())
        ));
        for (occ_idx, occ) in result.occurrences.iter().enumerate() {
            if occ_idx > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                "{{\"before\":\"{}\",\"matched\":\"{}\",\"after\":\"{}\"}}",
                json_escape(&occ.before),
                json_escape(&occ.matched),
                json_escape(&occ.after)
            ));
        }
        json.push_str("]}");
    }
    json.push_str("]}");
    json
}

// ---------------------------------------------------------------------------
// Standalone desktop window launcher (Edge/Chrome --app mode + fallback).
// ---------------------------------------------------------------------------

fn find_app_browser() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut candidates = Vec::new();
        for env_key in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
            if let Ok(base) = std::env::var(env_key) {
                let base_path = PathBuf::from(base);
                let mut edge = base_path.clone();
                edge.push("Microsoft");
                edge.push("Edge");
                edge.push("Application");
                edge.push("msedge.exe");
                candidates.push(edge);

                let mut chrome = base_path.clone();
                chrome.push("Google");
                chrome.push("Chrome");
                chrome.push("Application");
                chrome.push("chrome.exe");
                candidates.push(chrome);

                let mut yandex = base_path;
                yandex.push("Yandex");
                yandex.push("YandexBrowser");
                yandex.push("Application");
                yandex.push("browser.exe");
                candidates.push(yandex);
            }
        }
        candidates.into_iter().find(|path| path.is_file())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let browsers = [
            "chromium",
            "chromium-browser",
            "google-chrome",
            "google-chrome-stable",
            "yandex-browser",
            "brave-browser",
            "microsoft-edge",
        ];
        for name in browsers {
            for prefix in ["/usr/bin", "/usr/local/bin", "/snap/bin"] {
                let candidate = Path::new(prefix).join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
    }
}

fn spawn_desktop_window(url: &str, profile_dir: &Path) -> Option<Child> {
    if let Some(browser) = find_app_browser() {
        let user_data_arg = format!("--user-data-dir={}", profile_dir.display());
        let app_arg = format!("--app={url}");
        let mut command = Command::new(browser);
        command.args([
            &app_arg,
            "--window-size=1200,800",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            &user_data_arg,
        ]);

        #[cfg(not(target_os = "windows"))]
        {
            command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
        }

        if let Ok(child) = command.spawn() {
            return Some(child);
        }
    }

    // Firefox has no `--app` mode, so open the UI URL in a new window instead.
    #[cfg(not(target_os = "windows"))]
    {
        let mut firefox = Command::new("firefox");
        firefox
            .args(["--new-window", url])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if let Ok(child) = firefox.spawn() {
            return Some(child);
        }
    }

    let _ = win32::open_with_default_app(Path::new(url));
    None
}

// ---------------------------------------------------------------------------
// Fixed-size HTTP worker pool & strict HTTP/UTF-8 request handling.
// ---------------------------------------------------------------------------

struct HttpWorkerPool {
    sender: Option<SyncSender<TcpStream>>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl HttpWorkerPool {
    fn start(
        controller: Arc<Mutex<Controller>>,
        shutdown_requested_at: Arc<Mutex<Option<Instant>>>,
        session_token: Arc<String>,
    ) -> Self {
        let (tx, rx) = std_mpsc::sync_channel::<TcpStream>(HTTP_QUEUE_CAPACITY);
        let shared_rx = Arc::new(Mutex::new(rx));
        let mut workers = Vec::with_capacity(HTTP_WORKER_THREADS);

        for index in 0..HTTP_WORKER_THREADS {
            let rx_clone = Arc::clone(&shared_rx);
            let ctrl_clone = Arc::clone(&controller);
            let shutdown_clone = Arc::clone(&shutdown_requested_at);
            let token_clone = Arc::clone(&session_token);

            let handle = thread::Builder::new()
                .name(format!("document-searcher-http-{index}"))
                .spawn(move || loop {
                    let stream = {
                        let Ok(guard) = rx_clone.lock() else {
                            break;
                        };
                        match guard.recv() {
                            Ok(stream) => stream,
                            Err(_) => break,
                        }
                    };
                    handle_client(stream, &ctrl_clone, &shutdown_clone, &token_clone);
                })
                .expect("failed to spawn HTTP worker thread");
            workers.push(handle);
        }

        Self {
            sender: Some(tx),
            workers,
        }
    }

    fn dispatch(&self, stream: TcpStream) {
        if let Some(sender) = &self.sender {
            let _ = sender.try_send(stream);
        }
    }
}

impl Drop for HttpWorkerPool {
    fn drop(&mut self) {
        self.sender.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

struct HttpRequest {
    method: String,
    path: String,
    token_header: Option<String>,
    query_seq: Option<u64>,
    body: String,
}

fn read_http_request(stream: &mut TcpStream) -> Option<HttpRequest> {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let mut buffer = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];

    let header_end = loop {
        let bytes_read = stream.read(&mut chunk).ok()?;
        if bytes_read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..bytes_read]);
        if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        if buffer.len() > MAX_REQUEST_BYTES {
            return None;
        }
    };

    let header_str = std::str::from_utf8(&buffer[..header_end]).ok()?;
    let mut lines = header_str.lines();
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();

    let mut content_length = 0usize;
    let mut token_header = None;
    let mut query_seq = None;

    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let trimmed_name = name.trim();
            if trimmed_name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().ok()?;
            } else if trimmed_name.eq_ignore_ascii_case("x-document-searcher-token") {
                token_header = Some(value.trim().to_owned());
            } else if trimmed_name.eq_ignore_ascii_case("x-query-seq") {
                query_seq = value.trim().parse().ok();
            }
        }
    }

    if content_length > MAX_REQUEST_BYTES {
        return None;
    }

    let target_len = header_end.saturating_add(content_length);
    while buffer.len() < target_len {
        let bytes_read = stream.read(&mut chunk).ok()?;
        if bytes_read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..bytes_read]);
    }

    let body = std::str::from_utf8(&buffer[header_end..target_len])
        .ok()?
        .to_owned();

    Some(HttpRequest {
        method,
        path,
        token_header,
        query_seq,
        body,
    })
}

fn write_http_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn handle_client(
    mut stream: TcpStream,
    controller: &Arc<Mutex<Controller>>,
    shutdown_requested_at: &Arc<Mutex<Option<Instant>>>,
    session_token: &str,
) {
    let Some(req) = read_http_request(&mut stream) else {
        return;
    };

    if req.method == "POST" {
        let header_matches = req.token_header.as_deref() == Some(session_token);
        let beacon_matches = req.path == "/api/shutdown" && req.body.trim() == session_token;
        if !header_matches && !beacon_matches {
            write_http_response(
                &mut stream,
                "403 Forbidden",
                "text/plain; charset=utf-8",
                b"Forbidden",
            );
            return;
        }
    }

    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => {
            if let Ok(mut shutdown) = shutdown_requested_at.lock() {
                *shutdown = None;
            }
            let html = INDEX_HTML.replace("__SESSION_TOKEN__", session_token);
            write_http_response(
                &mut stream,
                "200 OK",
                "text/html; charset=utf-8",
                html.as_bytes(),
            );
        }
        ("GET", "/api/state") => {
            if let Ok(mut shutdown) = shutdown_requested_at.lock() {
                *shutdown = None;
            }
            let payload = match controller.lock() {
                Ok(guard) => serialize_state_json(&guard.ui),
                Err(_) => "{\"error\":\"state lock poisoned\"}".to_owned(),
            };
            write_http_response(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                payload.as_bytes(),
            );
        }
        ("POST", "/api/query") => {
            if let Ok(mut guard) = controller.lock() {
                guard.submit_query(req.body, req.query_seq);
            }
            write_http_response(&mut stream, "200 OK", "text/plain; charset=utf-8", b"ok");
        }
        ("POST", "/api/load-more") => {
            if let Ok(mut guard) = controller.lock() {
                guard.ui.load_more();
            }
            write_http_response(&mut stream, "200 OK", "text/plain; charset=utf-8", b"ok");
        }
        ("POST", "/api/open-file") => {
            let target = PathBuf::from(&req.body);
            let allowed = match controller.lock() {
                Ok(guard) => guard.ui.is_indexed_file(&target),
                Err(_) => false,
            };
            if !allowed {
                write_http_response(
                    &mut stream,
                    "403 Forbidden",
                    "text/plain; charset=utf-8",
                    b"File is not in the current scan index",
                );
                return;
            }
            if let Err(error) = open_file(&target) {
                tracing::warn!(%error, path = %target.display(), "Could not open file");
                if let Ok(mut guard) = controller.lock() {
                    guard.ui.set_error(error.to_string());
                }
            }
            write_http_response(&mut stream, "200 OK", "text/plain; charset=utf-8", b"ok");
        }
        ("POST", "/api/choose-folder") => {
            let initial_dir = match controller.lock() {
                Ok(mut guard) => {
                    if guard.ui.folder_dialog_open {
                        write_http_response(
                            &mut stream,
                            "200 OK",
                            "text/plain; charset=utf-8",
                            b"busy",
                        );
                        return;
                    }
                    guard.ui.folder_dialog_open = true;
                    guard.ui.bump();
                    normalize_platform_path(&guard.ui.state.current_path)
                }
                Err(_) => return,
            };

            let ctrl_clone = Arc::clone(controller);
            thread::spawn(move || {
                let selected = win32::pick_folder(&initial_dir);
                match selected {
                    Some(folder) => {
                        persist_scan_path(&folder);
                        if let Ok(mut guard) = ctrl_clone.lock() {
                            guard.start_folder_scan(folder);
                        }
                    }
                    None => {
                        if let Ok(mut guard) = ctrl_clone.lock() {
                            guard.ui.folder_dialog_open = false;
                            guard.ui.bump();
                        }
                    }
                }
            });

            write_http_response(&mut stream, "200 OK", "text/plain; charset=utf-8", b"ok");
        }
        ("POST", "/api/set-folder") => {
            let candidate = PathBuf::from(req.body.trim());
            if let Ok(canonical) = candidate.canonicalize() {
                let clean = normalize_platform_path(&canonical);
                if clean.is_dir() {
                    persist_scan_path(&clean);
                    if let Ok(mut guard) = controller.lock() {
                        guard.start_folder_scan(clean);
                    }
                    write_http_response(&mut stream, "200 OK", "text/plain; charset=utf-8", b"ok");
                    return;
                }
            }
            if let Ok(mut guard) = controller.lock() {
                guard.ui.set_error(format!(
                    "Selected path is not a readable directory: {}",
                    candidate.display()
                ));
            }
            write_http_response(
                &mut stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                b"invalid directory",
            );
        }
        ("POST", "/api/shutdown") => {
            if let Ok(mut shutdown) = shutdown_requested_at.lock() {
                *shutdown = Some(Instant::now());
            }
            write_http_response(&mut stream, "200 OK", "text/plain; charset=utf-8", b"bye");
        }
        _ => {
            write_http_response(
                &mut stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                b"Not Found",
            );
        }
    }
}

fn should_terminate_loop(
    controller: &Mutex<Controller>,
    shutdown_requested_at: &Mutex<Option<Instant>>,
) -> bool {
    let dialog_open = match controller.lock() {
        Ok(guard) => guard.ui.folder_dialog_open,
        Err(_) => false,
    };
    if dialog_open {
        return false;
    }

    let Ok(shutdown_guard) = shutdown_requested_at.lock() else {
        return false;
    };
    match *shutdown_guard {
        Some(instant) => instant.elapsed() >= Duration::from_millis(1200),
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Entry point.
// ---------------------------------------------------------------------------

pub fn run() {
    win32::silence_stdout_spam();

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let (bridge, mut events) = RuntimeBridge::start();
    let initial_state = resolve_initial_state();
    let initial_path = initial_state.current_path.clone();

    let mut initial_controller = Controller {
        ui: WinUiState::new(initial_state),
        bridge,
    };
    initial_controller.ui.scan_started();
    initial_controller
        .bridge
        .send(WorkerCommand::Scan { root: initial_path });

    let controller = Arc::new(Mutex::new(initial_controller));

    // Forward WorkerEvent messages into Controller.ui.
    {
        let ctrl_events = Arc::clone(&controller);
        thread::Builder::new()
            .name("document-searcher-ui-events".to_owned())
            .spawn(move || {
                while let Some(event) = events.blocking_recv() {
                    let Ok(mut guard) = ctrl_events.lock() else {
                        break;
                    };
                    match event {
                        WorkerEvent::ScanStarted => guard.ui.scan_started(),
                        WorkerEvent::ScanProgress { completed } => guard.ui.set_status(
                            SearchStatus::Scanning,
                            Some(&format!("Scanning… {completed} files")),
                        ),
                        WorkerEvent::ScanCompleted {
                            files,
                            oversized_files,
                            unreadable_entries,
                        } => guard
                            .ui
                            .scan_completed(files, oversized_files, unreadable_entries),
                        WorkerEvent::ScanError(error) => guard.ui.set_error(error),
                        WorkerEvent::SearchStarted { query } => guard.ui.search_started(&query),
                        WorkerEvent::SearchProgress { completed, total } => {
                            guard.ui.search_progress(completed, total)
                        }
                        WorkerEvent::SearchCompleted { query, results } => {
                            guard.ui.search_completed(query, results)
                        }
                        WorkerEvent::SearchCancelled { query } => {
                            if guard.ui.state.search_query == query {
                                guard
                                    .ui
                                    .set_status(SearchStatus::Idle, Some("Search cancelled"));
                            }
                        }
                        WorkerEvent::SearchError { query, error } => {
                            if guard.ui.state.search_query == query {
                                guard.ui.set_error(error);
                            }
                        }
                    }
                }
            })
            .expect("failed to start UI event forwarding thread");
    }

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("failed to bind local UI server on 127.0.0.1");
    let port = listener
        .local_addr()
        .expect("failed to read local UI server address")
        .port();
    let _ = listener.set_nonblocking(true);

    let session_token = Arc::new(generate_session_token());
    let shutdown_requested_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let pool = HttpWorkerPool::start(
        Arc::clone(&controller),
        Arc::clone(&shutdown_requested_at),
        Arc::clone(&session_token),
    );

    let url = format!("http://127.0.0.1:{port}");
    tracing::info!(%url, "Document Searcher Windows UI started");

    let profile_dir = std::env::temp_dir().join(format!(
        "document-searcher-win-ui-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    let _ = fs::create_dir_all(&profile_dir);

    let window_child = spawn_desktop_window(&url, &profile_dir);

    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                pool.dispatch(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => {
                tracing::warn!(%error, "UI listener accept error");
                thread::sleep(Duration::from_millis(50));
            }
        }

        if should_terminate_loop(&controller, &shutdown_requested_at) {
            break;
        }
    }

    drop(pool);
    if let Some(mut child) = window_child {
        let _ = child.kill();
    }
    let _ = fs::remove_dir_all(&profile_dir);
}

// ---------------------------------------------------------------------------
// Offline embedded UI (uses r##"..."## so "#..." inside JS/CSS never closes it).
// ---------------------------------------------------------------------------

const INDEX_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Document Searcher</title>
<style>
  :root {
    --bg-main: #ffffff;
    --bg-sidebar: #f8fafc;
    --border-color: #e2e8f0;
    --text-primary: #1e293b;
    --text-secondary: #64748b;
    --text-muted: #94a3b8;
    --accent: #2563eb;
    --accent-hover: #1d4ed8;
    --accent-soft: #eff6ff;
    --success: #16a34a;
    --error: #dc2626;
    --mark-bg: #fef08a;
  }
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body {
    font-family: "Segoe UI", -apple-system, BlinkMacSystemFont, sans-serif;
    background: var(--bg-main);
    color: var(--text-primary);
    height: 100vh;
    width: 100vw;
    display: flex;
    overflow: hidden;
    user-select: none;
  }
  aside {
    width: 42%;
    min-width: 320px;
    max-width: 560px;
    background: var(--bg-sidebar);
    border-right: 2px solid var(--border-color);
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .sidebar-header {
    padding: 20px 20px 14px 20px;
    border-bottom: 1px solid var(--border-color);
  }
  .folder-badge {
    font-family: Consolas, "Courier New", monospace;
    font-size: 13px;
    color: #334155;
    background: #e2e8f0;
    padding: 6px 10px;
    border-radius: 6px;
    display: block;
    word-break: break-all;
    margin-bottom: 8px;
    user-select: text;
  }
  .file-count {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
  }
  .section-caption {
    padding: 12px 20px 6px 20px;
    font-size: 11px;
    font-weight: 700;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .files-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 4px 12px 12px 12px;
  }
  .file-row {
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    padding: 8px 10px;
    border-radius: 6px;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 10px;
    color: #334155;
    font-size: 13.5px;
    font-weight: 500;
    transition: background 0.12s, color 0.12s;
  }
  .file-row:hover {
    background: #e2e8f0;
    color: #0f172a;
  }
  .file-row-icon {
    color: var(--accent);
    flex-shrink: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .file-row-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sidebar-footer {
    padding: 12px 20px;
    border-top: 1px solid var(--border-color);
    background: var(--bg-sidebar);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .path-input-row {
    display: flex;
    gap: 6px;
  }
  .path-input {
    flex: 1;
    border: 1px solid #cbd5e1;
    border-radius: 6px;
    padding: 7px 10px;
    font-family: Consolas, "Courier New", monospace;
    font-size: 12.5px;
    color: var(--text-primary);
    outline: none;
  }
  .path-input:focus {
    border-color: var(--accent);
  }
  .btn-secondary {
    background: #e2e8f0;
    color: #1e293b;
    border: none;
    border-radius: 6px;
    padding: 7px 12px;
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
  }
  .btn-secondary:hover {
    background: #cbd5e1;
  }
  .btn-primary {
    background: var(--accent);
    color: #ffffff;
    border: none;
    border-radius: 8px;
    padding: 10px 18px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    width: 100%;
    transition: background 0.15s;
  }
  .btn-primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .btn-primary:disabled {
    opacity: 0.6;
    cursor: default;
  }
  main {
    flex: 1;
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }
  .search-pane {
    padding: 28px 32px 20px 32px;
    border-bottom: 2px solid var(--border-color);
    background: #ffffff;
  }
  .search-title {
    font-size: 22px;
    font-weight: 700;
    color: #0f172a;
    margin-bottom: 6px;
  }
  .search-subtitle {
    font-size: 14px;
    color: var(--text-secondary);
    margin-bottom: 16px;
  }
  .search-input-wrap {
    display: flex;
    max-width: 720px;
  }
  .search-input {
    flex: 1;
    border: 1.5px solid #cbd5e1;
    border-radius: 8px;
    padding: 11px 14px;
    font-size: 15px;
    color: var(--text-primary);
    outline: none;
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .search-input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px rgba(37, 99, 235, 0.15);
  }
  .status-bar {
    margin-top: 14px;
    min-height: 24px;
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 13.5px;
  }
  .spinner {
    width: 18px;
    height: 18px;
    border: 2.5px solid #bfdbfe;
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
    display: none;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }
  .status-icon {
    font-size: 16px;
    font-weight: bold;
    display: none;
  }
  .status-icon-success { color: var(--success); }
  .status-icon-error { color: var(--error); }
  .status-idle { color: var(--text-secondary); }
  .status-scanning, .status-searching { color: var(--accent); font-weight: 600; }
  .status-success { color: var(--success); font-weight: 700; }
  .status-error { color: var(--error); font-weight: 600; }
  .results-pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: #ffffff;
  }
  .results-header {
    padding: 16px 32px 12px 32px;
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 17px;
    font-weight: 700;
    color: #1e293b;
  }
  .results-badge {
    background: #dbeafe;
    color: #1d4ed8;
    font-size: 12.5px;
    padding: 2px 10px;
    border-radius: 999px;
    font-weight: 600;
  }
  .results-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 4px 32px 24px 32px;
  }
  .result-card {
    border: 1px solid var(--border-color);
    border-radius: 10px;
    padding: 14px 18px;
    margin-bottom: 12px;
    background: #ffffff;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.03);
    transition: border-color 0.15s, background 0.15s;
  }
  .result-card:hover {
    border-color: #93c5fd;
    background: rgba(239, 246, 255, 0.35);
  }
  .result-link {
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
    text-align: left;
    font-size: 15.5px;
    font-weight: 700;
    color: var(--accent);
    text-decoration: underline;
    display: inline-block;
  }
  .result-link:hover {
    color: var(--accent-hover);
  }
  .result-abs-path {
    font-family: Consolas, "Courier New", monospace;
    font-size: 11.5px;
    color: var(--text-muted);
    margin-top: 2px;
    margin-bottom: 8px;
    user-select: text;
  }
  .context-box {
    font-family: Consolas, "Courier New", monospace;
    font-size: 13px;
    color: #1e293b;
    background: #f1f5f9;
    padding: 6px 10px;
    border-radius: 5px;
    margin-top: 6px;
    margin-left: 16px;
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
  }
  mark.search-match {
    background: var(--mark-bg);
    color: #0f172a;
    font-weight: 700;
    padding: 1px 4px;
    border-radius: 3px;
  }
  .load-more-wrap {
    padding: 12px 32px 18px 32px;
    text-align: center;
    display: none;
  }
  .btn-more {
    background: var(--accent);
    color: #ffffff;
    border: none;
    border-radius: 8px;
    padding: 9px 28px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
  }
  .btn-more:hover {
    background: var(--accent-hover);
  }
  .empty-hint {
    color: var(--text-muted);
    font-size: 14px;
    padding: 12px 8px;
  }
</style>
</head>
<body>
  <aside>
    <div class="sidebar-header">
      <div id="folderLabel" class="folder-badge">Folder: /home/documents</div>
      <div id="fileCountLabel" class="file-count">0 files are found</div>
    </div>
    <div class="section-caption">Files found in folder</div>
    <div id="filesList" class="files-scroll"></div>
    <div class="sidebar-footer">
      <div class="path-input-row">
        <input
          id="manualPathInput"
          class="path-input"
          type="text"
          placeholder="Paste folder path and press Go…"
          autocomplete="off"
          spellcheck="false"
        />
        <button id="goPathBtn" class="btn-secondary" type="button">Go</button>
      </div>
      <button id="changeFolderBtn" class="btn-primary" type="button">Change folder</button>
    </div>
  </aside>

  <main>
    <section class="search-pane">
      <h1 class="search-title">Write what you want to find!</h1>
      <p class="search-subtitle">
        The search runs through all supported files found in the selected folder
      </p>
      <div class="search-input-wrap">
        <input
          id="searchInput"
          class="search-input"
          type="text"
          placeholder="Enter the search string…"
          autocomplete="off"
          spellcheck="false"
        />
      </div>
      <div class="status-bar">
        <div id="spinner" class="spinner"></div>
        <span id="statusIcon" class="status-icon"></span>
        <span id="statusLabel" class="status-idle">Ready</span>
      </div>
    </section>

    <section class="results-pane">
      <div class="results-header">
        <span>Results</span>
        <span id="resultsCountBadge" class="results-badge">0</span>
      </div>
      <div id="resultsList" class="results-scroll"></div>
      <div id="loadMoreWrap" class="load-more-wrap">
        <button id="loadMoreBtn" class="btn-more" type="button">Show more</button>
      </div>
    </section>
  </main>

<script>
  const SESSION_TOKEN = "__SESSION_TOKEN__";
  let currentVersion = -1;
  let lastFilesSignature = "";
  let lastPath = "";
  let querySeq = 0;
  let activeQueryController = null;

  const folderLabel = document.getElementById("folderLabel");
  const fileCountLabel = document.getElementById("fileCountLabel");
  const filesList = document.getElementById("filesList");
  const changeFolderBtn = document.getElementById("changeFolderBtn");
  const manualPathInput = document.getElementById("manualPathInput");
  const goPathBtn = document.getElementById("goPathBtn");
  const searchInput = document.getElementById("searchInput");
  const spinner = document.getElementById("spinner");
  const statusIcon = document.getElementById("statusIcon");
  const statusLabel = document.getElementById("statusLabel");
  const resultsCountBadge = document.getElementById("resultsCountBadge");
  const resultsList = document.getElementById("resultsList");
  const loadMoreWrap = document.getElementById("loadMoreWrap");
  const loadMoreBtn = document.getElementById("loadMoreBtn");

  function postApi(endpoint, bodyText, extraHeaders, signal) {
    const headers = Object.assign({
      "Content-Type": "text/plain; charset=utf-8",
      "X-Document-Searcher-Token": SESSION_TOKEN
    }, extraHeaders || {});
    return fetch(endpoint, {
      method: "POST",
      headers: headers,
      body: bodyText || "",
      signal: signal
    }).catch(() => {});
  }

  function openFile(absPath) {
    postApi("/api/open-file", absPath);
  }

  changeFolderBtn.addEventListener("click", () => {
    postApi("/api/choose-folder", "");
  });

  function submitManualPath() {
    const raw = (manualPathInput.value || "").trim();
    if (!raw) {
      return;
    }
    postApi("/api/set-folder", raw);
    manualPathInput.value = "";
  }

  goPathBtn.addEventListener("click", submitManualPath);
  manualPathInput.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      submitManualPath();
    }
  });

  searchInput.addEventListener("input", () => {
    if (activeQueryController) {
      activeQueryController.abort();
    }
    activeQueryController = new AbortController();
    querySeq += 1;
    postApi(
      "/api/query",
      searchInput.value,
      { "X-Query-Seq": String(querySeq) },
      activeQueryController.signal
    );
  });

  loadMoreBtn.addEventListener("click", () => {
    postApi("/api/load-more", "");
  });

  window.addEventListener("pagehide", () => {
    navigator.sendBeacon("/api/shutdown", SESSION_TOKEN);
  });

  function renderFiles(files) {
    const signature = files.map(f => f.rel).join("\n");
    if (signature === lastFilesSignature && filesList.childElementCount > 0) {
      return;
    }
    lastFilesSignature = signature;
    filesList.textContent = "";

    if (files.length === 0) {
      const empty = document.createElement("div");
      empty.className = "empty-hint";
      empty.textContent = "No supported files in this folder.";
      filesList.appendChild(empty);
      return;
    }

    files.forEach((file, index) => {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "file-row";
      btn.title = file.abs;

      const icon = document.createElement("span");
      icon.className = "file-row-icon";
      icon.textContent = (index + 1) + ".";

      const label = document.createElement("span");
      label.className = "file-row-text";
      label.textContent = file.rel;

      btn.appendChild(icon);
      btn.appendChild(label);
      btn.addEventListener("click", () => openFile(file.abs));
      filesList.appendChild(btn);
    });
  }

  function renderResults(results) {
    resultsList.textContent = "";
    if (results.length === 0) {
      return;
    }

    results.forEach(item => {
      const card = document.createElement("div");
      card.className = "result-card";

      const link = document.createElement("button");
      link.type = "button";
      link.className = "result-link";
      link.textContent = item.num + ". " + item.rel;
      link.title = item.abs;
      link.addEventListener("click", () => openFile(item.abs));
      card.appendChild(link);

      const absLabel = document.createElement("div");
      absLabel.className = "result-abs-path";
      absLabel.textContent = item.abs;
      card.appendChild(absLabel);

      item.occurrences.forEach(occ => {
        const snippet = document.createElement("div");
        snippet.className = "context-box";

        if (occ.before) {
          snippet.appendChild(document.createTextNode(occ.before));
        }
        if (occ.matched) {
          const mark = document.createElement("mark");
          mark.className = "search-match";
          mark.textContent = occ.matched;
          snippet.appendChild(mark);
        }
        if (occ.after) {
          snippet.appendChild(document.createTextNode(occ.after));
        }
        card.appendChild(snippet);
      });

      resultsList.appendChild(card);
    });
  }

  function applyState(data) {
    changeFolderBtn.disabled = !!data.dialogOpen;
    if (data.version === currentVersion) {
      return;
    }
    currentVersion = data.version;

    if (data.currentPath !== lastPath) {
      lastPath = data.currentPath;
      if (!data.searchQuery) {
        searchInput.value = "";
      }
    }

    folderLabel.textContent = "Folder: " + data.currentPath;
    fileCountLabel.textContent = data.fileCount + " files are found";
    resultsCountBadge.textContent = String(data.totalResults);

    statusLabel.textContent = data.statusText || "";
    statusLabel.className = "status-" + data.status;

    if (data.status === "scanning" || data.status === "searching") {
      spinner.style.display = "inline-block";
      statusIcon.style.display = "none";
    } else if (data.status === "success") {
      spinner.style.display = "none";
      statusIcon.style.display = "inline";
      statusIcon.className = "status-icon status-icon-success";
      statusIcon.textContent = "✔";
    } else if (data.status === "error") {
      spinner.style.display = "none";
      statusIcon.style.display = "inline";
      statusIcon.className = "status-icon status-icon-error";
      statusIcon.textContent = "⚠";
    } else {
      spinner.style.display = "none";
      statusIcon.style.display = "none";
    }

    renderFiles(data.files || []);
    renderResults(data.results || []);
    loadMoreWrap.style.display = data.hasMore ? "block" : "none";
  }

  async function pollState() {
    try {
      const res = await fetch("/api/state", { cache: "no-store" });
      if (res.ok) {
        const data = await res.json();
        applyState(data);
      }
    } catch (_) {}
    setTimeout(pollState, 120);
  }

  pollState();
</script>
</body>
</html>
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_escape_handles_windows_paths_and_unicode() {
        let escaped = json_escape("C:\\Users\\Админ\\\"report\"\n\t\u{0007}");
        assert_eq!(
            escaped,
            "C:\\\\Users\\\\Админ\\\\\\\"report\\\"\\n\\t\\u0007"
        );
    }

    #[test]
    fn indexed_file_whitelist_rejects_unindexed_paths() {
        let mut ui = WinUiState::new(AppState::new());
        ui.scan_completed(
            vec![FileEntry {
                relative_path: "doc.txt".to_owned(),
                absolute_path: PathBuf::from(r"\\?\C:\docs\doc.txt"),
            }],
            0,
            0,
        );

        assert!(ui.is_indexed_file(Path::new(r"C:\docs\doc.txt")));
        assert!(ui.is_indexed_file(Path::new(r"c:\DOCS\doc.txt")));
        assert!(ui.is_indexed_file(Path::new(r"\\?\C:\docs\doc.txt")));
        assert!(!ui.is_indexed_file(Path::new(r"C:\Windows\System32\calc.exe")));
    }

    #[test]
    fn empty_query_does_not_overwrite_active_scanning_status() {
        let mut ui = WinUiState::new(AppState::new());
        ui.scan_started();
        assert_eq!(ui.state.status, SearchStatus::Scanning);

        assert!(ui.accept_query_edit(String::new(), Some(1)));
        assert_eq!(ui.state.status, SearchStatus::Scanning);

        ui.search_completed(String::new(), Vec::new());
        assert_eq!(ui.state.status, SearchStatus::Scanning);
    }

    #[test]
    fn stale_search_events_and_out_of_order_sequences_are_ignored() {
        let mut ui = WinUiState::new(AppState::new());
        assert!(ui.accept_query_edit("newest".to_owned(), Some(5)));
        assert!(!ui.accept_query_edit("older".to_owned(), Some(4)));
        assert_eq!(ui.state.search_query, "newest");

        ui.search_started("older");
        assert_eq!(ui.state.search_query, "newest");

        ui.scan_started();
        ui.search_progress(1, 10);
        assert_eq!(ui.state.status, SearchStatus::Scanning);
    }

    #[test]
    fn cumulative_pagination_preserves_previous_results_on_load_more() {
        let mut state = AppState::new();
        state.results_limit = 2;
        state.search_results = (1..=3)
            .map(|idx| SearchMatch {
                file_relative_path: format!("file_{idx}.txt"),
                file_absolute_path: PathBuf::from(format!(r"C:\docs\file_{idx}.txt")),
                contexts: Vec::new(),
                occurrences: Vec::new(),
            })
            .collect();

        let mut ui = WinUiState::new(state);
        let first_page = serialize_state_json(&ui);
        assert!(first_page.contains("\"num\":1,\"rel\":\"file_1.txt\""));
        assert!(first_page.contains("\"num\":2,\"rel\":\"file_2.txt\""));
        assert!(!first_page.contains("file_3.txt"));
        assert!(first_page.contains("\"hasMore\":true"));

        ui.load_more();
        let expanded = serialize_state_json(&ui);
        assert!(expanded.contains("\"num\":1,\"rel\":\"file_1.txt\""));
        assert!(expanded.contains("\"num\":2,\"rel\":\"file_2.txt\""));
        assert!(expanded.contains("\"num\":3,\"rel\":\"file_3.txt\""));
        assert!(expanded.contains("\"hasMore\":false"));
    }
}
