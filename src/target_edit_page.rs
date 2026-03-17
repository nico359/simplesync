use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;

use crate::db::Target;
use crate::keyring;
use crate::remote_browser_page::SimplesyncRemoteBrowserPage;
use crate::window::SimplesyncWindow;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/nico359/simplesync/target_edit_page.ui")]
    pub struct SimplesyncTargetEditPage {
        #[template_child]
        pub local_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub remote_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub mirror_switch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub save_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub delete_button: TemplateChild<gtk::Button>,

        pub window: RefCell<Option<SimplesyncWindow>>,
        pub target_id: RefCell<Option<i64>>,
        pub local_path: RefCell<Option<String>>,
        pub remote_path: RefCell<Option<String>>,
        pub on_save: RefCell<Option<Box<dyn Fn() + 'static>>>,
    }

    impl std::fmt::Debug for SimplesyncTargetEditPage {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SimplesyncTargetEditPage").finish()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SimplesyncTargetEditPage {
        const NAME: &'static str = "SimplesyncTargetEditPage";
        type Type = super::SimplesyncTargetEditPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SimplesyncTargetEditPage {}
    impl WidgetImpl for SimplesyncTargetEditPage {}
    impl NavigationPageImpl for SimplesyncTargetEditPage {}
}

glib::wrapper! {
    pub struct SimplesyncTargetEditPage(ObjectSubclass<imp::SimplesyncTargetEditPage>)
        @extends gtk::Widget, adw::NavigationPage;
}

impl SimplesyncTargetEditPage {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    pub fn set_window<F: Fn() + 'static>(
        &self,
        window: &SimplesyncWindow,
        target: Option<&Target>,
        on_save: F,
    ) {
        self.imp().window.replace(Some(window.clone()));
        self.imp().on_save.replace(Some(Box::new(on_save)));

        if let Some(target) = target {
            self.set_title("Edit Target");
            self.imp().target_id.replace(Some(target.id));
            self.imp().local_path.replace(Some(target.local_path.clone()));
            self.imp().remote_path.replace(Some(target.remote_path.clone()));
            self.imp().local_row.set_subtitle(&target.local_path);
            self.imp().remote_row.set_subtitle(&target.remote_path);
            self.imp().mirror_switch.set_active(target.mode == "mirror");
            self.imp().delete_button.set_visible(true);
        } else {
            self.set_title("Add Target");
        }

        self.setup_signals();
    }

    fn window(&self) -> SimplesyncWindow {
        self.imp().window.borrow().clone().expect("Window not set")
    }

    fn setup_signals(&self) {
        let page = self.clone();
        self.imp().local_row.connect_activated(move |_| {
            page.pick_local_folder();
        });

        let page = self.clone();
        self.imp().remote_row.connect_activated(move |_| {
            page.pick_remote_folder();
        });

        let page = self.clone();
        self.imp().save_button.connect_clicked(move |_| {
            page.save();
        });

        let page = self.clone();
        self.imp().delete_button.connect_clicked(move |_| {
            page.confirm_delete();
        });
    }

    fn pick_local_folder(&self) {
        let dialog = gtk::FileDialog::builder()
            .title("Select Local Folder")
            .build();

        let page = self.clone();
        dialog.select_folder(
            Some(&self.window()),
            None::<&gio::Cancellable>,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        let path_str = path.to_string_lossy().to_string();
                        page.imp().local_row.set_subtitle(&path_str);
                        page.imp().local_path.replace(Some(path_str));
                    }
                }
            },
        );
    }

    fn pick_remote_folder(&self) {
        let creds = match keyring::load_credentials_sync() {
            Some(c) => c,
            None => {
                self.window().show_toast("No account configured");
                return;
            }
        };

        let browser = SimplesyncRemoteBrowserPage::new();
        let page = self.clone();
        browser.set_window(&self.window(), &creds, move |selected_path| {
            page.imp().remote_row.set_subtitle(&selected_path);
            page.imp().remote_path.replace(Some(selected_path));
        });
        self.window().navigation_view().push(&browser);
    }

    fn save(&self) {
        let local = self.imp().local_path.borrow().clone();
        let remote = self.imp().remote_path.borrow().clone();

        let (local, remote) = match (local, remote) {
            (Some(l), Some(r)) => (l, r),
            _ => {
                self.window().show_toast("Please select both local and remote folders");
                return;
            }
        };

        let mode = if self.imp().mirror_switch.is_active() { "mirror" } else { "upload" };

        let window = self.window();
        let db = window.db();
        let result = if let Some(id) = *self.imp().target_id.borrow() {
            db.update_target(id, &local, &remote, mode)
        } else {
            db.add_target(&local, &remote, mode).map(|_| ())
        };

        match result {
            Ok(()) => {
                if let Some(ref callback) = *self.imp().on_save.borrow() {
                    callback();
                }
                self.window().navigation_view().pop();
            }
            Err(e) => {
                self.window().show_toast(&format!("Error saving: {}", e));
            }
        }
    }

    fn confirm_delete(&self) {
        let target_id = match *self.imp().target_id.borrow() {
            Some(id) => id,
            None => return,
        };

        let dialog = adw::AlertDialog::builder()
            .heading("Delete Target?")
            .body("This will remove the sync target and its upload history.")
            .build();
        dialog.add_responses(&[("cancel", "Cancel"), ("delete", "Delete")]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));

        let page = self.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "delete" {
                let window = page.window();
                if let Err(e) = window.db().delete_target(target_id) {
                    page.window().show_toast(&format!("Error: {}", e));
                    return;
                }
                if let Some(ref callback) = *page.imp().on_save.borrow() {
                    callback();
                }
                page.window().navigation_view().pop();
            }
        });

        dialog.present(Some(&self.window()));
    }
}
