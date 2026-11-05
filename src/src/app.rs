use relm4::prelude::*;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, RelmApp, SimpleComponent};
use relm4::gtk::{self, gio, glib};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::model::{
    actions::AppMsg,
    state::{AppState, FileEntry, SearchMatch},
    scanner::scan_directory,
    search::search_files,
};

pub struct AppModel {
    state: Arc<Mutex<AppState>>,
    sidebar: Controller<crate::widgets::sidebar::SidebarModel>,
    search_bar: Controller<crate::widgets::search_bar::SearchBarModel>,
    results_list: Controller<crate::widgets::results_list::ResultsListModel>,
    sender: ComponentSender<AppModel>,
}

#[derive(Debug)]
pub enum AppInput {
    SelectFolder,
    FolderSelected(PathBuf),
    FolderSelectionCancelled,
    ScanCompleted(Vec<FileEntry>),
    ScanError(String),
    SearchDebounced(String),
    SearchProgress(usize, usize),
    SearchCompleted(Vec<SearchMatch>),
    SearchError(String),
    LoadMoreResults,
    OpenFile(PathBuf),
    ClearError,
}

#[relm4::component(pub)]
impl SimpleComponent for AppModel {
    type Input = AppInput;
    type Output = ();
    type Init = ();

    view! {
        #[root]
        adw::ApplicationWindow {
            set_title: "Document Searcher",
            set_default_width: 1200,
            set_default_height: 800,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,

                crate::widgets::sidebar::SidebarModel(root) {
                    set_hexpand: true,
                    set_vexpand: true,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    set_vexpand: true,

                    crate::widgets::search_bar::SearchBarModel(root) {
                        set_vexpand: false,
                    },

                    crate::widgets::results_list::ResultsListModel(root) {
                        set_vexpand: true,
                    }
                }
            }
        }
    }

    fn init(
        _init: Self::Init,
        root: &Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let state = Arc::new(Mutex::new(AppState::new()));

        let sidebar = crate::widgets::sidebar::SidebarModel::builder()
            .launch(())
            .forward(sender.input_sender(), |msg| match msg {
                crate::widgets::sidebar::SidebarMsg::UpdateFiles(_) => {}
                crate::widgets::sidebar::SidebarMsg::UpdateFolder(_) => {}
                crate::widgets::sidebar::SidebarMsg::FileActivated(_) => {}
            });

        let search_bar = crate::widgets::search_bar::SearchBarModel::builder()
            .launch(())
            .forward(sender.input_sender(), |msg| match msg {});

        let results_list = crate::widgets::results_list::ResultsListModel::builder()
            .launch(())
            .forward(sender.input_sender(), |msg| match msg {});

        let model = AppModel {
            state: state.clone(),
            sidebar,
            search_bar,
            results_list,
            sender: sender.clone(),
        };

        let widgets = view_output!();

        // Trigger initial scan
        sender.input(AppInput::SelectFolder);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>) {
        match msg {
            AppInput::SelectFolder => {
                self.select_folder();
            }
            AppInput::FolderSelected(path) => {
                self.handle_folder_selected(path);
            }
            AppInput::FolderSelectionCancelled => {
                // User cancelled, keep current folder
            }
            AppInput::ScanCompleted(files) => {
                self.handle_scan_completed(files);
            }
            AppInput::ScanError(err) => {
                self.handle_error(err);
            }
            AppInput::SearchDebounced(query) => {
                self.start_search(query);
            }
            AppInput::SearchProgress(current, total) => {
                // Could update progress UI here
            }
            AppInput::SearchCompleted(results) => {
                self.handle_search_completed(results);
            }
            AppInput::SearchError(err) => {
                self.handle_error(err);
            }
            AppInput::LoadMoreResults => {
                self.load_more_results();
            }
            AppInput::OpenFile(path) => {
                self.open_file(path);
            }
            AppInput::ClearError => {
                let state = self.state.clone();
                let sender = self.sender.clone();
                tokio::spawn(async move {
                    let mut state = state.lock().await;
                    state.error = None;
                });
            }
        }
    }
}

impl AppModel {
    fn select_folder(&self) {
        let sender = self.sender.clone();
        let state = self.state.clone();

        tokio::spawn(async move {
            // Use ASHPD file chooser portal
            use ashpd::desktop::file_chooser::{FileChooser, FileChooserFlags};
            use ashpd::WindowIdentifier;

            // Get native window handle - we'll need to get it from the GTK window
            // For now, use a simple approach
            let result = FileChooser::new()
                .title("Select Folder")
                .flags(FileChooserFlags::MODAL | FileChooserFlags::SELECT_FOLDERS)
                .select_folder(None, "Select a folder to search")
                .await;

            match result {
                Ok(response) => {
                    if let Some(folder) = response.folders().first() {
                        sender.input(AppInput::FolderSelected(folder.clone())).unwrap();
                    } else {
                        sender.input(AppInput::FolderSelectionCancelled).unwrap();
                    }
                }
                Err(e) => {
                    sender.input(AppInput::ScanError(format!("Folder selection failed: {}", e))).unwrap();
                }
            }
        });
    }

    fn handle_folder_selected(&self, path: PathBuf) {
        let state = self.state.clone();
        let sender = self.sender.clone();
        let sidebar = self.sidebar.clone();

        tokio::spawn(async move {
            // Update state with new folder
            {
                let mut state = state.lock().await;
                state.current_path = path.clone();
                state.found_files.clear();
                state.search_results.clear();
                state.results_offset = 0;
                state.search_complete = false;
                state.error = None;
            }

            // Update sidebar
            sidebar.update_folder(path.clone());

            // Start scanning
            sender.input(AppInput::ScanStarted).unwrap();

            match scan_directory(&path) {
                Ok(files) => {
                    sender.input(AppInput::ScanCompleted(files)).unwrap();
                }
                Err(e) => {
                    sender.input(AppInput::ScanError(e.to_string())).unwrap();
                }
            }
        });
    }

    fn handle_scan_completed(&self, files: Vec<FileEntry>) {
        let state = self.state.clone();
        let sender = self.sender.clone();
        let sidebar = self.sidebar.clone();

        tokio::spawn(async move {
            {
                let mut state = state.lock().await;
                state.found_files = files.clone();
            }

            sidebar.update_files(files);
        });
    }

    fn start_search(&self, query: String) {
        let state = self.state.clone();
        let sender = self.sender.clone();
        let search_bar = self.search_bar.clone();
        let results_list = self.results_list.clone();

        tokio::spawn(async move {
            // Update state
            {
                let mut state = state.lock().await;
                state.search_query = query.clone();
                state.reset_search();
                state.is_searching = true;
            }

            search_bar.set_searching(true);

            let files = {
                let state = state.lock().await;
                state.found_files.clone()
            };

            sender.input(AppInput::SearchStarted).unwrap();

            // Perform search in chunks to show progress
            let chunk_size = 50;
            let mut all_results = Vec::new();

            for chunk in files.chunks(chunk_size) {
                match search_files(chunk, &query) {
                    Ok(results) => {
                        all_results.extend(results);

                        let current = all_results.len();
                        let total = files.len();
                        sender.input(AppInput::SearchProgress(current, total)).unwrap();
                    }
                    Err(e) => {
                        sender.input(AppInput::SearchError(e.to_string())).unwrap();
                        return;
                    }
                }
            }

            sender.input(AppInput::SearchCompleted(all_results)).unwrap();
        });
    }

    fn handle_search_completed(&self, results: Vec<SearchMatch>) {
        let state = self.state.clone();
        let sender = self.sender.clone();
        let search_bar = self.search_bar.clone();
        let results_list = self.results_list.clone();

        tokio::spawn(async move {
            {
                let mut state = state.lock().await;
                state.search_results = results.clone();
                state.is_searching = false;
                state.search_complete = true;
            }

            search_bar.set_search_complete(true);
            results_list.set_results(results.clone());

            let state = state.lock().await;
            results_list.set_has_more(state.has_more_results());
        });
    }

    fn load_more_results(&self) {
        let state = self.state.clone();
        let sender = self.sender.clone();
        let results_list = self.results_list.clone();

        tokio::spawn(async move {
            let more_results = {
                let mut state = state.lock().await;
                if !state.has_more_results() {
                    return;
                }
                let visible = state.get_visible_results().to_vec();
                state.load_more();
                let new_visible = state.get_visible_results().to_vec();
                new_visible[visible.len()..].to_vec()
            };

            if !more_results.is_empty() {
                results_list.append_results(more_results);

                let state = state.lock().await;
                results_list.set_has_more(state.has_more_results());
            }
        });
    }

    fn open_file(&self, path: PathBuf) {
        tokio::spawn(async move {
            // Open file with default application (nautilus or associated app)
            use ashpd::desktop::open_uri::OpenURI;
            let _ = OpenURI::open_file(&format!("file://{}", path.display())).await;
        });
    }

    fn handle_error(&self, error: String) {
        let state = self.state.clone();
        let sender = self.sender.clone();
        let search_bar = self.search_bar.clone();

        tokio::spawn(async move {
            {
                let mut state = state.lock().await;
                state.error = Some(error.clone());
                state.is_searching = false;
            }

            search_bar.set_search_complete(false);
            // Could show toast notification here
            eprintln!("Error: {}", error);
        });
    }
}

pub fn run() {
    // Register GResource
    let res = gio::Resource::load("data/resources/org.gtk.DocumentSearcher.gresource")
        .expect("Failed to load GResource");
    gio::resources_register(&res);

    let app = RelmApp::new("org.gtk.DocumentSearcher");
    app.run::<AppModel>(());
}