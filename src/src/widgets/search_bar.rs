use relm4::prelude::*;
use relm4::gtk::prelude::*;
use relm4::gtk::{self, glib};
use crate::model::actions::AppMsg;

#[derive(Debug)]
pub struct SearchBarModel {
    query: String,
    is_searching: bool,
    search_complete: bool,
    debounce_timer: Option<glib::SourceId>,
    sender: ComponentSender<SearchBarModel>,
}

#[derive(Debug)]
pub enum SearchBarMsg {
    QueryChanged(String),
    SetSearching(bool),
    SetSearchComplete(bool),
    TriggerSearch(String),
}

#[relm4::component(pub)]
impl Component for SearchBarModel {
    type CommandOutput = ();
    type Input = SearchBarMsg;
    type Output = AppMsg;

    view! {
        #[root]
        adw::NavigationPage {
            set_title: "Search",

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 12,
                set_margin_top: 24,
                set_margin_bottom: 24,
                set_margin_start: 24,
                set_margin_end: 24,

                gtk::Label {
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_label: "Write what you want to find! The search will be performed through all the files found in a folder.",
                    add_css_class: "heading",
                },

                adw::EntryRow {
                    set_title: "Search input",
                    set_placeholder_text: "Enter the search string...",
                    set_show_apply_button: false,
                    set_activates_default: false,
                    connect_changed[entry] => SearchBarMsg::QueryChanged(entry.text().to_string()),
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    set_halign: gtk::Align::Start,

                    gtk::Spinner {
                        set_visible: self.is_searching,
                        set_size_request: (24, 24),
                        start: (),
                    },

                    gtk::Label {
                        set_xalign: 0.0,
                        set_valign: gtk::Align::Center,
                        set_visible: self.is_searching || self.search_complete,
                        set_label: {
                            if self.is_searching {
                                "Searching..."
                            } else if self.search_complete && !self.query.is_empty() {
                                "Search complete"
                            } else {
                                ""
                            }
                        },
                        add_css_class: if self.is_searching { "search-status-searching" } else { "search-status-success" },
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
        let model = SearchBarModel {
            query: String::new(),
            is_searching: false,
            search_complete: false,
            debounce_timer: None,
            sender,
        };

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>) {
        match msg {
            SearchBarMsg::QueryChanged(query) => {
                self.query = query.clone();

                // Cancel previous debounce timer
                if let Some(timer) = self.debounce_timer.take() {
                    timer.remove();
                }

                // Debounce: wait 300ms before triggering search
                if !query.trim().is_empty() {
                    let sender = self.sender.clone();
                    self.debounce_timer = Some(glib::timeout_future_seconds(300 / 1000).await.into_source().attach(None, move || {
                        sender.input(SearchBarMsg::TriggerSearch(query.clone())).unwrap();
                        glib::ControlFlow::Break
                    }));
                } else {
                    self.sender.output(AppMsg::SearchQueryChanged(query)).unwrap();
                }
            }
            SearchBarMsg::SetSearching(searching) => {
                self.is_searching = searching;
                if searching {
                    self.search_complete = false;
                }
            }
            SearchBarMsg::SetSearchComplete(complete) => {
                self.search_complete = complete;
                self.is_searching = false;
            }
            SearchBarMsg::TriggerSearch(query) => {
                self.debounce_timer = None;
                self.sender.output(AppMsg::SearchDebounced(query)).unwrap();
            }
        }
    }
}

impl SearchBarModel {
    pub fn set_searching(&self, searching: bool) {
        self.sender.input(SearchBarMsg::SetSearching(searching)).unwrap();
    }

    pub fn set_search_complete(&self, complete: bool) {
        self.sender.input(SearchBarMsg::SetSearchComplete(complete)).unwrap();
    }
}