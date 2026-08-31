use relm4::prelude::*;
use relm4::gtk::prelude::*;
use relm4::gtk::{self, gio, glib, pango};
use relm4::factory::FactoryVecDeque;
use relm4::factory::FactoryComponent;
use crate::model::actions::AppMsg;
use crate::model::state::SearchMatch;

#[derive(Debug)]
pub struct ResultRowModel {
    search_match: SearchMatch,
    result_number: usize,
    sender: FactorySender<Self>,
}

#[derive(Debug)]
pub enum ResultRowMsg {
    OpenFile,
}

#[relm4::factory(pub)]
impl FactoryComponent for ResultRowModel {
    type Input = (SearchMatch, usize);
    type Output = AppMsg;
    type CommandOutput = ();
    type ParentWidget = gtk::ListView;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,
            set_margin_start: 12,
            set_margin_end: 12,
            set_margin_top: 12,
            set_margin_bottom: 12,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,

                gtk::Label {
                    set_xalign: 0.0,
                    set_halign: gtk::Align::Start,
                    set_valign: gtk::Align::Start,
                    set_selectable: true,
                    set_label: &format!("{}.", self.result_number),
                    add_css_class: "heading",
                    add_css_class: "dim-label",
                    add_css_class: "result-number",
                },

                gtk::LinkButton {
                    set_halign: gtk::Align::Start,
                    set_valign: gtk::Align::Start,
                    set_label: &self.search_match.file_relative_path,
                    set_uri: &format!("file://{}", std::env::current_dir().unwrap().join(&self.search_match.file_relative_path).display()),
                    add_css_class: "file-link",
                    connect_activate => ResultRowMsg::OpenFile,
                }
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 4,
                set_margin_start: 40,

                #[iterate]
                for context in &self.search_match.contexts {
                    gtk::Label {
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_selectable: true,
                        set_label: context,
                        add_css_class: "context-text",
                        set_use_markup: false,
                    }
                }
            }
        }
    }

    fn init_model(
        input: Self::Input,
        _index: usize,
        _sender: FactorySender<Self>,
    ) -> Self {
        let (search_match, result_number) = input;
        Self {
            search_match,
            result_number,
            sender: _sender,
        }
    }

    fn update(&mut self, msg: Self::Input, _sender: FactorySender<Self>) {
        self.search_match = msg.0;
        self.result_number = msg.1;
    }
}

#[derive(Debug)]
pub struct ResultsListModel {
    results: FactoryVecDeque<ResultRowModel>,
    has_more: bool,
    sender: ComponentSender<ResultsListModel>,
}

#[derive(Debug)]
pub enum ResultsListMsg {
    SetResults(Vec<SearchMatch>),
    AppendResults(Vec<SearchMatch>),
    SetHasMore(bool),
    LoadMore,
}

#[relm4::component(pub)]
impl Component for ResultsListModel {
    type CommandOutput = ();
    type Input = ResultsListMsg;
    type Output = AppMsg;

    view! {
        #[root]
        adw::NavigationPage {
            set_title: "Results",

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 0,

                adw::ToolbarView {
                    add_top_bar = &adw::HeaderBar {
                        set_title_widget = &gtk::Label {
                            set_xalign: 0.0,
                            set_label: "Result",
                            add_css_class: "heading",
                        }
                    },

                    gtk::ScrolledWindow {
                        set_hscrollbar_policy: gtk::PolicyType::Never,
                        set_vexpand: true,

                        gtk::ListView {
                            set_factory: &gtk::SignalListItemFactory::new(),
                            set_model: Some(self.results.clone()),
                        }
                    }
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    set_halign: gtk::Align::Center,
                    set_margin_top: 12,
                    set_margin_bottom: 12,
                    set_visible: self.has_more,

                    gtk::Button {
                        set_label: "Show more",
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        add_css_class: "suggested-action",
                        connect_clicked => ResultsListMsg::LoadMore,
                    }
                }
            }
        }
    }

    fn init(
        _init: Self::Input,
        root: &Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = ResultsListModel {
            results: FactoryVecDeque::new(sender.clone()),
            has_more: false,
            sender,
        };

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>) {
        match msg {
            ResultsListMsg::SetResults(results) => {
                self.results.clear();
                for (idx, result) in results.into_iter().enumerate() {
                    self.results.push_back((result, idx + 1));
                }
            }
            ResultsListMsg::AppendResults(results) => {
                let start_idx = self.results.len();
                for (idx, result) in results.into_iter().enumerate() {
                    self.results.push_back((result, start_idx + idx + 1));
                }
            }
            ResultsListMsg::SetHasMore(has_more) => {
                self.has_more = has_more;
            }
            ResultsListMsg::LoadMore => {
                self.sender.output(AppMsg::LoadMoreResults).unwrap();
            }
        }
    }
}

impl ResultsListModel {
    pub fn set_results(&self, results: Vec<SearchMatch>) {
        self.sender.input(ResultsListMsg::SetResults(results)).unwrap();
    }

    pub fn append_results(&self, results: Vec<SearchMatch>) {
        self.sender.input(ResultsListMsg::AppendResults(results)).unwrap();
    }

    pub fn set_has_more(&self, has_more: bool) {
        self.sender.input(ResultsListMsg::SetHasMore(has_more)).unwrap();
    }
}