# keyring.py
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

import gi

gi.require_version('Secret', '1')

from gi.repository import Secret

SCHEMA = Secret.Schema.new(
    "io.github.nico359.cloudsend",
    Secret.SchemaFlags.NONE,
    {"type": Secret.SchemaAttributeType.STRING},
)

_CREDENTIAL_TYPES = ("server_url", "username", "app_password")


def store_credentials(server_url, username, app_password, callback=None):
    """Store all three credentials in the secret service."""
    values = {
        "server_url": server_url,
        "username": username,
        "app_password": app_password,
    }

    if callback is None:
        try:
            for cred_type, password in values.items():
                Secret.password_store_sync(
                    SCHEMA,
                    {"type": cred_type},
                    Secret.COLLECTION_DEFAULT,
                    f"Cloudsend {cred_type}",
                    password,
                    None,
                )
        except Exception:
            return False
        return True

    stored_count = 0
    total = len(values)
    items = list(values.items())

    def _on_store_complete(_source, result, cred_type):
        nonlocal stored_count
        try:
            Secret.password_store_finish(result)
        except Exception:
            callback(False)
            return
        stored_count += 1
        if stored_count == total:
            callback(True)

    for cred_type, password in items:
        Secret.password_store(
            SCHEMA,
            {"type": cred_type},
            Secret.COLLECTION_DEFAULT,
            f"Cloudsend {cred_type}",
            password,
            None,
            lambda source, result, ct=cred_type: _on_store_complete(source, result, ct),
        )


def load_credentials(callback=None):
    """Load all three credentials. Returns a dict or None if any are missing."""
    if callback is None:
        try:
            result = {}
            for cred_type in _CREDENTIAL_TYPES:
                password = Secret.password_lookup_sync(
                    SCHEMA,
                    {"type": cred_type},
                    None,
                )
                if password is None:
                    return None
                result[cred_type] = password
            return result
        except Exception:
            return None

    loaded = {}
    total = len(_CREDENTIAL_TYPES)

    def _on_lookup_complete(_source, result, cred_type):
        try:
            password = Secret.password_lookup_finish(result)
        except Exception:
            callback(None)
            return
        if password is None:
            callback(None)
            return
        loaded[cred_type] = password
        if len(loaded) == total:
            callback(loaded)

    for cred_type in _CREDENTIAL_TYPES:
        Secret.password_lookup(
            SCHEMA,
            {"type": cred_type},
            None,
            lambda source, result, ct=cred_type: _on_lookup_complete(source, result, ct),
        )


def clear_credentials(callback=None):
    """Clear all three credentials from the secret service."""
    if callback is None:
        try:
            for cred_type in _CREDENTIAL_TYPES:
                Secret.password_clear_sync(
                    SCHEMA,
                    {"type": cred_type},
                    None,
                )
        except Exception:
            return False
        return True

    cleared_count = 0
    total = len(_CREDENTIAL_TYPES)

    def _on_clear_complete(_source, result, cred_type):
        nonlocal cleared_count
        try:
            Secret.password_clear_finish(result)
        except Exception:
            callback(False)
            return
        cleared_count += 1
        if cleared_count == total:
            callback(True)

    for cred_type in _CREDENTIAL_TYPES:
        Secret.password_clear(
            SCHEMA,
            {"type": cred_type},
            None,
            lambda source, result, ct=cred_type: _on_clear_complete(source, result, ct),
        )


def has_credentials():
    """Synchronous check: returns True if all three credentials exist."""
    try:
        for cred_type in _CREDENTIAL_TYPES:
            password = Secret.password_lookup_sync(
                SCHEMA,
                {"type": cred_type},
                None,
            )
            if password is None:
                return False
        return True
    except Exception:
        return False
