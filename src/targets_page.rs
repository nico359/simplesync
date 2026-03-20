use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::db::Target;
use crate::keyring;
use crate::push::{self, PushProgress};
use crate::pull::{self, PullProgress};
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
        #[template_child]
        pub setup_account_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub pull_all_button: TemplateChild<gtk::Button>,

        pub window: RefCell<Option<SimplesyncWindow>>,
        pub busy: Cell<bool>,
        pub active_cancel: RefCell<Option<Arc<AtomicBool>>>,
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
        self.imp().setup_account_button.connect_clicked(move |_| {
            let app = page.window().application().unwrap();
            app.activate_action("account", None);
        });

        // Refresh when page becomes visible again (e.g. after returning from account page)
        let page = self.clone();
        self.connect_map(move |_| {
            page.refresh_targets();
        });

        let page = self.clone();
        self.imp().push_all_button.connect_clicked(move |_| {
            if page.imp().busy.get() {
                if let Some(flag) = page.imp().active_cancel.borrow().as_ref() {
                    flag.store(true, Ordering::Relaxed);
                }
            } else {
                page.push_all();
            }
        });

        let page = self.clone();
        self.imp().pull_all_button.connect_clicked(move |_| {
            if page.imp().busy.get() {
                if let Some(flag) = page.imp().active_cancel.borrow().as_ref() {
                    flag.store(true, Ordering::Relaxed);
                }
            } else {
                page.pull_all();
            }
        });
    }

    fn set_busy(&self, busy: bool) {
        self.imp().busy.set(busy);
        if busy {
            self.imp().push_all_button.set_icon_name("process-stop-symbolic");
            self.imp().push_all_button.set_tooltip_text(Some("Cancel"));
            self.imp().push_all_button.add_css_class("destructive-action");
            self.imp().pull_all_button.set_icon_name("process-stop-symbolic");
            self.imp().pull_all_button.set_tooltip_text(Some("Cancel"));
            self.imp().pull_all_button.add_css_class("destructive-action");
        } else {
            self.imp().push_all_button.set_icon_name("go-up-symbolic");
            self.imp().push_all_button.set_tooltip_text(Some("Push All"));
            self.imp().push_all_button.remove_css_class("destructive-action");
            self.imp().pull_all_button.set_icon_name("go-down-symbolic");
            self.imp().pull_all_button.set_tooltip_text(Some("Pull All"));
            self.imp().pull_all_button.remove_css_class("destructive-action");
            *self.imp().active_cancel.borrow_mut() = None;
        }
    }

    pub fn refresh_targets(&self) {
        let list = &self.imp().targets_list;

        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        // Check if account is configured
        let has_account = keyring::load_credentials_sync().is_some();

        if !has_account {
            self.imp().content_stack.set_visible_child_name("no_account");
            return;
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

        let original_subtitle = format!("{} → {}", target.local_path, target.remote_path);

        let row = adw::ActionRow::builder()
            .title(&local_name)
            .subtitle(&original_subtitle)
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

        let push_button = gtk::Button::builder()
            .icon_name("go-up-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text("Push")
            .build();
        push_button.add_css_class("flat");

        let pull_button = gtk::Button::builder()
            .icon_name("go-down-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text("Pull")
            .build();
        pull_button.add_css_class("flat");

        let cancel_button = gtk::Button::builder()
            .icon_name("process-stop-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text("Cancel")
            .visible(false)
            .build();
        cancel_button.add_css_class("flat");

        row.add_suffix(&mode_label);
        row.add_suffix(&push_button);
        row.add_suffix(&pull_button);
        row.add_suffix(&cancel_button);

        // Shared cancel flag for this row
        let cancel_flag: Rc<RefCell<Option<Arc<AtomicBool>>>> = Rc::new(RefCell::new(None));

        // --- Push button ---
        {
            let page = self.clone();
            let cancel_flag = cancel_flag.clone();
            let push_btn = push_button.clone();
            let pull_btn = pull_button.clone();
            let cancel_btn = cancel_button.clone();
            let row_ref = row.clone();
            let orig_sub = original_subtitle.clone();
            let target_id = target.id;

            push_button.connect_clicked(move |_| {
                if page.imp().busy.get() {
                    page.window().show_toast("An operation is already in progress");
                    return;
                }

                let creds = match keyring::load_credentials_sync() {
                    Some(c) => c,
                    None => {
                        page.window().show_toast("No account configured. Set up an account first.");
                        return;
                    }
                };

                let target = match page.window().db().get_target(target_id) {
                    Ok(t) => t,
                    Err(_) => {
                        page.window().show_toast("Target not found");
                        return;
                    }
                };

                // Scan first
                row_ref.set_subtitle("Scanning…");
                push_btn.set_sensitive(false);
                pull_btn.set_sensitive(false);

                let client = WebDAVClient::new(&creds.server_url, &creds.username, &creds.app_password);
                let db_path = crate::db::Database::db_path();

                let target_clone = target.clone();
                let db_path_clone = db_path.clone();
                let (plan_tx, plan_rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let result = push::plan_push(&client, &target_clone, &db_path_clone);
                    let _ = plan_tx.send(result);
                });

                let page = page.clone();
                let push_btn = push_btn.clone();
                let pull_btn = pull_btn.clone();
                let cancel_btn = cancel_btn.clone();
                let cancel_flag = cancel_flag.clone();
                let row_ref = row_ref.clone();
                let orig_sub = orig_sub.clone();
                let target = Rc::new(target);

                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    match plan_rx.try_recv() {
                        Ok(Ok(plan)) => {
                            row_ref.set_subtitle(&orig_sub);
                            push_btn.set_sensitive(true);
                            pull_btn.set_sensitive(true);

                            if plan.to_upload == 0 {
                                page.window().show_toast("Nothing to push — all files up to date");
                                return glib::ControlFlow::Break;
                            }

                            let mut body = format!(
                                "{} file(s) to upload, {} to skip",
                                plan.to_upload, plan.to_skip
                            );
                            if plan.is_mirror {
                                body.push_str("\n\nMirror mode: remote files not in local will be deleted");
                            }

                            let dialog = adw::AlertDialog::builder()
                                .heading("Push")
                                .body(&body)
                                .build();
                            dialog.add_responses(&[("cancel", "Cancel"), ("push", "Push")]);
                            dialog.set_response_appearance("push", adw::ResponseAppearance::Suggested);
                            dialog.set_default_response(Some("cancel"));

                            let page_for_dialog = page.clone();
                            let push_btn = push_btn.clone();
                            let pull_btn = pull_btn.clone();
                            let cancel_btn = cancel_btn.clone();
                            let cancel_flag = cancel_flag.clone();
                            let row_ref = row_ref.clone();
                            let orig_sub = orig_sub.clone();
                            let creds = creds.clone();
                            let target = target.clone();

                            dialog.connect_response(None, move |_, response| {
                                if response != "push" {
                                    return;
                                }

                                let flag = Arc::new(AtomicBool::new(false));
                                *cancel_flag.borrow_mut() = Some(flag.clone());
                                *page_for_dialog.imp().active_cancel.borrow_mut() = Some(flag.clone());
                                page_for_dialog.set_busy(true);

                                push_btn.set_visible(false);
                                pull_btn.set_visible(false);
                                cancel_btn.set_visible(true);
                                row_ref.set_subtitle("Preparing push…");

                                let client = WebDAVClient::new(&creds.server_url, &creds.username, &creds.app_password);
                                let db_path = crate::db::Database::db_path();

                                let (tx, rx) = std::sync::mpsc::channel();
                                push::run_push(client, (*target).clone(), db_path, false, flag, tx);

                                let page = page_for_dialog.clone();
                                let push_btn = push_btn.clone();
                                let pull_btn = pull_btn.clone();
                                let cancel_btn = cancel_btn.clone();
                                let row_ref = row_ref.clone();
                                let orig_sub = orig_sub.clone();
                                let cancel_flag = cancel_flag.clone();

                                glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                                    while let Ok(progress) = rx.try_recv() {
                                        match progress {
                                            PushProgress::File { current_file, files_done, files_total } => {
                                                let name = std::path::Path::new(&current_file)
                                                    .file_name()
                                                    .map(|n| n.to_string_lossy().to_string())
                                                    .unwrap_or(current_file);
                                                row_ref.set_subtitle(&format!(
                                                    "Pushing {}/{}:  {}", files_done + 1, files_total, name
                                                ));
                                            }
                                            PushProgress::Complete { summary, .. } => {
                                                push_btn.set_visible(true);
                                                pull_btn.set_visible(true);
                                                cancel_btn.set_visible(false);
                                                row_ref.set_subtitle(&orig_sub);
                                                *cancel_flag.borrow_mut() = None;
                                                page.set_busy(false);

                                                let msg = if summary.cancelled {
                                                    format!("Push cancelled ({} uploaded)", summary.uploaded)
                                                } else if summary.errors.is_empty() {
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
                            });

                            dialog.present(Some(&page.window()));
                            glib::ControlFlow::Break
                        }
                        Ok(Err(e)) => {
                            row_ref.set_subtitle(&orig_sub);
                            push_btn.set_sensitive(true);
                            pull_btn.set_sensitive(true);
                            page.window().show_toast(&format!("Scan failed: {}", e));
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => glib::ControlFlow::Break,
                    }
                });
            });
        }

        // --- Pull button ---
        {
            let page = self.clone();
            let cancel_flag = cancel_flag.clone();
            let push_btn = push_button.clone();
            let pull_btn = pull_button.clone();
            let cancel_btn = cancel_button.clone();
            let row_ref = row.clone();
            let orig_sub = original_subtitle.clone();
            let target_id = target.id;

            pull_button.connect_clicked(move |_| {
                if page.imp().busy.get() {
                    page.window().show_toast("An operation is already in progress");
                    return;
                }

                let creds = match keyring::load_credentials_sync() {
                    Some(c) => c,
                    None => {
                        page.window().show_toast("No account configured. Set up an account first.");
                        return;
                    }
                };

                let target = match page.window().db().get_target(target_id) {
                    Ok(t) => t,
                    Err(_) => {
                        page.window().show_toast("Target not found");
                        return;
                    }
                };

                // Scan first
                row_ref.set_subtitle("Scanning…");
                push_btn.set_sensitive(false);
                pull_btn.set_sensitive(false);

                let client = WebDAVClient::new(&creds.server_url, &creds.username, &creds.app_password);

                let target_clone = target.clone();
                let (plan_tx, plan_rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let result = pull::plan_pull(&client, &target_clone);
                    let _ = plan_tx.send(result);
                });

                let page = page.clone();
                let push_btn = push_btn.clone();
                let pull_btn = pull_btn.clone();
                let cancel_btn = cancel_btn.clone();
                let cancel_flag = cancel_flag.clone();
                let row_ref = row_ref.clone();
                let orig_sub = orig_sub.clone();
                let target = Rc::new(target);

                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    match plan_rx.try_recv() {
                        Ok(Ok(plan)) => {
                            row_ref.set_subtitle(&orig_sub);
                            push_btn.set_sensitive(true);
                            pull_btn.set_sensitive(true);

                            if plan.to_download == 0 {
                                page.window().show_toast("Nothing to pull — all files up to date");
                                return glib::ControlFlow::Break;
                            }

                            let body = format!(
                                "{} file(s) to download, {} to skip",
                                plan.to_download, plan.to_skip
                            );

                            let dialog = adw::AlertDialog::builder()
                                .heading("Pull")
                                .body(&body)
                                .build();
                            dialog.add_responses(&[("cancel", "Cancel"), ("pull", "Pull")]);
                            dialog.set_response_appearance("pull", adw::ResponseAppearance::Suggested);
                            dialog.set_default_response(Some("cancel"));

                            let page_for_dialog = page.clone();
                            let push_btn = push_btn.clone();
                            let pull_btn = pull_btn.clone();
                            let cancel_btn = cancel_btn.clone();
                            let cancel_flag = cancel_flag.clone();
                            let row_ref = row_ref.clone();
                            let orig_sub = orig_sub.clone();
                            let creds = creds.clone();
                            let target = target.clone();

                            dialog.connect_response(None, move |_, response| {
                                if response != "pull" {
                                    return;
                                }

                                let flag = Arc::new(AtomicBool::new(false));
                                *cancel_flag.borrow_mut() = Some(flag.clone());
                                *page_for_dialog.imp().active_cancel.borrow_mut() = Some(flag.clone());
                                page_for_dialog.set_busy(true);

                                push_btn.set_visible(false);
                                pull_btn.set_visible(false);
                                cancel_btn.set_visible(true);
                                row_ref.set_subtitle("Preparing pull…");

                                let client = WebDAVClient::new(&creds.server_url, &creds.username, &creds.app_password);

                                let (tx, rx) = std::sync::mpsc::channel();
                                pull::run_pull(client, (*target).clone(), flag, tx);

                                let page = page_for_dialog.clone();
                                let push_btn = push_btn.clone();
                                let pull_btn = pull_btn.clone();
                                let cancel_btn = cancel_btn.clone();
                                let row_ref = row_ref.clone();
                                let orig_sub = orig_sub.clone();
                                let cancel_flag = cancel_flag.clone();

                                glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                                    while let Ok(progress) = rx.try_recv() {
                                        match progress {
                                            PullProgress::File { current_file, files_done, files_total } => {
                                                let name = std::path::Path::new(&current_file)
                                                    .file_name()
                                                    .map(|n| n.to_string_lossy().to_string())
                                                    .unwrap_or(current_file);
                                                row_ref.set_subtitle(&format!(
                                                    "Pulling {}/{}:  {}", files_done + 1, files_total, name
                                                ));
                                            }
                                            PullProgress::Complete { summary, .. } => {
                                                push_btn.set_visible(true);
                                                pull_btn.set_visible(true);
                                                cancel_btn.set_visible(false);
                                                row_ref.set_subtitle(&orig_sub);
                                                *cancel_flag.borrow_mut() = None;
                                                page.set_busy(false);

                                                let msg = if summary.cancelled {
                                                    format!("Pull cancelled ({} downloaded)", summary.downloaded)
                                                } else if summary.errors.is_empty() {
                                                    format!("Done: {} downloaded, {} skipped", summary.downloaded, summary.skipped)
                                                } else {
                                                    format!("Pull completed with {} error(s)", summary.errors.len())
                                                };
                                                page.window().show_toast(&msg);
                                                return glib::ControlFlow::Break;
                                            }
                                        }
                                    }
                                    glib::ControlFlow::Continue
                                });
                            });

                            dialog.present(Some(&page.window()));
                            glib::ControlFlow::Break
                        }
                        Ok(Err(e)) => {
                            row_ref.set_subtitle(&orig_sub);
                            push_btn.set_sensitive(true);
                            pull_btn.set_sensitive(true);
                            page.window().show_toast(&format!("Scan failed: {}", e));
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => glib::ControlFlow::Break,
                    }
                });
            });
        }

        // --- Cancel button ---
        {
            let cancel_flag = cancel_flag.clone();
            cancel_button.connect_clicked(move |_| {
                if let Some(flag) = cancel_flag.borrow().as_ref() {
                    flag.store(true, Ordering::Relaxed);
                }
            });
        }

        // --- Row activation (edit) ---
        {
            let target_id = target.id;
            let page = self.clone();
            row.connect_activated(move |_| {
                page.show_edit_target(target_id);
            });
        }

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

    fn push_all(&self) {
        if self.imp().busy.get() {
            self.window().show_toast("An operation is already in progress");
            return;
        }

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

        // Scan all targets first
        let db_path = crate::db::Database::db_path();
        let targets_clone: Vec<_> = targets.clone();
        let creds_clone = creds.clone();
        let db_path_clone = db_path.clone();
        let (plan_tx, plan_rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let mut total_upload = 0u32;
            let mut total_skip = 0u32;
            let mut any_mirror = false;
            for target in &targets_clone {
                let client = WebDAVClient::new(&creds_clone.server_url, &creds_clone.username, &creds_clone.app_password);
                match push::plan_push(&client, target, &db_path_clone) {
                    Ok(plan) => {
                        total_upload += plan.to_upload;
                        total_skip += plan.to_skip;
                        if plan.is_mirror { any_mirror = true; }
                    }
                    Err(_) => {}
                }
            }
            let _ = plan_tx.send((total_upload, total_skip, any_mirror));
        });

        let page = self.clone();
        let target_ids: Vec<i64> = targets.iter().map(|t| t.id).collect();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            match plan_rx.try_recv() {
                Ok((total_upload, total_skip, any_mirror)) => {
                    if total_upload == 0 {
                        page.window().show_toast("Nothing to push — all files up to date");
                        return glib::ControlFlow::Break;
                    }

                    let mut body = format!(
                        "{} file(s) to upload, {} to skip across {} target(s)",
                        total_upload, total_skip, target_ids.len()
                    );
                    if any_mirror {
                        body.push_str("\n\nMirror mode: remote files not in local will be deleted");
                    }

                    let dialog = adw::AlertDialog::builder()
                        .heading("Push All")
                        .body(&body)
                        .build();
                    dialog.add_responses(&[("cancel", "Cancel"), ("push", "Push All")]);
                    dialog.set_response_appearance("push", adw::ResponseAppearance::Suggested);
                    dialog.set_default_response(Some("cancel"));

                    let page_for_dialog = page.clone();
                    let creds = creds.clone();
                    let target_ids = target_ids.clone();
                    dialog.connect_response(None, move |_, response| {
                        if response != "push" {
                            return;
                        }
                        let cancel = Arc::new(AtomicBool::new(false));
                        *page_for_dialog.imp().active_cancel.borrow_mut() = Some(cancel.clone());
                        page_for_dialog.set_busy(true);
                        page_for_dialog.push_sequential(target_ids.clone(), 0, creds.clone(), cancel);
                    });

                    dialog.present(Some(&page.window()));
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => {
                    page.window().show_toast("Failed to scan targets");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn push_sequential(&self, target_ids: Vec<i64>, index: usize, creds: keyring::Credentials, cancel: Arc<AtomicBool>) {
        if cancel.load(Ordering::Relaxed) || index >= target_ids.len() {
            self.set_busy(false);
            if cancel.load(Ordering::Relaxed) {
                self.window().show_toast("Push all cancelled");
            } else {
                self.window().show_toast("All targets pushed");
            }
            self.refresh_targets();
            return;
        }

        let window = self.window();
        let target = match window.db().get_target(target_ids[index]) {
            Ok(t) => t,
            Err(_) => {
                self.push_sequential(target_ids, index + 1, creds, cancel);
                return;
            }
        };

        let client = WebDAVClient::new(&creds.server_url, &creds.username, &creds.app_password);
        let db_path = crate::db::Database::db_path();

        let (tx, rx) = std::sync::mpsc::channel();
        push::run_push(client, target, db_path, false, cancel.clone(), tx);

        let page = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            while let Ok(progress) = rx.try_recv() {
                if let PushProgress::Complete { .. } = progress {
                    page.push_sequential(target_ids.clone(), index + 1, creds.clone(), cancel.clone());
                    return glib::ControlFlow::Break;
                }
            }
            glib::ControlFlow::Continue
        });
    }

    fn pull_all(&self) {
        if self.imp().busy.get() {
            self.window().show_toast("An operation is already in progress");
            return;
        }

        let window = self.window();
        let targets = window.db().get_targets().unwrap_or_default();
        if targets.is_empty() {
            self.window().show_toast("No targets to pull");
            return;
        }

        let creds = match keyring::load_credentials_sync() {
            Some(c) => c,
            None => {
                self.window().show_toast("No account configured");
                return;
            }
        };

        // Scan all targets first
        let targets_clone: Vec<_> = targets.clone();
        let creds_clone = creds.clone();
        let (plan_tx, plan_rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let mut total_download = 0u32;
            let mut total_skip = 0u32;
            for target in &targets_clone {
                let client = WebDAVClient::new(&creds_clone.server_url, &creds_clone.username, &creds_clone.app_password);
                match pull::plan_pull(&client, target) {
                    Ok(plan) => {
                        total_download += plan.to_download;
                        total_skip += plan.to_skip;
                    }
                    Err(_) => {}
                }
            }
            let _ = plan_tx.send((total_download, total_skip));
        });

        let page = self.clone();
        let target_ids: Vec<i64> = targets.iter().map(|t| t.id).collect();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            match plan_rx.try_recv() {
                Ok((total_download, total_skip)) => {
                    if total_download == 0 {
                        page.window().show_toast("Nothing to pull — all files up to date");
                        return glib::ControlFlow::Break;
                    }

                    let body = format!(
                        "{} file(s) to download, {} to skip across {} target(s)",
                        total_download, total_skip, target_ids.len()
                    );

                    let dialog = adw::AlertDialog::builder()
                        .heading("Pull All")
                        .body(&body)
                        .build();
                    dialog.add_responses(&[("cancel", "Cancel"), ("pull", "Pull All")]);
                    dialog.set_response_appearance("pull", adw::ResponseAppearance::Suggested);
                    dialog.set_default_response(Some("cancel"));

                    let page_for_dialog = page.clone();
                    let creds = creds.clone();
                    let target_ids = target_ids.clone();
                    dialog.connect_response(None, move |_, response| {
                        if response != "pull" {
                            return;
                        }
                        let cancel = Arc::new(AtomicBool::new(false));
                        *page_for_dialog.imp().active_cancel.borrow_mut() = Some(cancel.clone());
                        page_for_dialog.set_busy(true);
                        page_for_dialog.pull_sequential(target_ids.clone(), 0, creds.clone(), cancel);
                    });

                    dialog.present(Some(&page.window()));
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => {
                    page.window().show_toast("Failed to scan targets");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn pull_sequential(&self, target_ids: Vec<i64>, index: usize, creds: keyring::Credentials, cancel: Arc<AtomicBool>) {
        if cancel.load(Ordering::Relaxed) || index >= target_ids.len() {
            self.set_busy(false);
            if cancel.load(Ordering::Relaxed) {
                self.window().show_toast("Pull all cancelled");
            } else {
                self.window().show_toast("All targets pulled");
            }
            self.refresh_targets();
            return;
        }

        let window = self.window();
        let target = match window.db().get_target(target_ids[index]) {
            Ok(t) => t,
            Err(_) => {
                self.pull_sequential(target_ids, index + 1, creds, cancel);
                return;
            }
        };

        let client = WebDAVClient::new(&creds.server_url, &creds.username, &creds.app_password);

        let (tx, rx) = std::sync::mpsc::channel();
        pull::run_pull(client, target, cancel.clone(), tx);

        let page = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            while let Ok(progress) = rx.try_recv() {
                if let PullProgress::Complete { .. } = progress {
                    page.pull_sequential(target_ids.clone(), index + 1, creds.clone(), cancel.clone());
                    return glib::ControlFlow::Break;
                }
            }
            glib::ControlFlow::Continue
        });
    }
}
