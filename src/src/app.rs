//! GTK4/Libadwaita desktop application.
//!
//! Layout is built procedurally. The Tokio worker runs on its own runtime
//! thread and communicates with the GLib main loop strictly through
//! `WorkerCommand`/`WorkerEvent` channels.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::thread;

use gtk::prelude::*;
use gtk::{gio, glib};
use libadwaita as adw;
use tokio::runtime::Builder;
use tokio::sync::mpsc;

use crate::model::actions::{WorkerCommand, WorkerEvent};
use crate::model::state::{AppState, FileEntry, SearchMatch, SearchStatus};
use crate::model::worker::Worker;

pub const APPLICATION_ID: &str = "org.gtk.DocumentSearcher";

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
                glib::MainContext::default().spawn_local(async move {
                    if let Err(error) = sender.send(command).await {
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

fn load_settings() -> Option<gio::Settings> {
    let default_source = gio::SettingsSchemaSource::default()?;
    if let Some(schema) = default_source.lookup(APPLICATION_ID, true) {
        return Some(gio::Settings::new_full(
            &schema,
            None::<&gio::SettingsBackend>,
            None,
        ));
    }

    if let Some(schema_dir) = option_env!("DOCUMENT_SEARCHER_SCHEMA_DIR") {
        if Path::new(schema_dir).is_dir() {
            let custom_source =
                gio::SettingsSchemaSource::from_directory(schema_dir, Some(&default_source), false)
                    .ok()?;
            let schema = custom_source.lookup(APPLICATION_ID, true)?;
            return Some(gio::Settings::new_full(
                &schema,
                None::<&gio::SettingsBackend>,
                None,
            ));
        }
    }

    None
}

fn load_css() {
    let Some(display) = gtk::gdk::Display::default() else {
        tracing::warn!("Could not load application CSS without a display");
        return;
    };

    let provider = gtk::CssProvider::new();
    let css = gio::resources_lookup_data(
        "/org/gtk/DocumentSearcher/styles.css",
        gio::ResourceLookupFlags::NONE,
    );
    match css {
        Ok(css) => match std::str::from_utf8(css.as_ref()) {
            Ok(css) => provider.load_from_data(css),
            Err(error) => tracing::warn!(%error, "Application CSS is not valid UTF-8"),
        },
        Err(error) => tracing::warn!(%error, "Could not load application CSS resource"),
    }
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

struct Ui {
    state: AppState,
    settings: Option<gio::Settings>,
    files_list: gtk::ListBox,
    results_list: gtk::ListBox,
    folder_label: gtk::Label,
    file_count_label: gtk::Label,
    status_label: gtk::Label,
    status_icon: gtk::Image,
    spinner: gtk::Spinner,
    load_more_button: gtk::Button,
}

impl Ui {
    fn set_status(&self, status: SearchStatus, detail: Option<&str>) {
        let (label, spinning, css_class, icon_name) = match status {
            SearchStatus::Idle => (detail.unwrap_or("Ready"), false, "search-status-idle", None),
            SearchStatus::Scanning => (
                detail.unwrap_or("Scanning…"),
                true,
                "search-status-searching",
                None,
            ),
            SearchStatus::Searching => (
                detail.unwrap_or("Searching…"),
                true,
                "search-status-searching",
                None,
            ),
            SearchStatus::Success => (
                detail.unwrap_or("Search complete"),
                false,
                "search-status-success",
                Some("emblem-ok-symbolic"),
            ),
            SearchStatus::Error => (
                detail.unwrap_or("An error occurred"),
                false,
                "search-status-error",
                Some("dialog-error-symbolic"),
            ),
        };

        self.status_label.set_label(label);
        self.status_label.set_visible(!label.is_empty());
        for class in [
            "search-status-idle",
            "search-status-searching",
            "search-status-success",
            "search-status-error",
        ] {
            self.status_label.remove_css_class(class);
        }
        self.status_label.add_css_class(css_class);
        self.status_icon.set_icon_name(icon_name);
        self.status_icon.set_visible(icon_name.is_some());
        self.spinner.set_visible(spinning);
        if spinning {
            self.spinner.start();
        } else {
            self.spinner.stop();
        }
    }

    fn set_error(&mut self, error: String) {
        self.state.error = Some(error.clone());
        self.state.status = SearchStatus::Error;
        self.set_status(SearchStatus::Error, Some(&error));
    }

    fn update_folder_labels(&self) {
        self.folder_label
            .set_label(&format!("Folder: {}", self.state.current_path.display()));
        self.file_count_label
            .set_label(&format!("{} files found", self.state.found_files.len()));
    }

    fn clear_list(list: &gtk::ListBox) {
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
    }

    fn render_files(&self) {
        Self::clear_list(&self.files_list);
        for file in &self.state.found_files {
            self.files_list.append(&file_row(file));
        }
    }

    fn render_results(&self) {
        Self::clear_list(&self.results_list);
        let visible = self.state.get_visible_results().to_vec();
        for result in &visible {
            self.results_list.append(&result_row(result));
        }
        self.load_more_button
            .set_visible(self.state.has_more_results());
    }

    fn scan_started(&mut self) {
        self.state.status = SearchStatus::Scanning;
        self.state.error = None;
        self.state.found_files.clear();
        self.state.search_results.clear();
        self.state.results_offset = 0;
        self.update_folder_labels();
        Self::clear_list(&self.files_list);
        Self::clear_list(&self.results_list);
        self.load_more_button.set_visible(false);
        self.set_status(SearchStatus::Scanning, None);
    }

    fn scan_completed(&mut self, files: Vec<FileEntry>, oversized: usize, unreadable: usize) {
        self.state.skipped_oversized_files = oversized;
        self.state.skipped_unreadable_entries = unreadable;
        self.state.found_files = files;
        self.state.status = SearchStatus::Idle;
        self.update_folder_labels();
        self.render_files();
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

    fn search_started(&mut self, query: &str) {
        self.state.search_query = query.to_owned();
        self.state.search_results.clear();
        self.state.results_offset = 0;
        self.state.status = SearchStatus::Searching;
        self.state.error = None;
        Self::clear_list(&self.results_list);
        self.load_more_button.set_visible(false);
        self.set_status(SearchStatus::Searching, None);
    }

    fn search_progress(&mut self, completed: usize, total: usize) {
        self.state.status = SearchStatus::Searching;
        self.set_status(
            SearchStatus::Searching,
            Some(&format!("Searching… {completed}/{total}")),
        );
    }

    fn search_completed(&mut self, query: String, results: Vec<SearchMatch>) {
        if self.state.search_query != query {
            return;
        }
        self.state.search_results = results;
        self.state.results_offset = 0;
        self.state.error = None;
        self.state.status = if query.trim().is_empty() {
            SearchStatus::Idle
        } else {
            SearchStatus::Success
        };
        let status = self.state.status;
        let count = self.state.search_results.len();
        self.set_status(
            status,
            Some(&format!("Search complete — {count} matching files")),
        );
        self.render_results();
    }

    fn load_more(&mut self) {
        self.state.load_more();
        self.render_results();
    }
}

fn file_row(file: &FileEntry) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let button = gtk::Button::with_label(&file.relative_path);
    button.set_halign(gtk::Align::Fill);
    button.set_hexpand(true);
    button.set_has_frame(false);
    button.set_tooltip_text(Some(&file.absolute_path.to_string_lossy()));
    button.add_css_class("file-row-button");
    let path = file.absolute_path.clone();
    button.connect_clicked(move |_| {
        if let Err(error) = open_file(&path) {
            tracing::warn!(%error, path = %path.display(), "Could not open file");
        }
    });
    row.set_child(Some(&button));
    row
}

fn result_row(result: &SearchMatch) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let container = gtk::Box::new(gtk::Orientation::Vertical, 8);
    container.set_margin_top(12);
    container.set_margin_bottom(12);
    container.set_margin_start(12);
    container.set_margin_end(12);

    let uri = gio::File::for_path(&result.file_absolute_path).uri();
    let link = gtk::LinkButton::with_label(&uri, &result.file_relative_path);
    link.add_css_class("file-link");
    link.set_halign(gtk::Align::Start);
    let path = result.file_absolute_path.clone();
    link.connect_activate(move |_| {
        if let Err(error) = open_file(&path) {
            tracing::warn!(%error, path = %path.display(), "Could not open file");
        }
    });
    container.append(&link);

    for occurrence in &result.occurrences {
        let snippet = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        snippet.set_margin_start(24);
        append_snippet_part(&snippet, &occurrence.before, false);
        append_snippet_part(&snippet, &occurrence.matched, true);
        append_snippet_part(&snippet, &occurrence.after, false);
        container.append(&snippet);
    }

    row.set_child(Some(&container));
    row
}

fn append_snippet_part(container: &gtk::Box, text: &str, matched: bool) {
    if text.is_empty() {
        return;
    }
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_selectable(true);
    label.add_css_class("context-text");
    if matched {
        label.add_css_class("search-match");
    }
    container.append(&label);
}

fn open_file(path: &Path) -> anyhow::Result<()> {
    let uri = gio::File::for_path(path).uri();
    gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
        .map_err(anyhow::Error::from)
        .map_err(|error| error.context(format!("Could not open {}", path.display())))
}

fn choose_folder(parent: &adw::ApplicationWindow, ui: Rc<RefCell<Ui>>, bridge: Rc<RuntimeBridge>) {
    let identifier = ashpd::WindowIdentifier::from_native(parent);
    glib::MainContext::default().spawn_local(async move {
        let identifier = match identifier {
            Ok(identifier) => identifier,
            Err(error) => {
                ui.borrow_mut()
                    .set_error(format!("Could not identify application window: {error}"));
                return;
            }
        };

        let request = ashpd::desktop::file_chooser::OpenFileRequest::default()
            .identifier(identifier)
            .title("Select a folder to search")
            .modal(true)
            .directory(true)
            .multiple(false);

        let result = request.send().await;
        let response = match result {
            Ok(response) => response,
            Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => return,
            Err(error) => {
                ui.borrow_mut()
                    .set_error(format!("Folder selection failed: {error}"));
                return;
            }
        };

        let Some(uri) = response.uris().first() else {
            return;
        };
        let path = match uri.to_file_path() {
            Ok(path) => path,
            Err(()) => {
                ui.borrow_mut()
                    .set_error(format!("Selected folder is not a local path: {uri}"));
                return;
            }
        };

        {
            let mut state = ui.borrow_mut();
            state.state.set_scan_path(path.clone());
            if let Some(settings) = &state.settings {
                if let Err(error) = settings.set_string("scan-path", &path.to_string_lossy()) {
                    tracing::warn!(%error, "Could not persist scan path in GSettings");
                }
            }
            state.update_folder_labels();
            state.scan_started();
        }
        bridge.send(WorkerCommand::Scan { root: path });
    });
}

fn build_ui(application: &adw::Application) {
    load_css();
    let (bridge, mut events) = RuntimeBridge::start();
    let bridge = Rc::new(bridge);

    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("Document Searcher")
        .default_width(1200)
        .default_height(800)
        .build();

    let files_list = gtk::ListBox::new();
    files_list.set_selection_mode(gtk::SelectionMode::None);
    let results_list = gtk::ListBox::new();
    results_list.set_selection_mode(gtk::SelectionMode::None);

    let folder_label = gtk::Label::new(None);
    folder_label.set_xalign(0.0);
    folder_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    folder_label.add_css_class("heading");
    let file_count_label = gtk::Label::new(Some("0 files found"));
    file_count_label.set_xalign(0.0);
    file_count_label.add_css_class("dim-label");

    let search_entry = adw::EntryRow::builder()
        .title("Search input")
        .placeholder_text("Enter the search string…")
        .build();
    let status_icon = gtk::Image::new();
    status_icon.set_visible(false);
    let status_label = gtk::Label::new(None);
    status_label.set_xalign(0.0);
    status_label.set_wrap(true);
    let spinner = gtk::Spinner::new();
    spinner.set_visible(false);
    let load_more_button = gtk::Button::with_label("Show more");
    load_more_button.add_css_class("suggested-action");
    load_more_button.set_visible(false);

    let settings = load_settings();
    let mut initial_state = AppState::new();
    if let Some(settings) = &settings {
        let configured_path = settings.string("scan-path");
        if !configured_path.is_empty() {
            initial_state.set_scan_path(PathBuf::from(configured_path.as_str()));
        }
    }

    let ui = Rc::new(RefCell::new(Ui {
        state: initial_state,
        settings,
        files_list: files_list.clone(),
        results_list: results_list.clone(),
        folder_label: folder_label.clone(),
        file_count_label: file_count_label.clone(),
        status_label: status_label.clone(),
        status_icon: status_icon.clone(),
        spinner: spinner.clone(),
        load_more_button: load_more_button.clone(),
    }));

    let change_folder = gtk::Button::with_label("Change folder");
    change_folder.add_css_class("suggested-action");
    {
        let parent = window.clone();
        let ui = Rc::clone(&ui);
        let bridge = Rc::clone(&bridge);
        change_folder.connect_clicked(move |_| {
            choose_folder(&parent, Rc::clone(&ui), Rc::clone(&bridge));
        });
    }

    {
        let ui = Rc::clone(&ui);
        let bridge = Rc::clone(&bridge);
        search_entry.connect_changed(move |entry| {
            let query = entry.text().to_string();
            {
                let mut state = ui.borrow_mut();
                state.state.search_query = query.clone();
                state.state.search_results.clear();
                state.state.results_offset = 0;
                state.state.error = None;
                state.state.status = if query.trim().is_empty() {
                    SearchStatus::Idle
                } else {
                    SearchStatus::Searching
                };
                Ui::clear_list(&state.results_list);
                state.load_more_button.set_visible(false);
                state.set_status(
                    state.state.status,
                    Some(if query.trim().is_empty() {
                        "Ready"
                    } else {
                        "Waiting for input…"
                    }),
                );
            }
            bridge.send(WorkerCommand::QueryChanged { query });
        });
    }

    {
        let ui = Rc::clone(&ui);
        load_more_button.connect_clicked(move |_| ui.borrow_mut().load_more());
    }

    let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let left_header = gtk::Box::new(gtk::Orientation::Vertical, 4);
    left_header.set_margin_top(12);
    left_header.set_margin_bottom(12);
    left_header.set_margin_start(12);
    left_header.set_margin_end(12);
    left_header.append(&folder_label);
    left_header.append(&file_count_label);
    left.append(&left_header);
    let files_scroll = gtk::ScrolledWindow::new();
    files_scroll.set_vexpand(true);
    files_scroll.set_child(Some(&files_list));
    left.append(&files_scroll);
    let folder_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    folder_actions.set_margin_top(12);
    folder_actions.set_margin_bottom(12);
    folder_actions.set_margin_start(12);
    folder_actions.set_margin_end(12);
    folder_actions.append(&change_folder);
    left.append(&folder_actions);

    let search = gtk::Box::new(gtk::Orientation::Vertical, 12);
    search.set_margin_top(24);
    search.set_margin_bottom(24);
    search.set_margin_start(24);
    search.set_margin_end(24);
    let instruction = gtk::Label::new(Some(
        "Write what you want to find. The search runs through all supported files in the selected folder.",
    ));
    instruction.set_xalign(0.0);
    instruction.set_wrap(true);
    instruction.add_css_class("heading");
    search.append(&instruction);
    search.append(&search_entry);
    let status = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    status.append(&spinner);
    status.append(&status_icon);
    status.append(&status_label);
    search.append(&status);

    let results_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let results_header = gtk::Label::new(Some("Results"));
    results_header.set_xalign(0.0);
    results_header.set_margin_start(12);
    results_header.set_margin_top(12);
    results_header.add_css_class("heading");
    results_box.append(&results_header);
    let results_scroll = gtk::ScrolledWindow::new();
    results_scroll.set_vexpand(true);
    results_scroll.set_child(Some(&results_list));
    results_box.append(&results_scroll);
    let more_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    more_box.set_halign(gtk::Align::Center);
    more_box.set_margin_top(12);
    more_box.set_margin_bottom(12);
    more_box.append(&load_more_button);
    results_box.append(&more_box);

    let right = gtk::Paned::new(gtk::Orientation::Vertical);
    right.set_wide_handle(true);
    right.set_start_child(Some(&search));
    right.set_end_child(Some(&results_box));

    let main_paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    main_paned.set_wide_handle(true);
    main_paned.set_position(600);
    main_paned.set_start_child(Some(&left));
    main_paned.set_end_child(Some(&right));
    window.set_content(Some(&main_paned));

    {
        let ui = Rc::clone(&ui);
        glib::MainContext::default().spawn_local(async move {
            while let Some(event) = events.recv().await {
                let mut ui = ui.borrow_mut();
                match event {
                    WorkerEvent::ScanStarted => ui.scan_started(),
                    WorkerEvent::ScanProgress { completed } => ui.set_status(
                        SearchStatus::Scanning,
                        Some(&format!("Scanning… {completed} files")),
                    ),
                    WorkerEvent::ScanCompleted {
                        files,
                        oversized_files,
                        unreadable_entries,
                    } => ui.scan_completed(files, oversized_files, unreadable_entries),
                    WorkerEvent::ScanError(error) => ui.set_error(error),
                    WorkerEvent::SearchStarted { query } => ui.search_started(&query),
                    WorkerEvent::SearchProgress { completed, total } => {
                        ui.search_progress(completed, total)
                    }
                    WorkerEvent::SearchCompleted { query, results } => {
                        ui.search_completed(query, results)
                    }
                    WorkerEvent::SearchCancelled { query } => {
                        if ui.state.search_query == query {
                            ui.state.status = SearchStatus::Idle;
                            ui.set_status(SearchStatus::Idle, Some("Search cancelled"));
                        }
                    }
                    WorkerEvent::SearchError { query, error } => {
                        if ui.state.search_query == query {
                            ui.set_error(error);
                        }
                    }
                }
            }
        });
    }

    let initial_path = ui.borrow().state.current_path.clone();
    ui.borrow().update_folder_labels();
    bridge.send(WorkerCommand::Scan { root: initial_path });

    window.present();
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let resource_bytes = include_bytes!(env!("DOCUMENT_SEARCHER_GRESOURCE"));
    let resource_data = glib::Bytes::from_static(resource_bytes);
    let resource =
        gio::Resource::from_data(&resource_data).expect("failed to parse embedded GTK resources");
    gio::resources_register(&resource);
    adw::init().expect("failed to initialize GTK/Libadwaita");
    let application = adw::Application::builder()
        .application_id(APPLICATION_ID)
        .build();
    application.connect_activate(build_ui);
    application.run();
}
