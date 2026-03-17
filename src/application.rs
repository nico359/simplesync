/* application.rs
 *
 * Copyright 2026 nico359
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use gettextrs::gettext;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::config::VERSION;
use crate::window::SimplesyncWindow;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct SimplesyncApplication {}

    #[glib::object_subclass]
    impl ObjectSubclass for SimplesyncApplication {
        const NAME: &'static str = "SimplesyncApplication";
        type Type = super::SimplesyncApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for SimplesyncApplication {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_gactions();
            obj.set_accels_for_action("app.quit", &["<control>q"]);
        }
    }

    impl ApplicationImpl for SimplesyncApplication {
        fn activate(&self) {
            let application = self.obj();
            let window = application.active_window().unwrap_or_else(|| {
                let window = SimplesyncWindow::new(&*application);
                window.upcast()
            });
            window.present();
        }
    }

    impl GtkApplicationImpl for SimplesyncApplication {}
    impl AdwApplicationImpl for SimplesyncApplication {}
}

glib::wrapper! {
    pub struct SimplesyncApplication(ObjectSubclass<imp::SimplesyncApplication>)
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl SimplesyncApplication {
    pub fn new(application_id: &str, flags: &gio::ApplicationFlags) -> Self {
        glib::Object::builder()
            .property("application-id", application_id)
            .property("flags", flags)
            .property("resource-base-path", "/io/github/nico359/simplesync")
            .build()
    }

    fn setup_gactions(&self) {
        let quit_action = gio::ActionEntry::builder("quit")
            .activate(move |app: &Self, _, _| app.quit())
            .build();
        let about_action = gio::ActionEntry::builder("about")
            .activate(move |app: &Self, _, _| app.show_about())
            .build();
        let account_action = gio::ActionEntry::builder("account")
            .activate(move |app: &Self, _, _| app.show_account())
            .build();
        self.add_action_entries([quit_action, about_action, account_action]);
    }

    fn show_account(&self) {
        let window = self.active_window().unwrap();
        let window: SimplesyncWindow = window.downcast().unwrap();
        let account_page = crate::account_page::SimplesyncAccountPage::new();
        account_page.set_window(&window);
        window.navigation_view().push(&account_page);
    }

    fn show_about(&self) {
        let window = self.active_window().unwrap();
        let about = adw::AboutDialog::builder()
            .application_name("SimpleSync")
            .application_icon("io.github.nico359.simplesync")
            .developer_name("nico359")
            .version(VERSION)
            .developers(vec!["nico359", "GitHub Copilot CLI (Claude Haiku 4.5, Claude Opus 4.6)"])
            .translator_credits(&gettext("translator-credits"))
            .copyright("© 2026 nico359")
            .license_type(gtk::License::Gpl30)
            .comments("A simple file sync tool for Nextcloud and WebDAV servers.\n\nBuilt with the assistance of AI (GitHub Copilot CLI, powered by Claude Haiku 4.5 and Claude Opus 4.6).")
            .website("https://github.com/nico359/cloudsend")
            .issue_url("https://github.com/nico359/cloudsend/issues")
            .build();

        about.present(Some(&window));
    }
}
