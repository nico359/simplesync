# db.py
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
import sqlite3

from gi.repository import GLib


_instance = None


def get_db():
    """Return the singleton Database instance, creating it on first call."""
    global _instance
    if _instance is None:
        _instance = Database()
    return _instance


class Database:
    """SQLite database for CloudSend target and file-state tracking."""

    def __init__(self):
        data_dir = os.path.join(GLib.get_user_data_dir(), "cloudsend")
        os.makedirs(data_dir, exist_ok=True)

        db_path = os.path.join(data_dir, "cloudsend.db")
        self._conn = sqlite3.connect(db_path)
        self._conn.row_factory = sqlite3.Row
        self._conn.execute("PRAGMA journal_mode=WAL")
        self._conn.execute("PRAGMA foreign_keys=ON")
        self._init_db()

    # ------------------------------------------------------------------
    # Schema
    # ------------------------------------------------------------------

    def _init_db(self):
        self._conn.executescript("""
            CREATE TABLE IF NOT EXISTS targets (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                local_path  TEXT NOT NULL,
                remote_path TEXT NOT NULL,
                mode        TEXT NOT NULL DEFAULT 'upload',
                last_push   TEXT,
                created_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS file_state (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                target_id   INTEGER NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
                rel_path    TEXT NOT NULL,
                mtime       REAL NOT NULL,
                size        INTEGER NOT NULL,
                uploaded_at TEXT NOT NULL,
                UNIQUE(target_id, rel_path)
            );
        """)

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------

    @staticmethod
    def _row_to_dict(cursor, row):
        """Convert a sqlite3.Row to a plain dict."""
        return {col[0]: row[idx] for idx, col in enumerate(cursor.description)}

    # ------------------------------------------------------------------
    # Target management
    # ------------------------------------------------------------------

    def get_targets(self):
        """Return list of all targets as dicts."""
        cur = self._conn.execute("SELECT * FROM targets")
        return [dict(row) for row in cur.fetchall()]

    def get_target(self, target_id):
        """Return single target as dict or None."""
        cur = self._conn.execute("SELECT * FROM targets WHERE id = ?", (target_id,))
        row = cur.fetchone()
        return dict(row) if row else None

    def add_target(self, local_path, remote_path, mode="upload"):
        """Insert and return the new target as dict."""
        cur = self._conn.execute(
            "INSERT INTO targets (local_path, remote_path, mode) VALUES (?, ?, ?)",
            (local_path, remote_path, mode),
        )
        self._conn.commit()
        return self.get_target(cur.lastrowid)

    def update_target(self, target_id, local_path=None, remote_path=None, mode=None):
        """Update specified fields on a target."""
        fields = []
        values = []
        if local_path is not None:
            fields.append("local_path = ?")
            values.append(local_path)
        if remote_path is not None:
            fields.append("remote_path = ?")
            values.append(remote_path)
        if mode is not None:
            fields.append("mode = ?")
            values.append(mode)
        if not fields:
            return
        values.append(target_id)
        self._conn.execute(
            f"UPDATE targets SET {', '.join(fields)} WHERE id = ?", values
        )
        self._conn.commit()

    def delete_target(self, target_id):
        """Delete target (cascade deletes file_state)."""
        self._conn.execute("DELETE FROM targets WHERE id = ?", (target_id,))
        self._conn.commit()

    def update_last_push(self, target_id):
        """Set last_push to current datetime."""
        self._conn.execute(
            "UPDATE targets SET last_push = datetime('now') WHERE id = ?",
            (target_id,),
        )
        self._conn.commit()

    # ------------------------------------------------------------------
    # File state management
    # ------------------------------------------------------------------

    def get_file_state(self, target_id, rel_path):
        """Return file state dict or None."""
        cur = self._conn.execute(
            "SELECT * FROM file_state WHERE target_id = ? AND rel_path = ?",
            (target_id, rel_path),
        )
        row = cur.fetchone()
        return dict(row) if row else None

    def get_all_file_states(self, target_id):
        """Return all file states for a target as list of dicts."""
        cur = self._conn.execute(
            "SELECT * FROM file_state WHERE target_id = ?", (target_id,)
        )
        return [dict(row) for row in cur.fetchall()]

    def upsert_file_state(self, target_id, rel_path, mtime, size):
        """Insert or update file state, set uploaded_at to now."""
        self._conn.execute(
            """INSERT INTO file_state (target_id, rel_path, mtime, size, uploaded_at)
               VALUES (?, ?, ?, ?, datetime('now'))
               ON CONFLICT(target_id, rel_path)
               DO UPDATE SET mtime = excluded.mtime,
                             size = excluded.size,
                             uploaded_at = excluded.uploaded_at""",
            (target_id, rel_path, mtime, size),
        )
        self._conn.commit()

    def delete_file_state(self, target_id, rel_path):
        """Delete a single file state entry."""
        self._conn.execute(
            "DELETE FROM file_state WHERE target_id = ? AND rel_path = ?",
            (target_id, rel_path),
        )
        self._conn.commit()

    def clear_file_states(self, target_id):
        """Delete all file state entries for a target."""
        self._conn.execute(
            "DELETE FROM file_state WHERE target_id = ?", (target_id,)
        )
        self._conn.commit()
