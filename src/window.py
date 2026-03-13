# window.py
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

from gi.repository import Adw
from gi.repository import Gtk

from .targets_page import CloudsendTargetsPage
from .account_page import CloudsendAccountPage


@Gtk.Template(resource_path='/io/github/nico359/cloudsend/window.ui')
class CloudsendWindow(Adw.ApplicationWindow):
    __gtype_name__ = 'CloudsendWindow'

    toast_overlay = Gtk.Template.Child()
    navigation_view = Gtk.Template.Child()

    def __init__(self, **kwargs):
        super().__init__(**kwargs)
        # Expose toast_overlay for child pages
        self._toast_overlay = self.toast_overlay

        # Push the targets page as the root
        self._targets_page = CloudsendTargetsPage()
        self.navigation_view.push(self._targets_page)
        self._targets_page.refresh()

    def show_account_page(self):
        """Push the account settings page onto the navigation stack."""
        page = CloudsendAccountPage()
        self.navigation_view.push(page)
