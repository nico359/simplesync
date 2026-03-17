use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;

use crate::keyring::{self, Credentials};
use crate::webdav::WebDAVClient;
use crate::window::SimplesyncWindow;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/nico359/simplesync/account_page.ui")]
    pub struct SimplesyncAccountPage {
        #[template_child]
        pub server_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub username_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub password_entry: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        pub test_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub test_spinner: TemplateChild<gtk::Spinner>,
        #[template_child]
        pub test_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub save_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub remove_button: TemplateChild<gtk::Button>,

        pub window: RefCell<Option<SimplesyncWindow>>,
    }

    impl std::fmt::Debug for SimplesyncAccountPage {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SimplesyncAccountPage").finish()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SimplesyncAccountPage {
        const NAME: &'static str = "SimplesyncAccountPage";
        type Type = super::SimplesyncAccountPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SimplesyncAccountPage {}
    impl WidgetImpl for SimplesyncAccountPage {}
    impl NavigationPageImpl for SimplesyncAccountPage {}
}

glib::wrapper! {
    pub struct SimplesyncAccountPage(ObjectSubclass<imp::SimplesyncAccountPage>)
        @extends gtk::Widget, adw::NavigationPage;
}

impl SimplesyncAccountPage {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    pub fn set_window(&self, window: &SimplesyncWindow) {
        self.imp().window.replace(Some(window.clone()));
        self.setup_signals();
        self.load_existing_credentials();
    }

    fn window(&self) -> SimplesyncWindow {
        self.imp().window.borrow().clone().expect("Window not set")
    }

    fn setup_signals(&self) {
        let page = self.clone();
        self.imp().test_button.connect_clicked(move |_| {
            page.test_connection();
        });

        let page = self.clone();
        self.imp().save_button.connect_clicked(move |_| {
            page.save_credentials();
        });

        let page = self.clone();
        self.imp().remove_button.connect_clicked(move |_| {
            page.remove_account();
        });
    }

    fn load_existing_credentials(&self) {
        // Run keyring lookup on background thread
        let (tx, rx) = std::sync::mpsc::channel::<Option<Credentials>>();
        std::thread::spawn(move || {
            let creds = keyring::load_credentials_sync();
            let _ = tx.send(creds);
        });

        let page = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            match rx.try_recv() {
                Ok(Some(creds)) => {
                    page.imp().server_entry.set_text(&creds.server_url);
                    page.imp().username_entry.set_text(&creds.username);
                    page.imp().password_entry.set_text(&creds.app_password);
                    page.imp().remove_button.set_visible(true);
                    glib::ControlFlow::Break
                }
                Ok(None) => glib::ControlFlow::Break,
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            }
        });
    }

    fn test_connection(&self) {
        let server = self.imp().server_entry.text().trim().to_string();
        let username = self.imp().username_entry.text().trim().to_string();
        let password = self.imp().password_entry.text().to_string();

        if server.is_empty() || username.is_empty() || password.is_empty() {
            self.set_test_status("Please fill in all fields", false);
            return;
        }

        self.imp().test_spinner.set_visible(true);
        self.imp().test_spinner.set_spinning(true);
        self.imp().test_label.set_text("Testing...");
        self.imp().test_button.set_sensitive(false);

        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        std::thread::spawn(move || {
            let client = WebDAVClient::new(&server, &username, &password);
            let result = client.test_connection().map_err(|e| e.to_string());
            let _ = tx.send(result);
        });

        let page = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            match rx.try_recv() {
                Ok(result) => {
                    page.imp().test_spinner.set_visible(false);
                    page.imp().test_spinner.set_spinning(false);
                    page.imp().test_button.set_sensitive(true);
                    match result {
                        Ok(()) => page.set_test_status("Connection successful!", true),
                        Err(e) => page.set_test_status(&format!("Failed: {}", e), false),
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            }
        });
    }

    fn set_test_status(&self, message: &str, success: bool) {
        let label = &self.imp().test_label;
        label.set_text(message);
        label.remove_css_class("success");
        label.remove_css_class("error");
        if success {
            label.add_css_class("success");
        } else {
            label.add_css_class("error");
        }
    }

    fn save_credentials(&self) {
        let server = self.imp().server_entry.text().trim().to_string();
        let username = self.imp().username_entry.text().trim().to_string();
        let password = self.imp().password_entry.text().to_string();

        if server.is_empty() || username.is_empty() || password.is_empty() {
            self.window().show_toast("Please fill in all fields");
            return;
        }

        let creds = Credentials {
            server_url: server,
            username,
            app_password: password,
        };

        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        std::thread::spawn(move || {
            let success = keyring::store_credentials_sync(&creds);
            let _ = tx.send(success);
        });

        let page = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            match rx.try_recv() {
                Ok(success) => {
                    if success {
                        page.window().show_toast("Account saved");
                        page.imp().remove_button.set_visible(true);
                    } else {
                        page.window().show_toast("Failed to save credentials");
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            }
        });
    }

    fn remove_account(&self) {
        let dialog = adw::AlertDialog::builder()
            .heading("Remove Account?")
            .body("This will remove the stored credentials. Your sync targets will be kept.")
            .build();
        dialog.add_responses(&[("cancel", "Cancel"), ("remove", "Remove")]);
        dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));

        let page = self.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "remove" {
                let (tx, rx) = std::sync::mpsc::channel::<bool>();
                std::thread::spawn(move || {
                    let success = keyring::clear_credentials_sync();
                    let _ = tx.send(success);
                });

                let page_inner = page.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    match rx.try_recv() {
                        Ok(success) => {
                            if success {
                                page_inner.imp().server_entry.set_text("");
                                page_inner.imp().username_entry.set_text("");
                                page_inner.imp().password_entry.set_text("");
                                page_inner.imp().remove_button.set_visible(false);
                                page_inner.imp().test_label.set_text("");
                                page_inner.window().show_toast("Account removed");
                            } else {
                                page_inner.window().show_toast("Failed to remove credentials");
                            }
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => glib::ControlFlow::Break,
                    }
                });
            }
        });

        dialog.present(Some(&self.window()));
    }
}
