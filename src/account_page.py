# account_page.py
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


@Gtk.Template(resource_path='/io/github/nico359/cloudsend/account_page.ui')
class CloudsendAccountPage(Adw.NavigationPage):
    __gtype_name__ = 'CloudsendAccountPage'

    server_url_row = Gtk.Template.Child()
    username_row = Gtk.Template.Child()
    password_row = Gtk.Template.Child()
    test_button = Gtk.Template.Child()
    save_button = Gtk.Template.Child()
    remove_button = Gtk.Template.Child()
    status_label = Gtk.Template.Child()

    def __init__(self, **kwargs):
        super().__init__(**kwargs)
        self.test_button.connect('clicked', self._on_test_clicked)
        self.save_button.connect('clicked', self._on_save_clicked)
        self.remove_button.connect('clicked', self._on_remove_clicked)
        self._load_existing()

    def _load_existing(self):
        """Load existing credentials into the form fields."""
        creds = keyring.load_credentials()
        if creds:
            self.server_url_row.set_text(creds['server_url'])
            self.username_row.set_text(creds['username'])
            self.password_row.set_text(creds['app_password'])
            self.remove_button.set_visible(True)

    def _validate_server_url(self, server):
        """Return True if server URL is valid, otherwise show error and return False."""
        if not server.startswith('http://') and not server.startswith('https://'):
            self.status_label.set_text("Server URL must start with http:// or https://")
            self.status_label.add_css_class('error')
            self.status_label.remove_css_class('success')
            return False
        return True

    def _on_test_clicked(self, button):
        """Test connection with current field values."""
        server = self.server_url_row.get_text().strip()
        user = self.username_row.get_text().strip()
        pwd = self.password_row.get_text().strip()

        if not server or not user or not pwd:
            self.status_label.set_text("Please fill in all fields")
            self.status_label.add_css_class('error')
            return

        if not self._validate_server_url(server):
            return

        self.status_label.set_text("Testing connection…")
        self.status_label.remove_css_class('error')
        self.status_label.remove_css_class('success')
        self.test_button.set_sensitive(False)

        client = WebDAVClient(server, user, pwd)
        client.test_connection(self._on_test_result)

    @staticmethod
    def _friendly_error(error):
        """Convert raw exception text into a user-friendly message."""
        err = str(error).lower()
        if any(kw in err for kw in (
            'connectionerror', 'newconnectionerror', 'nameresolutionerror',
            'nodename nor servname', 'name or service not known',
            'no address associated', 'connection refused', 'timed out',
            'timeout', 'unreachable',
        )):
            return "Could not reach server — check the URL and your network connection"
        return str(error)

    def _on_test_result(self, success, error):
        """Handle test connection result (called from async callback)."""
        def update():
            self.test_button.set_sensitive(True)
            if success:
                self.status_label.set_text("Connection successful ✓")
                self.status_label.remove_css_class('error')
                self.status_label.add_css_class('success')
            else:
                msg = self._friendly_error(error)
                self.status_label.set_text(f"Connection failed: {msg}")
                self.status_label.add_css_class('error')
                self.status_label.remove_css_class('success')
        GLib.idle_add(update)

    def _on_save_clicked(self, button):
        """Save credentials to keyring."""
        server = self.server_url_row.get_text().strip()
        user = self.username_row.get_text().strip()
        pwd = self.password_row.get_text().strip()

        if not server or not user or not pwd:
            self.status_label.set_text("Please fill in all fields")
            self.status_label.add_css_class('error')
            return

        if not self._validate_server_url(server):
            return

        keyring.store_credentials(server, user, pwd)
        self.remove_button.set_visible(True)
        self.status_label.set_text("Account saved ✓")
        self.status_label.remove_css_class('error')
        self.status_label.add_css_class('success')

        toast = Adw.Toast(title="Account saved successfully")
        win = self.get_ancestor(Adw.ApplicationWindow)
        if hasattr(win, '_toast_overlay'):
            win._toast_overlay.add_toast(toast)

    def _on_remove_clicked(self, button):
        """Clear credentials from keyring."""
        keyring.clear_credentials()
        self.server_url_row.set_text('')
        self.username_row.set_text('')
        self.password_row.set_text('')
        self.remove_button.set_visible(False)
        self.status_label.set_text("Account removed")
        self.status_label.remove_css_class('success')
        self.status_label.remove_css_class('error')
