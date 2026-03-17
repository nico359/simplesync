use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::db::Database;
use crate::targets_page::SimplesyncTargetsPage;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/nico359/simplesync/window.ui")]
    pub struct SimplesyncWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub navigation_view: TemplateChild<adw::NavigationView>,

        pub db: std::cell::OnceCell<Database>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SimplesyncWindow {
        const NAME: &'static str = "SimplesyncWindow";
        type Type = super::SimplesyncWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SimplesyncWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let window = self.obj();
            window.init_db();
            window.setup_pages();
        }
    }

    impl WidgetImpl for SimplesyncWindow {}
    impl WindowImpl for SimplesyncWindow {}
    impl ApplicationWindowImpl for SimplesyncWindow {}
    impl AdwApplicationWindowImpl for SimplesyncWindow {}
}

glib::wrapper! {
    pub struct SimplesyncWindow(ObjectSubclass<imp::SimplesyncWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl SimplesyncWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }

    fn init_db(&self) {
        let db = Database::new();
        db.init();
        self.imp().db.set(db).expect("Database already initialized");
    }

    pub(crate) fn db(&self) -> &Database {
        self.imp().db.get().expect("Database not initialized")
    }

    pub(crate) fn navigation_view(&self) -> &adw::NavigationView {
        &self.imp().navigation_view
    }

    pub(crate) fn show_toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        self.imp().toast_overlay.add_toast(toast);
    }

    fn setup_pages(&self) {
        let targets_page = SimplesyncTargetsPage::new();
        targets_page.set_window(self);
        self.imp().navigation_view.push(&targets_page);
    }
}
