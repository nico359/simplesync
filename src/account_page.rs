use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;

use crate::keyring::{self, Credentials};
use crate::webdav::WebDAVClient;
use crate::window::SimplesyncWindow;

#[derive(serde::Deserialize)]
struct LoginInitResponse {
    poll: PollInfo,
    login: String,
}

#[derive(serde::Deserialize)]
struct PollInfo {
    token: String,
    endpoint: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginPollResponse {
    server: String,
    login_name: String,
    app_password: String,
}

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/nico359/simplesync/account_page.ui")]
    pub struct SimplesyncAccountPage {
        #[template_child]
        pub account_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub server_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub login_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub login_spinner: TemplateChild<gtk::Spinner>,
        #[template_child]
        pub login_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub server_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub username_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub test_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub test_spinner: TemplateChild<gtk::Spinner>,
        #[template_child]
        pub test_label: TemplateChild<gtk::Label>,
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
        self.imp().login_button.connect_clicked(move |_| {
            page.start_login_flow();
        });

        let page = self.clone();
        self.imp().test_button.connect_clicked(move |_| {
            page.test_connection();
        });

        let page = self.clone();
        self.imp().remove_button.connect_clicked(move |_| {
            page.remove_account();
        });
    }

    fn load_existing_credentials(&self) {
        let (tx, rx) = std::sync::mpsc::channel::<Option<Credentials>>();
        std::thread::spawn(move || {
            let creds = keyring::load_credentials_sync();
            let _ = tx.send(creds);
        });

        let page = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            match rx.try_recv() {
                Ok(Some(creds)) => {
                    page.show_logged_in(&creds.server_url, &creds.username);
                    glib::ControlFlow::Break
                }
                Ok(None) => {
                    page.imp().account_stack.set_visible_child_name("logged_out");
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            }
        });
    }

    fn show_logged_in(&self, server: &str, username: &str) {
        self.imp().server_row.set_subtitle(server);
        self.imp().username_row.set_subtitle(username);
        self.imp().account_stack.set_visible_child_name("logged_in");
    }

    fn show_logged_out(&self) {
        self.imp().server_entry.set_text("");
        self.imp().login_label.set_text("");
        self.imp().login_spinner.set_visible(false);
        self.imp().login_spinner.set_spinning(false);
        self.imp().login_button.set_sensitive(true);
        self.imp().account_stack.set_visible_child_name("logged_out");
    }

    fn start_login_flow(&self) {
        let server = self.imp().server_entry.text().trim().to_string();
        if server.is_empty() {
            self.set_login_status("Please enter a server URL", false);
            return;
        }

        // Normalize: strip trailing slash
        let server = server.trim_end_matches('/').to_string();

        self.imp().login_button.set_sensitive(false);
        self.imp().login_spinner.set_visible(true);
        self.imp().login_spinner.set_spinning(true);
        self.set_login_status("Initiating login…", false);

        // Step 1: POST to /index.php/login/v2 to get login URL and poll info
        enum LoginMsg {
            OpenBrowser(String, String, String), // login_url, poll_endpoint, poll_token
            Credentials(String, String, String),  // server, username, app_password
            Error(String),
        }

        let (tx, rx) = std::sync::mpsc::channel::<LoginMsg>();

        let server_clone = server.clone();
        std::thread::spawn(move || {
            let client = match reqwest::blocking::Client::builder()
                .user_agent("SimpleSync")
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(LoginMsg::Error(format!("HTTP client error: {}", e)));
                    return;
                }
            };

            // Initiate Login Flow v2
            let init_url = format!("{}/index.php/login/v2", server_clone);
            let resp = match client.post(&init_url).send() {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(LoginMsg::Error(format!("Could not reach server: {}", e)));
                    return;
                }
            };

            if !resp.status().is_success() {
                let _ = tx.send(LoginMsg::Error(format!(
                    "Server returned {}. Is this a Nextcloud server?",
                    resp.status()
                )));
                return;
            }

            let init: LoginInitResponse = match resp.json() {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.send(LoginMsg::Error(format!("Invalid server response: {}", e)));
                    return;
                }
            };

            // Tell UI to open the browser
            let _ = tx.send(LoginMsg::OpenBrowser(
                init.login.clone(),
                init.poll.endpoint.clone(),
                init.poll.token.clone(),
            ));

            // Step 2: Poll for credentials (up to 5 minutes)
            let max_attempts = 150; // 150 * 2s = 5 minutes
            for _ in 0..max_attempts {
                std::thread::sleep(std::time::Duration::from_secs(2));

                let poll_resp = match client
                    .post(&init.poll.endpoint)
                    .form(&[("token", &init.poll.token)])
                    .send()
                {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                if poll_resp.status() == reqwest::StatusCode::NOT_FOUND {
                    // User hasn't logged in yet
                    continue;
                }

                if poll_resp.status().is_success() {
                    match poll_resp.json::<LoginPollResponse>() {
                        Ok(creds) => {
                            let _ = tx.send(LoginMsg::Credentials(
                                creds.server,
                                creds.login_name,
                                creds.app_password,
                            ));
                            return;
                        }
                        Err(e) => {
                            let _ = tx.send(LoginMsg::Error(format!("Invalid credentials response: {}", e)));
                            return;
                        }
                    }
                }
            }

            let _ = tx.send(LoginMsg::Error("Login timed out. Please try again.".to_string()));
        });

        let page = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
            match rx.try_recv() {
                Ok(LoginMsg::OpenBrowser(login_url, _endpoint, _token)) => {
                    // Open login URL in default browser
                    let launcher = gtk::UriLauncher::new(&login_url);
                    launcher.launch(
                        gtk::Window::NONE,
                        gtk::gio::Cancellable::NONE,
                        |_| {},
                    );
                    page.set_login_status("Waiting for browser login…", false);
                    glib::ControlFlow::Continue
                }
                Ok(LoginMsg::Credentials(server_url, username, app_password)) => {
                    page.imp().login_spinner.set_visible(false);
                    page.imp().login_spinner.set_spinning(false);

                    // Save credentials
                    let creds = Credentials {
                        server_url: server_url.clone(),
                        username: username.clone(),
                        app_password,
                    };

                    let (save_tx, save_rx) = std::sync::mpsc::channel::<bool>();
                    std::thread::spawn(move || {
                        let success = keyring::store_credentials_sync(&creds);
                        let _ = save_tx.send(success);
                    });

                    let page_inner = page.clone();
                    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                        match save_rx.try_recv() {
                            Ok(true) => {
                                page_inner.show_logged_in(&server_url, &username);
                                page_inner.window().show_toast("Logged in successfully");
                                glib::ControlFlow::Break
                            }
                            Ok(false) => {
                                page_inner.set_login_status("Failed to save credentials", true);
                                page_inner.imp().login_button.set_sensitive(true);
                                glib::ControlFlow::Break
                            }
                            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                            Err(_) => glib::ControlFlow::Break,
                        }
                    });

                    glib::ControlFlow::Break
                }
                Ok(LoginMsg::Error(msg)) => {
                    page.imp().login_spinner.set_visible(false);
                    page.imp().login_spinner.set_spinning(false);
                    page.imp().login_button.set_sensitive(true);
                    page.set_login_status(&msg, true);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            }
        });
    }

    fn set_login_status(&self, message: &str, is_error: bool) {
        let label = &self.imp().login_label;
        label.set_text(message);
        label.remove_css_class("success");
        label.remove_css_class("error");
        if is_error {
            label.add_css_class("error");
        }
    }

    fn test_connection(&self) {
        self.imp().test_spinner.set_visible(true);
        self.imp().test_spinner.set_spinning(true);
        self.imp().test_label.set_text("Testing…");
        self.imp().test_button.set_sensitive(false);

        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        std::thread::spawn(move || {
            let creds = match keyring::load_credentials_sync() {
                Some(c) => c,
                None => {
                    let _ = tx.send(Err("No credentials found".to_string()));
                    return;
                }
            };
            let client = WebDAVClient::new(&creds.server_url, &creds.username, &creds.app_password);
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
                    let label = &page.imp().test_label;
                    label.remove_css_class("success");
                    label.remove_css_class("error");
                    match result {
                        Ok(()) => {
                            label.set_text("Connection successful!");
                            label.add_css_class("success");
                        }
                        Err(e) => {
                            label.set_text(&format!("Failed: {}", e));
                            label.add_css_class("error");
                        }
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
                                page_inner.show_logged_out();
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
