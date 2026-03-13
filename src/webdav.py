# webdav.py
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

import base64
import xml.etree.ElementTree as ET
from urllib.parse import quote, unquote, urlparse

import gi

gi.require_version('Soup', '3.0')

from gi.repository import Soup, GLib, Gio


PROPFIND_BODY = (
    '<?xml version="1.0" encoding="UTF-8"?>'
    '<d:propfind xmlns:d="DAV:">'
    '<d:prop>'
    '<d:getcontentlength/>'
    '<d:getlastmodified/>'
    '<d:resourcetype/>'
    '</d:prop>'
    '</d:propfind>'
)

DAV_NS = 'DAV:'


class WebDAVClient:
    """WebDAV client for Nextcloud using libsoup3.

    All operations are async (callback-based) and compatible with the
    GLib main loop so the GTK UI is never blocked.
    """

    def __init__(self, server_url, username, password):
        self._server_url = server_url.rstrip('/')
        self._username = username
        self._password = password

        parsed = urlparse(self._server_url)
        self._base_url = (
            f"{parsed.scheme}://{parsed.hostname}"
            + (f":{parsed.port}" if parsed.port else "")
            + f"/remote.php/dav/files/{quote(self._username, safe='')}/"
        )

        creds = f"{self._username}:{self._password}"
        self._auth_header = 'Basic ' + base64.b64encode(
            creds.encode('utf-8')
        ).decode('ascii')

        self._session = Soup.Session.new()

    # ------------------------------------------------------------------
    # Helper
    # ------------------------------------------------------------------

    def _build_url(self, remote_path):
        """Construct the full WebDAV URL from a relative remote path."""
        stripped = remote_path.strip('/')
        if not stripped:
            return self._base_url
        encoded_parts = '/'.join(
            quote(part, safe='') for part in stripped.split('/')
        )
        return self._base_url + encoded_parts + (
            '/' if remote_path.endswith('/') else ''
        )

    def _set_auth(self, msg):
        """Set the Authorization header on a Soup.Message."""
        msg.get_request_headers().replace('Authorization', self._auth_header)

    def _read_response_body(self, input_stream):
        """Synchronously read all bytes from a Gio.InputStream."""
        chunks = []
        while True:
            chunk = input_stream.read_bytes(65536, None)
            if chunk is None or chunk.get_size() == 0:
                break
            chunks.append(chunk.get_data())
        input_stream.close(None)
        return b''.join(chunks)

    def _parse_multistatus(self, xml_bytes):
        """Parse a PROPFIND multistatus XML response into a list of dicts."""
        root = ET.fromstring(xml_bytes)
        items = []
        responses = root.findall(f'{{{DAV_NS}}}response')

        # Skip the first response (the directory itself)
        for resp in responses[1:]:
            href_el = resp.find(f'{{{DAV_NS}}}href')
            href = href_el.text if href_el is not None else ''

            propstat = resp.find(f'{{{DAV_NS}}}propstat')
            if propstat is None:
                continue
            prop = propstat.find(f'{{{DAV_NS}}}prop')
            if prop is None:
                continue

            restype = prop.find(f'{{{DAV_NS}}}resourcetype')
            is_dir = (
                restype is not None
                and restype.find(f'{{{DAV_NS}}}collection') is not None
            )

            size_el = prop.find(f'{{{DAV_NS}}}getcontentlength')
            size = int(size_el.text) if size_el is not None and size_el.text else 0

            mtime_el = prop.find(f'{{{DAV_NS}}}getlastmodified')
            mtime = mtime_el.text if mtime_el is not None and mtime_el.text else ''

            # Derive a human-readable name from the href
            name = href.rstrip('/').rsplit('/', 1)[-1]
            name = unquote(name)

            items.append({
                'name': name,
                'is_dir': is_dir,
                'size': size,
                'mtime': mtime,
            })

        return items

    # ------------------------------------------------------------------
    # Public async operations
    # ------------------------------------------------------------------

    def test_connection(self, callback):
        """PROPFIND Depth:0 on the WebDAV root to verify connectivity."""
        url = self._base_url
        msg = Soup.Message.new('PROPFIND', url)
        self._set_auth(msg)
        msg.get_request_headers().replace('Depth', '0')

        body_bytes = GLib.Bytes.new(PROPFIND_BODY.encode('utf-8'))
        msg.set_request_body_from_bytes('application/xml', body_bytes)

        def on_send_finish(session, result):
            try:
                input_stream = session.send_finish(result)
                status = msg.get_status()
                self._read_response_body(input_stream)

                if 200 <= status <= 299:
                    callback(True, None)
                else:
                    callback(False, f"HTTP {status}")
            except Exception as e:
                callback(False, str(e))

        self._session.send_async(
            msg, GLib.PRIORITY_DEFAULT, None, on_send_finish
        )

    def list_directory(self, remote_path, callback):
        """PROPFIND Depth:1 to list the contents of a remote directory."""
        path = remote_path if remote_path.endswith('/') else remote_path + '/'
        url = self._build_url(path)
        msg = Soup.Message.new('PROPFIND', url)
        self._set_auth(msg)
        msg.get_request_headers().replace('Depth', '1')

        body_bytes = GLib.Bytes.new(PROPFIND_BODY.encode('utf-8'))
        msg.set_request_body_from_bytes('application/xml', body_bytes)

        def on_send_finish(session, result):
            try:
                input_stream = session.send_finish(result)
                status = msg.get_status()
                body = self._read_response_body(input_stream)

                if 200 <= status <= 299:
                    items = self._parse_multistatus(body)
                    callback(True, items)
                else:
                    callback(False, f"HTTP {status}")
            except Exception as e:
                callback(False, str(e))

        self._session.send_async(
            msg, GLib.PRIORITY_DEFAULT, None, on_send_finish
        )

    def upload_file(self, local_path, remote_path, callback):
        """PUT a local file to the remote WebDAV path."""
        url = self._build_url(remote_path)
        msg = Soup.Message.new('PUT', url)
        self._set_auth(msg)

        try:
            with open(local_path, 'rb') as f:
                file_data = f.read()
        except Exception as e:
            callback(False, str(e))
            return

        body_bytes = GLib.Bytes.new(file_data)
        msg.set_request_body_from_bytes('application/octet-stream', body_bytes)

        def on_send_finish(session, result):
            try:
                input_stream = session.send_finish(result)
                status = msg.get_status()
                self._read_response_body(input_stream)

                if 200 <= status <= 299:
                    callback(True, None)
                else:
                    callback(False, f"HTTP {status}")
            except Exception as e:
                callback(False, str(e))

        self._session.send_async(
            msg, GLib.PRIORITY_DEFAULT, None, on_send_finish
        )

    def create_directory(self, remote_path, callback):
        """MKCOL to create a remote directory. HTTP 405 (already exists) is OK."""
        path = remote_path if remote_path.endswith('/') else remote_path + '/'
        url = self._build_url(path)
        msg = Soup.Message.new('MKCOL', url)
        self._set_auth(msg)

        def on_send_finish(session, result):
            try:
                input_stream = session.send_finish(result)
                status = msg.get_status()
                self._read_response_body(input_stream)

                if 200 <= status <= 299 or status == 405:
                    callback(True, None)
                else:
                    callback(False, f"HTTP {status}")
            except Exception as e:
                callback(False, str(e))

        self._session.send_async(
            msg, GLib.PRIORITY_DEFAULT, None, on_send_finish
        )

    def delete(self, remote_path, callback):
        """DELETE a remote file or directory."""
        url = self._build_url(remote_path)
        msg = Soup.Message.new('DELETE', url)
        self._set_auth(msg)

        def on_send_finish(session, result):
            try:
                input_stream = session.send_finish(result)
                status = msg.get_status()
                self._read_response_body(input_stream)

                if 200 <= status <= 299:
                    callback(True, None)
                else:
                    callback(False, f"HTTP {status}")
            except Exception as e:
                callback(False, str(e))

        self._session.send_async(
            msg, GLib.PRIORITY_DEFAULT, None, on_send_finish
        )
