use base64::Engine;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Default chunk size used for Nextcloud chunked upload v2 (10 MiB).
const DEFAULT_CHUNK_SIZE: u64 = 10 * 1024 * 1024;
/// Nextcloud requires chunks to be at least 5 MB (the final chunk may be smaller).
const MIN_CHUNK_SIZE: u64 = 5 * 1024 * 1024;
/// Upper bound Nextcloud enforces for a single chunk.
const MAX_CHUNK_SIZE: u64 = 5 * 1024 * 1024 * 1024;

/// Subset of the Nextcloud capabilities that are relevant for uploading.
#[derive(Debug, Clone, Default)]
pub struct UploadCapabilities {
    /// Whether the server supports chunked upload v2 (`dav.chunking >= 1.0`).
    pub chunking_ng: bool,
    /// Maximum chunk size advertised by the server (`files.chunked_upload.max_size`).
    pub max_chunk_size: u64,
    /// Maximum number of parallel chunk uploads (`files.chunked_upload.max_parallel_count`).
    #[allow(dead_code)]
    pub max_parallel: u32,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RemoteItem {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct WebDAVClient {
    server_url: String,
    username: String,
    app_password: String,
}

#[derive(Debug)]
pub enum WebDAVError {
    Http(String),
    Parse(String),
    Io(String),
    Cancelled,
}

impl std::fmt::Display for WebDAVError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebDAVError::Http(msg) => write!(f, "HTTP error: {}", msg),
            WebDAVError::Parse(msg) => write!(f, "Parse error: {}", msg),
            WebDAVError::Io(msg) => write!(f, "IO error: {}", msg),
            WebDAVError::Cancelled => write!(f, "Cancelled"),
        }
    }
}

impl WebDAVClient {
    pub fn new(server_url: &str, username: &str, app_password: &str) -> Self {
        let server_url = server_url.trim_end_matches('/').to_string();
        Self {
            server_url,
            username: username.to_string(),
            app_password: app_password.to_string(),
        }
    }

    fn base_url(&self) -> String {
        format!("{}/remote.php/dav/files/{}", self.server_url, self.username)
    }

    fn auth_header(&self) -> String {
        let creds = format!("{}:{}", self.username, self.app_password);
        let encoded = base64::engine::general_purpose::STANDARD.encode(creds.as_bytes());
        format!("Basic {}", encoded)
    }

    fn client(&self) -> Result<reqwest::blocking::Client, WebDAVError> {
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| WebDAVError::Http(e.to_string()))
    }

    /// Test connection by doing a PROPFIND on the root
    pub fn test_connection(&self) -> Result<(), WebDAVError> {
        let url = format!("{}/", self.base_url());
        let client = self.client()?;
        let resp = client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .header("Authorization", self.auth_header())
            .header("Depth", "0")
            .header("Content-Type", "application/xml")
            .body(r#"<?xml version="1.0" encoding="UTF-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:resourcetype/>
  </d:prop>
</d:propfind>"#)
            .send()
            .map_err(|e| WebDAVError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        if status == 207 || status == 200 {
            Ok(())
        } else {
            Err(WebDAVError::Http(format!("Server returned status {}", status)))
        }
    }

    /// Check whether a remote path exists. Returns Ok(true), Ok(false) for 404,
    /// or Err for network/auth failures.
    pub fn path_exists(&self, remote_path: &str) -> Result<bool, WebDAVError> {
        let path = remote_path.trim_matches('/');
        let url = if path.is_empty() {
            format!("{}/", self.base_url())
        } else {
            format!("{}/{}/", self.base_url(), encode_path(path))
        };

        let client = self.client()?;
        let resp = client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .header("Authorization", self.auth_header())
            .header("Depth", "0")
            .header("Content-Type", "application/xml")
            .body(r#"<?xml version="1.0" encoding="UTF-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:resourcetype/>
  </d:prop>
</d:propfind>"#)
            .send()
            .map_err(|e| WebDAVError::Http(e.to_string()))?;

        match resp.status().as_u16() {
            207 | 200 => Ok(true),
            404 => Ok(false),
            status => Err(WebDAVError::Http(format!("Server returned status {}", status))),
        }
    }

    /// List contents of a remote directory
    pub fn list_directory(&self, remote_path: &str) -> Result<Vec<RemoteItem>, WebDAVError> {
        let path = remote_path.trim_matches('/');
        let url = if path.is_empty() {
            format!("{}/", self.base_url())
        } else {
            format!("{}/{}/", self.base_url(), encode_path(path))
        };

        let client = self.client()?;
        let resp = client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .header("Authorization", self.auth_header())
            .header("Depth", "1")
            .header("Content-Type", "application/xml")
            .body(r#"<?xml version="1.0" encoding="UTF-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:resourcetype/>
    <d:getcontentlength/>
    <d:displayname/>
  </d:prop>
</d:propfind>"#)
            .send()
            .map_err(|e| WebDAVError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        if status != 207 {
            return Err(WebDAVError::Http(format!("PROPFIND returned status {}", status)));
        }

        let body = resp.text().map_err(|e| WebDAVError::Http(e.to_string()))?;
        parse_propfind_response(&body, remote_path)
    }

    /// List directory recursively (for mirror mode)
    pub fn list_directory_recursive(&self, remote_path: &str) -> Result<Vec<String>, WebDAVError> {
        let mut all_files = Vec::new();
        self.list_recursive_inner(remote_path, &mut all_files)?;
        Ok(all_files)
    }

    fn list_recursive_inner(&self, path: &str, files: &mut Vec<String>) -> Result<(), WebDAVError> {
        let items = self.list_directory(path)?;
        for item in items {
            let full_path = if path == "/" || path.is_empty() {
                format!("/{}", item.name)
            } else {
                format!("{}/{}", path.trim_end_matches('/'), item.name)
            };

            if item.is_dir {
                self.list_recursive_inner(&full_path, files)?;
            } else {
                files.push(full_path);
            }
        }
        Ok(())
    }

    /// Upload a file
    pub fn upload_file(&self, local_path: &str, remote_path: &str) -> Result<(), WebDAVError> {
        let path = remote_path.trim_start_matches('/');
        let url = format!("{}/{}", self.base_url(), encode_path(path));

        let data = std::fs::read(local_path)
            .map_err(|e| WebDAVError::Io(format!("Failed to read {}: {}", local_path, e)))?;

        let client = self.client()?;
        let resp = client
            .put(&url)
            .header("Authorization", self.auth_header())
            .timeout(self.upload_timeout(data.len() as u64))
            .body(data)
            .send()
            .map_err(|e| WebDAVError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        if status == 200 || status == 201 || status == 204 {
            Ok(())
        } else {
            Err(WebDAVError::Http(format!("PUT returned status {}", status)))
        }
    }

    /// Create a remote directory (MKCOL). 405 = already exists = OK.
    pub fn create_directory(&self, remote_path: &str) -> Result<(), WebDAVError> {
        let path = remote_path.trim_start_matches('/');
        let url = format!("{}/{}/", self.base_url(), encode_path(path));

        let client = self.client()?;
        let resp = client
            .request(reqwest::Method::from_bytes(b"MKCOL").unwrap(), &url)
            .header("Authorization", self.auth_header())
            .send()
            .map_err(|e| WebDAVError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        // 201 Created, 405 Already exists - both are fine
        if status == 201 || status == 405 {
            Ok(())
        } else {
            Err(WebDAVError::Http(format!("MKCOL returned status {}", status)))
        }
    }

    /// Delete a remote file or directory
    pub fn delete(&self, remote_path: &str) -> Result<(), WebDAVError> {
        let path = remote_path.trim_start_matches('/');
        let url = format!("{}/{}", self.base_url(), encode_path(path));

        let client = self.client()?;
        let resp = client
            .delete(&url)
            .header("Authorization", self.auth_header())
            .send()
            .map_err(|e| WebDAVError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        if status == 200 || status == 204 || status == 404 {
            Ok(())
        } else {
            Err(WebDAVError::Http(format!("DELETE returned status {}", status)))
        }
    }

    /// Download a remote file to a local path (GET request)
    pub fn download_file(&self, remote_path: &str, local_path: &str) -> Result<(), WebDAVError> {
        let path = remote_path.trim_start_matches('/');
        let url = format!("{}/{}", self.base_url(), encode_path(path));

        let client = self.client()?;
        let resp = client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .map_err(|e| WebDAVError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        if status != 200 {
            return Err(WebDAVError::Http(format!("GET returned status {}", status)));
        }

        let bytes = resp.bytes().map_err(|e| WebDAVError::Http(e.to_string()))?;

        if let Some(parent) = std::path::Path::new(local_path).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| WebDAVError::Io(format!("Failed to create dir: {}", e)))?;
        }

        std::fs::write(local_path, &bytes)
            .map_err(|e| WebDAVError::Io(format!("Failed to write {}: {}", local_path, e)))?;

        Ok(())
    }

    /// Check if a remote file exists (HEAD request)
    #[allow(dead_code)]
    pub fn file_exists(&self, remote_path: &str) -> Result<bool, WebDAVError> {
        let path = remote_path.trim_start_matches('/');
        let url = format!("{}/{}", self.base_url(), encode_path(path));

        let client = self.client()?;
        let resp = client
            .head(&url)
            .header("Authorization", self.auth_header())
            .send()
            .map_err(|e| WebDAVError::Http(e.to_string()))?;

        Ok(resp.status().as_u16() == 200)
    }

    /// Query the Nextcloud capabilities that are relevant for uploading.
    ///
    /// Returns defaults (chunking disabled) for plain WebDAV servers or whenever
    /// the endpoint cannot be reached/parsed, so callers can fall back to a
    /// normal PUT.
    pub fn upload_capabilities(&self) -> UploadCapabilities {
        let url = format!("{}/ocs/v1.php/cloud/capabilities", self.server_url);
        let client = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
        {
            Ok(c) => c,
            Err(_) => return UploadCapabilities::default(),
        };

        let resp = match client
            .get(&url)
            .header("Authorization", self.auth_header())
            .header("OCS-APIRequest", "true")
            .header("Accept", "application/json")
            .send()
        {
            Ok(r) => r,
            Err(_) => return UploadCapabilities::default(),
        };

        if !resp.status().is_success() {
            return UploadCapabilities::default();
        }

        let json: serde_json::Value = match resp.json() {
            Ok(v) => v,
            Err(_) => return UploadCapabilities::default(),
        };

        let caps = &json["ocs"]["data"]["capabilities"];
        let chunking_ng = match &caps["dav"]["chunking"] {
            serde_json::Value::String(s) => s
                .split('.')
                .next()
                .and_then(|major| major.parse::<u32>().ok())
                .map(|major| major >= 1)
                .unwrap_or(false),
            serde_json::Value::Number(n) => n.as_f64().map(|v| v >= 1.0).unwrap_or(false),
            _ => false,
        };

        let max_chunk_size = caps["files"]["chunked_upload"]["max_size"]
            .as_u64()
            .unwrap_or(0);
        let max_parallel = caps["files"]["chunked_upload"]["max_parallel_count"]
            .as_u64()
            .unwrap_or(0) as u32;

        UploadCapabilities {
            chunking_ng,
            max_chunk_size,
            max_parallel,
        }
    }

    /// Clamp the configured chunk size to what the server allows.
    pub fn effective_chunk_size(caps: &UploadCapabilities) -> u64 {
        let server_max = if caps.max_chunk_size == 0 {
            DEFAULT_CHUNK_SIZE
        } else {
            caps.max_chunk_size
        };
        DEFAULT_CHUNK_SIZE
            .min(server_max)
            .max(MIN_CHUNK_SIZE)
            .min(MAX_CHUNK_SIZE)
    }

    /// Upload a file using Nextcloud chunked upload v2 ("NG"), resuming any
    /// chunks that already exist on the server.
    ///
    /// `on_progress` is called with `(bytes_done, bytes_total)` after every
    /// successfully transferred chunk.
    pub fn upload_file_chunked(
        &self,
        local_path: &str,
        remote_path: &str,
        mtime: f64,
        chunk_size: u64,
        cancel: &AtomicBool,
        on_progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), WebDAVError> {
        let path = remote_path.trim_start_matches('/');
        let destination = format!("{}/{}", self.base_url(), encode_path(path));

        let size = std::fs::metadata(local_path)
            .map_err(|e| WebDAVError::Io(format!("Failed to stat {}: {}", local_path, e)))?
            .len();

        let chunk_size = chunk_size.clamp(MIN_CHUNK_SIZE, MAX_CHUNK_SIZE);
        let transfer_id = transfer_id_for(path, mtime, size);
        let folder_url = self.uploads_url(transfer_id);

        let mut file = std::fs::File::open(local_path)
            .map_err(|e| WebDAVError::Io(format!("Failed to open {}: {}", local_path, e)))?;

        // 1. Create the upload folder. 405 means it already exists, which is
        //    exactly what we want when resuming.
        self.create_upload_folder(&folder_url, &destination, size)?;

        // 2. Figure out how much the server already has.
        let mut offset = self.uploaded_bytes(&folder_url).unwrap_or(0);
        // Start over if the server state is inconsistent with the local file,
        // or if the existing chunks do not align with our chunk size (e.g. the
        // server's advertised max size changed between runs).
        if offset > size || (offset > 0 && offset < size && offset % chunk_size != 0) {
            let _ = self.delete_url(&format!("{}/", folder_url));
            self.create_upload_folder(&folder_url, &destination, size)?;
            offset = 0;
        }

        if offset > 0 {
            on_progress(offset, size);
        }

        // 3. Upload the remaining chunks.
        let client = self.client()?;
        while offset < size {
            if cancel.load(Ordering::Relaxed) {
                let _ = self.delete_url(&format!("{}/", folder_url));
                return Err(WebDAVError::Cancelled);
            }

            let this_chunk = std::cmp::min(chunk_size, size - offset);
            let index = offset / chunk_size + 1;

            file.seek(SeekFrom::Start(offset))
                .map_err(|e| WebDAVError::Io(format!("Failed to seek {}: {}", local_path, e)))?;
            let mut buffer = vec![0u8; this_chunk as usize];
            file.read_exact(&mut buffer)
                .map_err(|e| WebDAVError::Io(format!("Failed to read {}: {}", local_path, e)))?;

            let url = self.chunk_url(transfer_id, index);
            let resp = client
                .put(&url)
                .header("Authorization", self.auth_header())
                .header("OC-Chunk-Offset", offset.to_string())
                .header("OC-Total-Length", size.to_string())
                .header("Destination", &destination)
                .timeout(self.upload_timeout(this_chunk))
                .body(buffer)
                .send()
                .map_err(|e| WebDAVError::Http(e.to_string()))?;

            let status = resp.status().as_u16();
            if !(200..300).contains(&status) {
                return Err(WebDAVError::Http(format!(
                    "Chunk {} PUT returned status {}",
                    index, status
                )));
            }

            offset += this_chunk;
            on_progress(offset, size);
        }

        // 4. Ask the server to assemble the chunks into the destination file.
        self.assemble_upload(&folder_url, &destination, size, mtime)
    }

    /// Create the chunk upload folder (MKCOL). 405 = already exists.
    fn create_upload_folder(
        &self,
        folder_url: &str,
        destination: &str,
        size: u64,
    ) -> Result<(), WebDAVError> {
        let client = self.client()?;
        let resp = client
            .request(
                reqwest::Method::from_bytes(b"MKCOL").unwrap(),
                &format!("{}/", folder_url),
            )
            .header("Authorization", self.auth_header())
            .header("OC-Total-Length", size.to_string())
            .header("Destination", destination)
            .header("Content-Length", "0")
            .send()
            .map_err(|e| WebDAVError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        if status == 201 || status == 405 {
            Ok(())
        } else {
            Err(WebDAVError::Http(format!("MKCOL returned status {}", status)))
        }
    }

    /// Sum the sizes of contiguous chunks already present in the upload folder.
    fn uploaded_bytes(&self, folder_url: &str) -> Result<u64, WebDAVError> {
        let client = self.client()?;
        let resp = client
            .request(
                reqwest::Method::from_bytes(b"PROPFIND").unwrap(),
                &format!("{}/", folder_url),
            )
            .header("Authorization", self.auth_header())
            .header("Depth", "1")
            .header("Content-Type", "application/xml")
            .body(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:getcontentlength/>
  </d:prop>
</d:propfind>"#,
            )
            .send()
            .map_err(|e| WebDAVError::Http(e.to_string()))?;

        if resp.status().as_u16() != 207 {
            return Ok(0);
        }

        let body = resp.text().map_err(|e| WebDAVError::Http(e.to_string()))?;
        let doc = roxmltree::Document::parse(&body)
            .map_err(|e| WebDAVError::Parse(e.to_string()))?;

        let mut chunks: HashMap<u32, u64> = HashMap::new();
        for response in doc.descendants().filter(|n| n.has_tag_name("response")) {
            let href = response
                .descendants()
                .find(|n| n.has_tag_name("href"))
                .and_then(|n| n.text())
                .unwrap_or("");
            let decoded = urldecoding_decode(href);
            // The upload folder itself has a trailing slash; skip it.
            if decoded.ends_with('/') {
                continue;
            }
            let name = decoded.rsplit('/').next().unwrap_or("");
            if let Ok(index) = name.parse::<u32>() {
                let len: u64 = response
                    .descendants()
                    .find(|n| n.has_tag_name("getcontentlength"))
                    .and_then(|n| n.text())
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(0);
                chunks.insert(index, len);
            }
        }

        let mut offset = 0u64;
        let mut expected = 1u32;
        while let Some(len) = chunks.get(&expected) {
            offset += *len;
            expected += 1;
        }
        Ok(offset)
    }

    /// Assemble the uploaded chunks via `MOVE .../.file` to the destination.
    fn assemble_upload(
        &self,
        folder_url: &str,
        destination: &str,
        size: u64,
        mtime: f64,
    ) -> Result<(), WebDAVError> {
        let client = self.client()?;
        let url = format!("{}/.file", folder_url);
        let mtime_secs = mtime.floor() as i64;

        let resp = client
            .request(reqwest::Method::from_bytes(b"MOVE").unwrap(), &url)
            .header("Authorization", self.auth_header())
            .header("Destination", destination)
            .header("OC-Total-Length", size.to_string())
            .header("X-OC-Mtime", mtime_secs.to_string())
            .timeout(self.upload_timeout(size))
            .send()
            .map_err(|e| WebDAVError::Http(e.to_string()))?;

        match resp.status().as_u16() {
            201 | 204 => Ok(()),
            202 => {
                let location = resp
                    .headers()
                    .get("OC-JobStatus-Location")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                match location {
                    Some(loc) => self.poll_job(&loc),
                    None => Err(WebDAVError::Http(
                        "MOVE returned 202 without OC-JobStatus-Location".to_string(),
                    )),
                }
            }
            status => Err(WebDAVError::Http(format!("MOVE returned status {}", status))),
        }
    }

    /// Poll an asynchronous Nextcloud job until it finishes.
    fn poll_job(&self, location: &str) -> Result<(), WebDAVError> {
        let client = self.client()?;
        for _ in 0..60 {
            let resp = client
                .get(location)
                .header("Authorization", self.auth_header())
                .send()
                .map_err(|e| WebDAVError::Http(e.to_string()))?;

            if !resp.status().is_success() {
                return Err(WebDAVError::Http(format!(
                    "Job status returned status {}",
                    resp.status().as_u16()
                )));
            }

            let json: serde_json::Value = resp
                .json()
                .map_err(|e| WebDAVError::Parse(e.to_string()))?;

            match json["status"].as_str() {
                Some("finished") => return Ok(()),
                Some("init") | Some("started") => {
                    std::thread::sleep(Duration::from_secs(2));
                }
                _ => {
                    let message = json["errorMessage"].as_str().unwrap_or("unknown error");
                    return Err(WebDAVError::Http(format!("Server job failed: {}", message)));
                }
            }
        }
        Err(WebDAVError::Http(
            "Timed out waiting for server job".to_string(),
        ))
    }

    /// Delete a resource by its full URL. 404 is treated as success.
    fn delete_url(&self, url: &str) -> Result<(), WebDAVError> {
        let client = self.client()?;
        let resp = client
            .delete(url)
            .header("Authorization", self.auth_header())
            .send()
            .map_err(|e| WebDAVError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        if status == 200 || status == 204 || status == 404 {
            Ok(())
        } else {
            Err(WebDAVError::Http(format!("DELETE returned status {}", status)))
        }
    }

    fn uploads_url(&self, transfer_id: u32) -> String {
        format!(
            "{}/remote.php/dav/uploads/{}/{}",
            self.server_url,
            encode_path(&self.username),
            transfer_id
        )
    }

    fn chunk_url(&self, transfer_id: u32, index: u64) -> String {
        format!("{}/{:05}", self.uploads_url(transfer_id), index)
    }

    /// Timeout for an upload request, scaled by payload size.
    fn upload_timeout(&self, bytes: u64) -> Duration {
        let seconds = (bytes / (1024 * 1024)) * 10 + 60;
        Duration::from_secs(seconds.clamp(60, 1800))
    }
}

/// Deterministic transfer id for a file, so interrupted uploads can be resumed.
fn transfer_id_for(remote_path: &str, mtime: f64, size: u64) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    fn mix(hash: &mut u32, bytes: &[u8]) {
        for byte in bytes {
            *hash ^= *byte as u32;
            *hash = hash.wrapping_mul(0x0100_0193);
        }
    }
    mix(&mut hash, remote_path.as_bytes());
    mix(&mut hash, &mtime.to_bits().to_le_bytes());
    mix(&mut hash, &size.to_le_bytes());
    hash
}

/// URL-encode path segments individually (preserve /)
fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|segment| urlencoding_encode(segment))
        .collect::<Vec<_>>()
        .join("/")
}

/// Simple percent-encoding for URL path segments
fn urlencoding_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

/// Simple percent-decoding
fn urldecoding_decode(s: &str) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                &s[i + 1..i + 3], 16
            ) {
                result.push(byte);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).to_string()
}

/// Parse a PROPFIND multi-status XML response
fn parse_propfind_response(xml: &str, request_path: &str) -> Result<Vec<RemoteItem>, WebDAVError> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| WebDAVError::Parse(e.to_string()))?;

    let mut items = Vec::new();
    let request_path_clean = request_path.trim_matches('/');

    for response in doc.descendants().filter(|n| n.has_tag_name("response")) {
        let href = response.descendants()
            .find(|n| n.has_tag_name("href"))
            .and_then(|n| n.text())
            .unwrap_or("");

        // Decode the href and extract the path after /remote.php/dav/files/USERNAME/
        let decoded_href = urldecoding_decode(href);
        let item_path = if let Some(pos) = decoded_href.find("/remote.php/dav/files/") {
            let after = &decoded_href[pos + "/remote.php/dav/files/".len()..];
            // Skip username segment
            if let Some(slash_pos) = after.find('/') {
                after[slash_pos..].trim_matches('/').to_string()
            } else {
                String::new()
            }
        } else {
            decoded_href.trim_matches('/').to_string()
        };

        // Skip the directory itself (the request path)
        if item_path == request_path_clean || item_path.is_empty() {
            continue;
        }

        let is_dir = response.descendants()
            .any(|n| n.has_tag_name("collection"));

        let size: u64 = response.descendants()
            .find(|n| n.has_tag_name("getcontentlength"))
            .and_then(|n| n.text())
            .and_then(|t| t.parse().ok())
            .unwrap_or(0);

        // Extract just the name (last segment)
        let name = item_path.trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(&item_path)
            .to_string();

        if !name.is_empty() {
            items.push(RemoteItem { name, is_dir, size });
        }
    }

    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_client() -> Option<WebDAVClient> {
        let url = std::env::var("SIMPLESYNC_TEST_URL").ok()?;
        let user = std::env::var("SIMPLESYNC_TEST_USER").ok()?;
        let pass = std::env::var("SIMPLESYNC_TEST_PASS").ok()?;
        Some(WebDAVClient::new(&url, &user, &pass))
    }

    fn payload(size: usize) -> Vec<u8> {
        (0..size).map(|i| (i % 251) as u8).collect()
    }

    const CHUNK: u64 = 5 * 1024 * 1024;
    const MTIME: f64 = 1_700_000_000.0;

    /// End-to-end chunked upload against a real server. Ignored by default.
    ///
    /// Run with:
    ///   SIMPLESYNC_TEST_URL=... SIMPLESYNC_TEST_USER=... SIMPLESYNC_TEST_PASS=... \
    ///     cargo test -- --ignored --nocapture chunked_upload
    #[test]
    #[ignore = "requires a live Nextcloud server"]
    fn chunked_upload_roundtrip() {
        let Some(client) = env_client() else {
            eprintln!("skipping: set SIMPLESYNC_TEST_URL/USER/PASS");
            return;
        };
        assert!(
            client.upload_capabilities().chunking_ng,
            "server does not advertise chunking NG"
        );

        let dir = std::env::temp_dir().join(format!("simplesync-chunk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let local = dir.join("payload.bin");
        let data = payload(12 * 1024 * 1024 + 12345);
        std::fs::write(&local, &data).unwrap();

        let remote_dir = "/simplesync-chunk-test";
        let remote = format!("{}/payload-{}.bin", remote_dir, std::process::id());
        client.create_directory(remote_dir).ok();

        let cancel = AtomicBool::new(false);
        let mut last_done = 0u64;
        let result = client.upload_file_chunked(
            local.to_str().unwrap(),
            &remote,
            MTIME,
            CHUNK,
            &cancel,
            &mut |done, _total| last_done = done,
        );
        assert!(result.is_ok(), "chunked upload failed: {:?}", result.err());
        assert_eq!(last_done as usize, data.len());

        let out = dir.join("downloaded.bin");
        client
            .download_file(&remote, out.to_str().unwrap())
            .expect("download failed");
        assert_eq!(std::fs::read(&out).unwrap(), data);

        client.delete(&remote).ok();
        client.delete(remote_dir).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Upload one chunk by hand, then verify `upload_file_chunked` resumes
    /// instead of re-sending everything.
    #[test]
    #[ignore = "requires a live Nextcloud server"]
    fn chunked_upload_resume() {
        let Some(client) = env_client() else {
            eprintln!("skipping: set SIMPLESYNC_TEST_URL/USER/PASS");
            return;
        };
        assert!(
            client.upload_capabilities().chunking_ng,
            "server does not advertise chunking NG"
        );

        let dir = std::env::temp_dir().join(format!("simplesync-resume-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let local = dir.join("payload.bin");
        let data = payload(12 * 1024 * 1024 + 12345);
        std::fs::write(&local, &data).unwrap();

        let remote_dir = "/simplesync-chunk-test";
        let remote = format!("{}/resume-{}.bin", remote_dir, std::process::id());
        client.create_directory(remote_dir).ok();

        let path = remote.trim_start_matches('/');
        let size = data.len() as u64;
        let transfer_id = transfer_id_for(path, MTIME, size);
        let folder = client.uploads_url(transfer_id);
        let destination = format!("{}/{}", client.base_url(), encode_path(path));

        // Pretend a previous run uploaded the first chunk and then died.
        client.create_upload_folder(&folder, &destination, size).unwrap();
        let raw = client.client().unwrap();
        let resp = raw
            .put(client.chunk_url(transfer_id, 1))
            .header("Authorization", client.auth_header())
            .header("OC-Chunk-Offset", "0")
            .header("OC-Total-Length", size.to_string())
            .header("Destination", &destination)
            .body(data[..CHUNK as usize].to_vec())
            .send()
            .unwrap();
        assert!(resp.status().is_success(), "seed chunk failed");

        // Now run the real upload; it must start from CHUNK, not from 0.
        let cancel = AtomicBool::new(false);
        let mut first_reported = None;
        let result = client.upload_file_chunked(
            local.to_str().unwrap(),
            &remote,
            MTIME,
            CHUNK,
            &cancel,
            &mut |done, _total| {
                if first_reported.is_none() {
                    first_reported = Some(done);
                }
            },
        );
        assert!(result.is_ok(), "resumed upload failed: {:?}", result.err());
        assert_eq!(first_reported, Some(CHUNK), "did not resume from the first chunk");

        let out = dir.join("downloaded.bin");
        client
            .download_file(&remote, out.to_str().unwrap())
            .expect("download failed");
        assert_eq!(std::fs::read(&out).unwrap(), data);

        client.delete(&remote).ok();
        client.delete(remote_dir).ok();
        std::fs::remove_dir_all(&dir).ok();
    }
}
