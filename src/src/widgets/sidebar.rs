use relm4::prelude::*;
use relm4::gtk::prelude::*;
use relm4::factory::FactoryVecDeque;
use relm4::factory::FactoryComponent;
use relm4::gtk::{gio, glib};
use std::path::PathBuf;
use crate::model::actions::AppMsg;
use crate::model::state::FileEntry;

#[derive(Debug)]
pub struct SidebarModel {
    files: FactoryVecDeque<FileRow>,
    folder_path: PathBuf,
    file_count: usize,
    sender: ComponentSender<SidebarModel>,
}

#[derive(Debug)]
pub enum SidebarMsg {
    UpdateFiles(Vec<FileEntry>),
    UpdateFolder(PathBuf),
    FileActivated(usize),
}

#[relm4::factory(pub)]
impl FactoryComponent for FileRow {
    type Input = FileEntry;
    type Output = AppMsg;
    type CommandOutput = ();
    type ParentWidget = gtk::ListView;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 12,
            set_margin_start: 12,
            set_margin_end: 12,
            set_margin_top: 8,
            set_margin_bottom: 8,

            gtk::Image {
                set_icon_name: Some("text-x-generic-symbolic"),
                set_pixel_size: 24,
                set_valign: gtk::Align::Center,
            },

            gtk::Label {
                set_xalign: 0.0,
                set_ellipsize: pango::EllipsizeMode::End,
                set_max_width_chars: 50,
                set_label: &self.file_entry.relative_path,
                set_selectable: true,
            }
        }
    }

    fn init_model(file_entry: FileEntry, _index: usize, _sender: FactorySender<Self>) -> Self {
        Self { file_entry }
    }

    struct FileRowWidgets {
        root: gtk::Box,
    }
}

#[relm4::component(pub)]
impl Component for SidebarModel {
    type CommandOutput = ();
    type Input = SidebarMsg;
    type Output = AppMsg;

    view! {
        #[root]
        adw::NavigationPage {
            set_title: "Files",

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 0,

                adw::ToolbarView {
                    add_top_bar = &adw::HeaderBar {
                        set_title_widget: &gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 2,

                            gtk::Label {
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_ellipsize: pango::EllipsizeMode::End,
                                set_max_width_chars: 50,
                                set_label: &format!("Folder: {}", self.folder_path.display()),
                                add_css_class: "heading",
                            },

                            gtk::Label {
                                set_xalign: 0.0,
                                set_label: &format!("{} files found", self.file_count),
                                add_css_class: "dim-label",
                            }
                        }
                    },

                    gtk::ScrolledWindow {
                        set_hscrollbar_policy: gtk::PolicyType::Never,
                        set_vexpand: true,

                        gtk::ListView {
                            set_factory: &gtk::SignalListItemFactory::new(),
                            set_model: Some(self.files.clone()),

                            connect_activate[n] => AppMsg::OpenFile(self.files.get(n).unwrap().file_entry.absolute_path.clone()),
                        }
                    }
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_margin_top: 12,
                    set_margin_bottom: 12,
                    set_margin_start: 12,
                    set_margin_end: 12,

                    gtk::Button {
                        set_label: "Change Folder",
                        set_hexpand: true,
                        set_valign: gtk::Align::Center,
                        connect_clicked => AppMsg::SelectFolder,
                    }
                }
            }
        }
    }

    fn init(
        init: Self::Input,
        root: &Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = SidebarModel {
            files: FactoryVecDeque::new(sender.clone()),
            folder_path: PathBuf::from("/home/documents"),
            file_count: 0,
            sender,
        };

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>) {
        match msg {
            SidebarMsg::UpdateFiles(files) => {
                self.file_count = files.len();
                self.files.clear();
                for file in files {
                    self.files.push_back(file);
                }
            }
            SidebarMsg::UpdateFolder(path) => {
                self.folder_path = path;
            }
            SidebarMsg::FileActivated(_index) => {
                // Handled by connect_activate in view
            }
        }
    }
}

impl SidebarModel {
    pub fn update_files(&mut self, files: Vec<FileEntry>) {
        self.sender.input(SidebarMsg::UpdateFiles(files)).unwrap();
    }

    pub fn update_folder(&mut self, path: PathBuf) {
        self.sender.input(SidebarMsg::UpdateFolder(path)).unwrap();
    }
}