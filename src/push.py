# push.py
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

from gi.repository import GLib

from .webdav import WebDAVClient
from .db import get_db


class PushEngine:
    """Upload/mirror engine that walks a local directory and pushes
    new or changed files to a remote WebDAV target."""

    def __init__(self, client, target, on_progress=None, on_complete=None):
        """
        client: WebDAVClient instance
        target: dict from db.get_target() with keys: id, local_path, remote_path, mode
        on_progress: callback(current_file, files_done, files_total) - called after each file
        on_complete: callback(success, summary_dict) - called when push finishes
            summary_dict: {'uploaded': int, 'skipped': int, 'deleted': int, 'errors': list[str]}
        """
        self._client = client
        self._target = target
        self._on_progress = on_progress
        self._on_complete = on_complete

        self._cancelled = False
        self._upload_queue = []
        self._local_rel_paths = set()
        self._files_total = 0
        self._files_done = 0
        self._summary = {
            'uploaded': 0,
            'skipped': 0,
            'deleted': 0,
            'errors': [],
        }

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def start(self, force=False):
        """Begin the push process.

        If force is True, clear all file_state records for this target
        and re-upload everything regardless of mtime/size.
        """
        self._cancelled = False
        self._summary = {
            'uploaded': 0,
            'skipped': 0,
            'deleted': 0,
            'errors': [],
        }

        db = get_db()
        target_id = self._target['id']
        local_path = self._target['local_path']
        remote_base = self._target['remote_path'].rstrip('/')

        if force:
            db.clear_file_states(target_id)

        # Synchronously walk the local directory
        all_files = []
        for dirpath, _dirnames, filenames in os.walk(local_path):
            for fname in filenames:
                abs_path = os.path.join(dirpath, fname)
                rel_path = os.path.relpath(abs_path, local_path)
                # Normalise to forward slashes for remote paths
                rel_path = rel_path.replace(os.sep, '/')
                mtime = os.path.getmtime(abs_path)
                size = os.path.getsize(abs_path)
                all_files.append({
                    'rel_path': rel_path,
                    'local_path': abs_path,
                    'remote_path': remote_base + '/' + rel_path,
                    'mtime': mtime,
                    'size': size,
                })

        # Keep a set of all local relative paths for the mirror pass
        self._local_rel_paths = {f['rel_path'] for f in all_files}

        # Filter to files that actually need uploading
        upload_list = []
        for entry in all_files:
            if force:
                upload_list.append(entry)
                continue
            state = db.get_file_state(target_id, entry['rel_path'])
            if state and state['mtime'] == entry['mtime'] and state['size'] == entry['size']:
                self._summary['skipped'] += 1
            else:
                upload_list.append(entry)

        self._upload_queue = upload_list
        self._files_total = len(upload_list)
        self._files_done = 0

        # Collect unique parent directories, sorted shallowest first
        dirs_set = set()
        for entry in upload_list:
            parent = os.path.dirname(entry['rel_path'])
            while parent:
                dirs_set.add(parent)
                parent = os.path.dirname(parent)

        dirs_sorted = sorted(dirs_set, key=lambda d: d.count('/'))
        remote_dirs = [remote_base + '/' + d for d in dirs_sorted]

        # Start the async chain: create dirs → upload files → mirror → complete
        self._create_directories(remote_dirs, self._upload_next)

    def cancel(self):
        """Cancel the running push. The upload loop will stop between files."""
        self._cancelled = True

    # ------------------------------------------------------------------
    # Internal async chain
    # ------------------------------------------------------------------

    def _create_directories(self, dirs_list, callback):
        """Create remote directories one by one via callback chaining."""
        if not dirs_list:
            callback()
            return

        remaining = list(dirs_list)

        def _create_next():
            if not remaining:
                callback()
                return
            current_dir = remaining.pop(0)

            def _on_done(success, error):
                if not success:
                    self._summary['errors'].append(
                        f"mkdir {current_dir}: {error}"
                    )
                _create_next()

            self._client.create_directory(current_dir, _on_done)

        _create_next()

    def _upload_next(self):
        """Upload the next file in the queue, chaining via WebDAV callback."""
        if self._cancelled or not self._upload_queue:
            self._on_uploads_finished()
            return

        entry = self._upload_queue.pop(0)

        def _on_upload_done(success, error):
            if success:
                db = get_db()
                db.upsert_file_state(
                    self._target['id'],
                    entry['rel_path'],
                    entry['mtime'],
                    entry['size'],
                )
                self._summary['uploaded'] += 1
                self._files_done += 1
                if self._on_progress:
                    GLib.idle_add(
                        self._on_progress,
                        entry['rel_path'],
                        self._files_done,
                        self._files_total,
                    )
            else:
                self._summary['errors'].append(
                    f"upload {entry['rel_path']}: {error}"
                )
                self._files_done += 1

            self._upload_next()

        self._client.upload_file(
            entry['local_path'], entry['remote_path'], _on_upload_done
        )

    def _on_uploads_finished(self):
        """Called after all uploads complete (or cancel). Run mirror if needed."""
        if self._target['mode'] == 'mirror' and not self._cancelled:
            self._mirror_pass(self._finish)
        else:
            self._finish()

    def _mirror_pass(self, callback):
        """Delete remote files that no longer exist locally (mirror mode).

        Lists the remote directory recursively by collecting subdirectory
        listings, then deletes any remote file whose relative path is not
        in the local file set.
        """
        remote_base = self._target['remote_path'].rstrip('/')
        to_delete = []

        # We collect all remote relative paths via recursive listing,
        # then delete those not present locally.
        def _list_recursive(remote_dir, prefix, done_cb):
            """List *remote_dir*, recurse into subdirectories, call done_cb
            when the full subtree has been enumerated."""
            self._client.list_directory(remote_dir, lambda ok, res: _on_list(
                ok, res, remote_dir, prefix, done_cb,
            ))

        def _on_list(success, result, remote_dir, prefix, done_cb):
            if not success:
                self._summary['errors'].append(
                    f"mirror list {remote_dir}: {result}"
                )
                done_cb()
                return

            # Separate files and subdirectories
            subdirs = []
            for item in result:
                rel = prefix + '/' + item['name'] if prefix else item['name']
                if item['is_dir']:
                    subdirs.append((remote_dir.rstrip('/') + '/' + item['name'], rel))
                else:
                    if rel not in self._local_rel_paths:
                        to_delete.append(remote_base + '/' + rel)

            # Recurse into subdirectories sequentially
            def _recurse_subdirs():
                if not subdirs:
                    done_cb()
                    return
                rd, rp = subdirs.pop(0)
                _list_recursive(rd, rp, _recurse_subdirs)

            _recurse_subdirs()

        def _on_listing_complete():
            # Delete collected remote paths one by one
            self._delete_remote_files(to_delete, callback)

        _list_recursive(remote_base, '', _on_listing_complete)

    def _delete_remote_files(self, paths, callback):
        """Delete a list of remote paths sequentially via callback chaining."""
        if not paths:
            callback()
            return

        remaining = list(paths)

        def _delete_next():
            if not remaining:
                callback()
                return
            current = remaining.pop(0)

            def _on_deleted(success, error):
                if success:
                    self._summary['deleted'] += 1
                else:
                    self._summary['errors'].append(
                        f"delete {current}: {error}"
                    )
                _delete_next()

            self._client.delete(current, _on_deleted)

        _delete_next()

    def _finish(self):
        """Final step: update DB timestamp and fire the on_complete callback."""
        db = get_db()
        db.update_last_push(self._target['id'])

        success = not self._summary['errors']
        if self._on_complete:
            GLib.idle_add(self._on_complete, success, self._summary)
