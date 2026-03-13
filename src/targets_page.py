# targets_page.py
#
# Copyright 2026 Unknown
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# SPDX-License-Identifier: GPL-3.0-or-later

import os

from gi.repository import Adw, Gtk, GLib, Gio
from .db import get_db
from . import keyring
from .webdav import WebDAVClient
from .push import PushEngine


@Gtk.Template(resource_path='/io/github/nico359/cloudsend/targets_page.ui')
class CloudsendTargetsPage(Adw.NavigationPage):
    __gtype_name__ = 'CloudsendTargetsPage'

    content_stack = Gtk.Template.Child()
    targets_listbox = Gtk.Template.Child()
    add_button = Gtk.Template.Child()
    push_all_button = Gtk.Template.Child()

    def __init__(self, **kwargs):
        super().__init__(**kwargs)
        self.add_button.connect('clicked', self._on_add_clicked)
        self.push_all_button.connect('clicked', self._on_push_all_clicked)
        self._push_engine = None
        self._push_all_queue = []
        self._push_all_running = False

    def refresh(self):
        """Reload targets from DB and update the list."""
        while True:
            row = self.targets_listbox.get_row_at_index(0)
            if row is None:
                break
            self.targets_listbox.remove(row)

        targets = get_db().get_targets()
        if not targets:
            self.content_stack.set_visible_child_name('empty')
        else:
            self.content_stack.set_visible_child_name('list')
            for target in targets:
                row = self._create_target_row(target)
                self.targets_listbox.append(row)

    def _create_target_row(self, target):
        """Create an AdwActionRow for a target."""
        row = Adw.ActionRow()
        local_name = target['local_path'].rstrip('/').rsplit('/', 1)[-1]
        row.set_title(local_name)
        row.set_subtitle(f"{target['local_path']} → {target['remote_path']}")

        # Mode badge
        mode_label = Gtk.Label(label=target['mode'].capitalize())
        mode_label.add_css_class('dim-label')
        mode_label.add_css_class('caption')
        mode_label.set_valign(Gtk.Align.CENTER)
        row.add_suffix(mode_label)

        # Push button
        push_btn = Gtk.Button(icon_name='send-to-symbolic')
        push_btn.set_valign(Gtk.Align.CENTER)
        push_btn.add_css_class('flat')
        push_btn.set_tooltip_text('Push now')
        push_btn.connect('clicked', self._on_push_clicked, target)
        row.add_suffix(push_btn)

        # Tap row to edit
        row.set_activatable(True)
        row.connect('activated', self._on_row_activated, target)

        return row

    def _on_add_clicked(self, button):
        """Navigate to the target edit page for a new target."""
        from .target_edit_page import CloudsendTargetEditPage
        page = CloudsendTargetEditPage(targets_page=self)
        nav = self.get_ancestor(Adw.NavigationView)
        nav.push(page)

    def _on_row_activated(self, row, target):
        """Navigate to the target edit page for an existing target."""
        from .target_edit_page import CloudsendTargetEditPage
        page = CloudsendTargetEditPage(target=target, targets_page=self)
        nav = self.get_ancestor(Adw.NavigationView)
        nav.push(page)

    def _show_error_details(self, errors):
        """Show an AlertDialog listing push errors."""
        body = "\n".join(f"• {e}" for e in errors)
        dialog = Adw.AlertDialog(
            heading="Push Errors",
            body=body,
        )
        dialog.add_response("close", "Close")
        dialog.set_default_response("close")
        dialog.present(self.get_ancestor(Gtk.Window))

    def _on_push_clicked(self, button, target, done_callback=None):
        """Push a single target."""
        if not keyring.has_credentials():
            toast = Adw.Toast(title="Please set up your account first")
            win = self.get_ancestor(Adw.ApplicationWindow)
            if hasattr(win, '_toast_overlay'):
                win._toast_overlay.add_toast(toast)
            if done_callback:
                done_callback()
            return

        local_path = target['local_path']
        if not os.path.exists(local_path):
            toast = Adw.Toast(
                title=f"Local folder not found: {local_path}",
            )
            win = self.get_ancestor(Adw.ApplicationWindow)
            if hasattr(win, '_toast_overlay'):
                win._toast_overlay.add_toast(toast)
            if done_callback:
                done_callback()
            return

        creds = keyring.load_credentials()
        client = WebDAVClient(
            creds['server_url'], creds['username'], creds['app_password'],
        )

        # Find the row for this target to update its subtitle
        row = button.get_ancestor(Adw.ActionRow)
        original_subtitle = row.get_subtitle() if row else ''

        button.set_sensitive(False)
        spinner = Gtk.Spinner(spinning=True)
        button.set_child(spinner)

        def on_progress(current_file, files_done, files_total):
            if row:
                row.set_subtitle(f"Uploading {files_done}/{files_total}: {current_file}")
            return GLib.SOURCE_REMOVE

        def on_complete(success, summary):
            button.set_sensitive(True)
            button.set_child(None)
            button.set_icon_name('send-to-symbolic')
            if row:
                row.set_subtitle(original_subtitle)

            if success:
                msg = f"Uploaded {summary['uploaded']}, skipped {summary['skipped']}"
                toast = Adw.Toast(title=msg)
            else:
                errors = summary.get('errors', [])
                count = len(errors)
                msg = f"Push completed with {count} error(s)"
                toast = Adw.Toast(title=msg)
                if errors:
                    toast.set_button_label("View Details")
                    toast.connect(
                        'button-clicked',
                        lambda _t: self._show_error_details(errors),
                    )

            win = self.get_ancestor(Adw.ApplicationWindow)
            if hasattr(win, '_toast_overlay'):
                win._toast_overlay.add_toast(toast)

            self.refresh()

            if done_callback:
                done_callback()

            return GLib.SOURCE_REMOVE

        engine = PushEngine(client, target, on_progress=on_progress, on_complete=on_complete)
        engine.start()

    def _on_push_all_clicked(self, button):
        """Push all targets sequentially."""
        targets = get_db().get_targets()
        if not targets:
            return

        self._push_all_queue = list(targets)
        self.push_all_button.set_sensitive(False)

        def _push_next():
            if not self._push_all_queue:
                self.push_all_button.set_sensitive(True)
                return
            target = self._push_all_queue.pop(0)
            # Find the push button for this target row
            idx = 0
            while True:
                row = self.targets_listbox.get_row_at_index(idx)
                if row is None:
                    break
                # Match by looking for the target's path in the subtitle
                child = row.get_child() if hasattr(row, 'get_child') else row
                if isinstance(child, Adw.ActionRow) and target['local_path'] in (child.get_subtitle() or ''):
                    # Find the push button among suffixes — it's the last button
                    # We'll just trigger push via the method directly
                    break
                idx += 1

            # Create a temporary client and push directly
            self._on_push_target_direct(target, _push_next)

        _push_next()

    def _on_push_target_direct(self, target, done_callback):
        """Push a target directly without needing a button reference."""
        if not keyring.has_credentials():
            if done_callback:
                done_callback()
            return

        if not os.path.exists(target['local_path']):
            if done_callback:
                done_callback()
            return

        creds = keyring.load_credentials()
        client = WebDAVClient(
            creds['server_url'], creds['username'], creds['app_password'],
        )

        def on_complete(success, summary):
            if not success:
                errors = summary.get('errors', [])
                if errors:
                    msg = f"{target['local_path']}: {len(errors)} error(s)"
                    toast = Adw.Toast(title=msg)
                    win = self.get_ancestor(Adw.ApplicationWindow)
                    if hasattr(win, '_toast_overlay'):
                        win._toast_overlay.add_toast(toast)

            if done_callback:
                done_callback()
            return GLib.SOURCE_REMOVE

        engine = PushEngine(client, target, on_complete=on_complete)
        engine.start()
