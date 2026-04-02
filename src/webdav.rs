use base64::Engine;

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
}

impl std::fmt::Display for WebDAVError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebDAVError::Http(msg) => write!(f, "HTTP error: {}", msg),
            WebDAVError::Parse(msg) => write!(f, "Parse error: {}", msg),
            WebDAVError::Io(msg) => write!(f, "IO error: {}", msg),
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
