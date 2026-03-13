# target_edit_page.py
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

from gi.repository import Adw, Gtk, GLib, Gio
from .db import get_db


@Gtk.Template(resource_path='/io/github/nico359/cloudsend/target_edit_page.ui')
class CloudsendTargetEditPage(Adw.NavigationPage):
    __gtype_name__ = 'CloudsendTargetEditPage'

    local_folder_row = Gtk.Template.Child()
    remote_folder_row = Gtk.Template.Child()
    mirror_switch = Gtk.Template.Child()
    save_button = Gtk.Template.Child()
    delete_button = Gtk.Template.Child()

    def __init__(self, target=None, targets_page=None, **kwargs):
        """
        target: dict from db (None = creating new target)
        targets_page: reference to the targets list page (to refresh after save)
        """
        super().__init__(**kwargs)
        self._target = target
        self._targets_page = targets_page
        self._local_path = None
        self._remote_path = None

        self.local_folder_row.connect('activated', self._on_local_folder_clicked)
        self.remote_folder_row.connect('activated', self._on_remote_folder_clicked)
        self.save_button.connect('clicked', self._on_save_clicked)
        self.delete_button.connect('clicked', self._on_delete_clicked)

        if target:
            self.set_title("Edit Target")
            self._local_path = target['local_path']
            self._remote_path = target['remote_path']
            self.local_folder_row.set_subtitle(target['local_path'])
            self.remote_folder_row.set_subtitle(target['remote_path'])
            self.mirror_switch.set_active(target['mode'] == 'mirror')
            self.delete_button.set_visible(True)

    def _on_local_folder_clicked(self, row):
        dialog = Gtk.FileDialog()
        dialog.set_title("Select Local Folder")

        dialog.select_folder(
            self.get_ancestor(Gtk.Window),
            None,
            self._on_local_folder_selected,
        )

    def _on_local_folder_selected(self, dialog, result):
        try:
            folder = dialog.select_folder_finish(result)
            if folder:
                self._local_path = folder.get_path()
                self.local_folder_row.set_subtitle(self._local_path)
        except GLib.Error:
            pass  # User cancelled

    def _on_remote_folder_clicked(self, row):
        from .remote_browser_page import CloudsendRemoteBrowserPage

        initial_path = self._remote_path or '/'
        page = CloudsendRemoteBrowserPage(
            current_path=initial_path,
            on_selected=self._on_remote_folder_selected,
        )
        nav = self.get_ancestor(Adw.NavigationView)
        nav.push(page)

    def _on_remote_folder_selected(self, remote_path):
        self._remote_path = remote_path
        self.remote_folder_row.set_subtitle(remote_path)

    def _on_save_clicked(self, button):
        if not self._local_path or not self._remote_path:
            toast = Adw.Toast(title="Please select both local and remote folders")
            win = self.get_ancestor(Adw.ApplicationWindow)
            if hasattr(win, '_toast_overlay'):
                win._toast_overlay.add_toast(toast)
            return

        mode = 'mirror' if self.mirror_switch.get_active() else 'upload'
        db = get_db()

        if self._target:
            db.update_target(
                self._target['id'],
                local_path=self._local_path,
                remote_path=self._remote_path,
                mode=mode,
            )
        else:
            db.add_target(self._local_path, self._remote_path, mode)

        if self._targets_page:
            self._targets_page.refresh()

        nav = self.get_ancestor(Adw.NavigationView)
        nav.pop()

    def _on_delete_clicked(self, button):
        if not self._target:
            return

        dialog = Adw.AlertDialog(
            heading="Delete Target?",
            body=(
                f"This will remove the sync target for "
                f"{self._target['local_path']}. "
                f"Files on the server will not be deleted."
            ),
        )
        dialog.add_response("cancel", "Cancel")
        dialog.add_response("delete", "Delete")
        dialog.set_response_appearance(
            "delete", Adw.ResponseAppearance.DESTRUCTIVE
        )
        dialog.set_default_response("cancel")

        def on_response(dlg, response):
            if response == "delete":
                get_db().delete_target(self._target['id'])
                if self._targets_page:
                    self._targets_page.refresh()
                nav = self.get_ancestor(Adw.NavigationView)
                nav.pop()

        dialog.connect('response', on_response)
        dialog.present(self.get_ancestor(Gtk.Window))
