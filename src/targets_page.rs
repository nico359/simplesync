use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;

use crate::db::Target;
use crate::keyring;
use crate::push::{self, PushProgress};
use crate::target_edit_page::SimplesyncTargetEditPage;
use crate::webdav::WebDAVClient;
use crate::window::SimplesyncWindow;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/nico359/simplesync/targets_page.ui")]
    pub struct SimplesyncTargetsPage {
        #[template_child]
        pub add_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub push_all_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub content_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub targets_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub empty_add_button: TemplateChild<gtk::Button>,

        pub window: RefCell<Option<SimplesyncWindow>>,
    }

    impl std::fmt::Debug for SimplesyncTargetsPage {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SimplesyncTargetsPage").finish()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SimplesyncTargetsPage {
        const NAME: &'static str = "SimplesyncTargetsPage";
        type Type = super::SimplesyncTargetsPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SimplesyncTargetsPage {}
    impl WidgetImpl for SimplesyncTargetsPage {}
    impl NavigationPageImpl for SimplesyncTargetsPage {}
}

glib::wrapper! {
    pub struct SimplesyncTargetsPage(ObjectSubclass<imp::SimplesyncTargetsPage>)
        @extends gtk::Widget, adw::NavigationPage;
}

impl SimplesyncTargetsPage {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    pub fn set_window(&self, window: &SimplesyncWindow) {
        self.imp().window.replace(Some(window.clone()));
        self.setup_signals();
        self.refresh_targets();
    }

    fn window(&self) -> SimplesyncWindow {
        self.imp().window.borrow().clone().expect("Window not set")
    }

    fn setup_signals(&self) {
        let page = self.clone();
        self.imp().add_button.connect_clicked(move |_| {
            page.show_add_target();
        });

        let page = self.clone();
        self.imp().empty_add_button.connect_clicked(move |_| {
            page.show_add_target();
        });

        let page = self.clone();
        self.imp().push_all_button.connect_clicked(move |_| {
            page.push_all();
        });

        // Set up application account action
        let page = self.clone();
        if let Some(app) = self.window().application() {
            let account_action = gio::SimpleAction::new("account", None);
            let page_ref = page.clone();
            account_action.connect_activate(move |_, _| {
                page_ref.show_account_page();
            });
            app.add_action(&account_action);
        }
    }

    pub fn refresh_targets(&self) {
        let list = &self.imp().targets_list;

        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        let window = self.window();
        let targets = window.db().get_targets().unwrap_or_default();

        if targets.is_empty() {
            self.imp().content_stack.set_visible_child_name("empty");
        } else {
            self.imp().content_stack.set_visible_child_name("list");
            for target in &targets {
                let row = self.create_target_row(target);
                list.append(&row);
            }
        }
    }

    fn create_target_row(&self, target: &Target) -> adw::ActionRow {
        let local_name = std::path::Path::new(&target.local_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| target.local_path.clone());

        let subtitle = format!("{} → {}", target.local_path, target.remote_path);

        let row = adw::ActionRow::builder()
            .title(&local_name)
            .subtitle(&subtitle)
            .activatable(true)
            .build();

        let mode_label = gtk::Label::builder()
            .label(if target.mode == "mirror" { "Mirror" } else { "Upload" })
            .valign(gtk::Align::Center)
            .build();
        mode_label.add_css_class("caption");
        if target.mode == "mirror" {
            mode_label.add_css_class("warning");
        }
        row.add_suffix(&mode_label);

        let push_button = gtk::Button::builder()
            .icon_name("emblem-synchronizing-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text("Push")
            .build();
        push_button.add_css_class("flat");

        let target_id = target.id;
        let page = self.clone();
        push_button.connect_clicked(move |btn| {
            page.push_target(target_id, Some(btn.clone()));
        });
        row.add_suffix(&push_button);

        let target_id = target.id;
        let page = self.clone();
        row.connect_activated(move |_| {
            page.show_edit_target(target_id);
        });

        row
    }

    fn show_add_target(&self) {
        let edit_page = SimplesyncTargetEditPage::new();
        let page = self.clone();
        edit_page.set_window(&self.window(), None, move || {
            page.refresh_targets();
        });
        self.window().navigation_view().push(&edit_page);
    }

    fn show_edit_target(&self, target_id: i64) {
        let window = self.window();
        let target = window.db().get_target(target_id).ok();
        let edit_page = SimplesyncTargetEditPage::new();
        let page = self.clone();
        edit_page.set_window(&self.window(), target.as_ref(), move || {
            page.refresh_targets();
        });
        self.window().navigation_view().push(&edit_page);
    }

    fn show_account_page(&self) {
        let account_page = crate::account_page::SimplesyncAccountPage::new();
        account_page.set_window(&self.window());
        self.window().navigation_view().push(&account_page);
    }

    fn push_target(&self, target_id: i64, button: Option<gtk::Button>) {
        let creds = match keyring::load_credentials_sync() {
            Some(c) => c,
            None => {
                self.window().show_toast("No account configured. Set up an account first.");
                return;
            }
        };

        let window = self.window();
        let target = match window.db().get_target(target_id) {
            Ok(t) => t,
            Err(_) => {
                self.window().show_toast("Target not found");
                return;
            }
        };

        let client = WebDAVClient::new(&creds.server_url, &creds.username, &creds.app_password);
        let db_path = crate::db::Database::db_path();

        if let Some(ref btn) = button {
            btn.set_sensitive(false);
        }

        let (tx, rx) = std::sync::mpsc::channel();
        push::run_push(client, target, db_path, false, tx);

        let page = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            while let Ok(progress) = rx.try_recv() {
                match progress {
                    PushProgress::File { .. } => {}
                    PushProgress::Complete { success, summary } => {
                        if let Some(ref btn) = button {
                            btn.set_sensitive(true);
                        }
                        let msg = if success {
                            format!("Done: {} uploaded, {} skipped", summary.uploaded, summary.skipped)
                        } else {
                            format!("Completed with {} error(s)", summary.errors.len())
                        };
                        page.window().show_toast(&msg);
                        page.refresh_targets();
                        return glib::ControlFlow::Break;
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    fn push_all(&self) {
        let window = self.window();
        let targets = window.db().get_targets().unwrap_or_default();
        if targets.is_empty() {
            self.window().show_toast("No targets to push");
            return;
        }

        let creds = match keyring::load_credentials_sync() {
            Some(c) => c,
            None => {
                self.window().show_toast("No account configured");
                return;
            }
        };

        self.imp().push_all_button.set_sensitive(false);
        let target_ids: Vec<i64> = targets.iter().map(|t| t.id).collect();
        self.push_sequential(target_ids, 0, creds);
    }

    fn push_sequential(&self, target_ids: Vec<i64>, index: usize, creds: keyring::Credentials) {
        if index >= target_ids.len() {
            self.imp().push_all_button.set_sensitive(true);
            self.window().show_toast("All targets pushed");
            self.refresh_targets();
            return;
        }

        let window = self.window();
        let target = match window.db().get_target(target_ids[index]) {
            Ok(t) => t,
            Err(_) => {
                self.push_sequential(target_ids, index + 1, creds);
                return;
            }
        };

        let client = WebDAVClient::new(&creds.server_url, &creds.username, &creds.app_password);
        let db_path = crate::db::Database::db_path();

        let (tx, rx) = std::sync::mpsc::channel();
        push::run_push(client, target, db_path, false, tx);

        let page = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            while let Ok(progress) = rx.try_recv() {
                if let PushProgress::Complete { .. } = progress {
                    page.push_sequential(target_ids.clone(), index + 1, creds.clone());
                    return glib::ControlFlow::Break;
                }
            }
            glib::ControlFlow::Continue
        });
    }
}
