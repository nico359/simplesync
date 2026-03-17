use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;

use crate::keyring::Credentials;
use crate::webdav::{RemoteItem, WebDAVClient};
use crate::window::SimplesyncWindow;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/nico359/simplesync/remote_browser_page.ui")]
    pub struct SimplesyncRemoteBrowserPage {
        #[template_child]
        pub select_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub content_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub path_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub folder_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub new_folder_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub error_status: TemplateChild<adw::StatusPage>,

        pub window: RefCell<Option<SimplesyncWindow>>,
        pub current_path: RefCell<String>,
        pub creds: RefCell<Option<Credentials>>,
        pub on_select: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
    }

    impl std::fmt::Debug for SimplesyncRemoteBrowserPage {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SimplesyncRemoteBrowserPage").finish()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SimplesyncRemoteBrowserPage {
        const NAME: &'static str = "SimplesyncRemoteBrowserPage";
        type Type = super::SimplesyncRemoteBrowserPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SimplesyncRemoteBrowserPage {}
    impl WidgetImpl for SimplesyncRemoteBrowserPage {}
    impl NavigationPageImpl for SimplesyncRemoteBrowserPage {}
}

glib::wrapper! {
    pub struct SimplesyncRemoteBrowserPage(ObjectSubclass<imp::SimplesyncRemoteBrowserPage>)
        @extends gtk::Widget, adw::NavigationPage;
}

impl SimplesyncRemoteBrowserPage {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    pub fn set_window<F: Fn(String) + 'static>(
        &self,
        window: &SimplesyncWindow,
        creds: &Credentials,
        on_select: F,
    ) {
        self.imp().window.replace(Some(window.clone()));
        self.imp().on_select.replace(Some(Box::new(on_select)));
        self.imp().current_path.replace("/".to_string());
        self.imp().creds.replace(Some(creds.clone()));

        self.setup_signals();
        self.load_directory("/");
    }

    fn window(&self) -> SimplesyncWindow {
        self.imp().window.borrow().clone().expect("Window not set")
    }

    fn make_client(&self) -> Option<WebDAVClient> {
        let creds = self.imp().creds.borrow();
        creds.as_ref().map(|c| WebDAVClient::new(&c.server_url, &c.username, &c.app_password))
    }

    fn setup_signals(&self) {
        let page = self.clone();
        self.imp().select_button.connect_clicked(move |_| {
            let path = page.imp().current_path.borrow().clone();
            if let Some(ref callback) = *page.imp().on_select.borrow() {
                callback(path);
            }
            page.window().navigation_view().pop();
        });

        let page = self.clone();
        self.imp().new_folder_button.connect_clicked(move |_| {
            page.show_new_folder_dialog();
        });
    }

    fn load_directory(&self, path: &str) {
        self.imp().content_stack.set_visible_child_name("loading");
        self.imp().current_path.replace(path.to_string());
        self.imp().path_label.set_text(path);

        let client = match self.make_client() {
            Some(c) => c,
            None => return,
        };
        let path_owned = path.to_string();

        let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<RemoteItem>, String>>();
        std::thread::spawn(move || {
            let result = client.list_directory(&path_owned).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });

        let page = self.clone();
        let path_for_cb = path.to_string();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            match rx.try_recv() {
                Ok(Ok(items)) => {
                    page.populate_list(&path_for_cb, items);
                    page.imp().content_stack.set_visible_child_name("content");
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    page.imp().error_status.set_description(Some(&e));
                    page.imp().content_stack.set_visible_child_name("error");
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            }
        });
    }

    fn populate_list(&self, current_path: &str, items: Vec<RemoteItem>) {
        let list = &self.imp().folder_list;

        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        // Parent directory row
        if current_path != "/" && !current_path.is_empty() {
            let parent = std::path::Path::new(current_path)
                .parent()
                .map(|p| {
                    let s = p.to_string_lossy().to_string();
                    if s.is_empty() { "/".to_string() } else { s }
                })
                .unwrap_or_else(|| "/".to_string());

            let row = adw::ActionRow::builder()
                .title("..")
                .subtitle("Parent directory")
                .activatable(true)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name("go-up-symbolic"));

            let page = self.clone();
            row.connect_activated(move |_| {
                page.load_directory(&parent);
            });
            list.append(&row);
        }

        let mut dirs: Vec<_> = items.into_iter().filter(|i| i.is_dir).collect();
        dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        for item in dirs {
            let child_path = if current_path == "/" || current_path.is_empty() {
                format!("/{}", item.name)
            } else {
                format!("{}/{}", current_path.trim_end_matches('/'), item.name)
            };

            let row = adw::ActionRow::builder()
                .title(&item.name)
                .activatable(true)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));

            let page = self.clone();
            row.connect_activated(move |_| {
                page.load_directory(&child_path);
            });
            list.append(&row);
        }
    }

    fn show_new_folder_dialog(&self) {
        let dialog = adw::AlertDialog::builder()
            .heading("New Folder")
            .body("Enter a name for the new folder")
            .build();
        dialog.add_responses(&[("cancel", "Cancel"), ("create", "Create")]);
        dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("create"));

        let entry = gtk::Entry::builder()
            .placeholder_text("Folder name")
            .build();
        dialog.set_extra_child(Some(&entry));

        let page = self.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "create" {
                let name = entry.text().trim().to_string();
                if name.is_empty() {
                    return;
                }

                let current = page.imp().current_path.borrow().clone();
                let new_path = if current == "/" || current.is_empty() {
                    format!("/{}", name)
                } else {
                    format!("{}/{}", current.trim_end_matches('/'), name)
                };

                let client = match page.make_client() {
                    Some(c) => c,
                    None => return,
                };

                let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
                std::thread::spawn(move || {
                    let result = client.create_directory(&new_path).map_err(|e| e.to_string());
                    let _ = tx.send(result);
                });

                let page_clone = page.clone();
                let current_clone = current.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    match rx.try_recv() {
                        Ok(Ok(())) => {
                            page_clone.load_directory(&current_clone);
                            glib::ControlFlow::Break
                        }
                        Ok(Err(e)) => {
                            page_clone.window().show_toast(&format!("Error: {}", e));
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
