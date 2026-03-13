# remote_browser_page.py
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

from gi.repository import Adw, Gtk, GLib
from . import keyring
from .webdav import WebDAVClient


@Gtk.Template(resource_path='/io/github/nico359/cloudsend/remote_browser_page.ui')
class CloudsendRemoteBrowserPage(Adw.NavigationPage):
    __gtype_name__ = 'CloudsendRemoteBrowserPage'

    browser_stack = Gtk.Template.Child()
    folders_listbox = Gtk.Template.Child()
    select_button = Gtk.Template.Child()
    new_folder_button = Gtk.Template.Child()
    error_status = Gtk.Template.Child()

    def __init__(self, current_path='/', on_selected=None, **kwargs):
        """
        current_path: initial remote path to browse
        on_selected: callback(selected_remote_path) called when user taps Select
        """
        super().__init__(**kwargs)
        self._current_path = current_path.rstrip('/') or '/'
        self._on_selected = on_selected
        self._client = None

        self.select_button.connect('clicked', self._on_select_clicked)
        self.new_folder_button.connect('clicked', self._on_new_folder_clicked)

        self._setup_client()
        self._load_directory()

    def _setup_client(self):
        creds = keyring.load_credentials()
        if creds:
            self._client = WebDAVClient(
                creds['server_url'], creds['username'], creds['app_password']
            )

    def _load_directory(self):
        """Load the current remote path contents."""
        self.set_title(self._current_path or '/')
        self.browser_stack.set_visible_child_name('loading')

        while True:
            row = self.folders_listbox.get_row_at_index(0)
            if row is None:
                break
            self.folders_listbox.remove(row)

        if not self._client:
            self.error_status.set_description("No account configured")
            self.browser_stack.set_visible_child_name('error')
            return

        self._client.list_directory(self._current_path, self._on_list_result)

    def _on_list_result(self, success, result):
        def update():
            if not success:
                self.error_status.set_description(str(result))
                self.browser_stack.set_visible_child_name('error')
                return

            if self._current_path and self._current_path != '/':
                parent_row = Adw.ActionRow()
                parent_row.set_title("📁 ..")
                parent_row.set_subtitle("Go up")
                parent_row.set_activatable(True)
                parent_row.connect('activated', self._on_go_up)
                self.folders_listbox.append(parent_row)

            # Only show directories (this is a folder picker)
            dirs = sorted(
                (item for item in result if item['is_dir']),
                key=lambda d: d['name'].lower(),
            )

            for item in dirs:
                row = Adw.ActionRow()
                row.set_title(f"📁 {item['name']}")
                row.set_activatable(True)
                row.connect('activated', self._on_folder_activated, item['name'])
                self.folders_listbox.append(row)

            self.browser_stack.set_visible_child_name('content')

        GLib.idle_add(update)

    def _on_folder_activated(self, row, folder_name):
        """Navigate into a subfolder."""
        if self._current_path == '/':
            self._current_path = '/' + folder_name
        else:
            self._current_path = self._current_path.rstrip('/') + '/' + folder_name
        self._load_directory()

    def _on_go_up(self, row):
        """Navigate to parent directory."""
        if '/' in self._current_path.rstrip('/'):
            self._current_path = self._current_path.rstrip('/').rsplit('/', 1)[0]
        if not self._current_path:
            self._current_path = '/'
        self._load_directory()

    def _on_select_clicked(self, button):
        """Confirm selection and pop the page."""
        if self._on_selected:
            self._on_selected(self._current_path)
        nav = self.get_ancestor(Adw.NavigationView)
        if nav:
            nav.pop()

    def _on_new_folder_clicked(self, button):
        """Show dialog to create a new folder in the current directory."""
        dialog = Adw.AlertDialog(
            heading="New Folder",
            body="Enter a name for the new folder:",
        )

        entry = Gtk.Entry()
        entry.set_placeholder_text("Folder name")
        dialog.set_extra_child(entry)

        dialog.add_response("cancel", "Cancel")
        dialog.add_response("create", "Create")
        dialog.set_response_appearance("create", Adw.ResponseAppearance.SUGGESTED)
        dialog.set_default_response("create")

        def on_response(dlg, response):
            if response == "create":
                name = entry.get_text().strip()
                if name:
                    path = self._current_path.rstrip('/') + '/' + name
                    self._client.create_directory(
                        path,
                        lambda ok, err: GLib.idle_add(self._on_folder_created, ok, err),
                    )

        dialog.connect('response', on_response)
        dialog.present(self.get_ancestor(Gtk.Window))

    def _on_folder_created(self, success, error):
        """Refresh the listing after creating a folder."""
        if success:
            self._load_directory()
            toast = Adw.Toast(title="Folder created")
            win = self.get_ancestor(Adw.ApplicationWindow)
            if hasattr(win, '_toast_overlay'):
                win._toast_overlay.add_toast(toast)
        else:
            dialog = Adw.AlertDialog(
                heading="Folder Creation Failed",
                body=str(error),
            )
            dialog.add_response("close", "Close")
            dialog.set_default_response("close")
            dialog.present(self.get_ancestor(Gtk.Window))
