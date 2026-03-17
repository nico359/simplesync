const SCHEMA_NAME: &str = "io.github.nico359.simplesync";

#[derive(Debug, Clone)]
pub struct Credentials {
    pub server_url: String,
    pub username: String,
    pub app_password: String,
}

pub fn store_credentials_sync(creds: &Credentials) -> bool {
    store_secret("server_url", &creds.server_url)
        && store_secret("username", &creds.username)
        && store_secret("app_password", &creds.app_password)
}

pub fn load_credentials_sync() -> Option<Credentials> {
    let server = load_secret("server_url")?;
    let user = load_secret("username")?;
    let pass = load_secret("app_password")?;

    if server.is_empty() || user.is_empty() || pass.is_empty() {
        return None;
    }

    Some(Credentials {
        server_url: server,
        username: user,
        app_password: pass,
    })
}

pub fn clear_credentials_sync() -> bool {
    clear_secret("server_url")
        && clear_secret("username")
        && clear_secret("app_password")
}

#[allow(dead_code)]
pub fn has_credentials() -> bool {
    load_secret("server_url").is_some()
        && load_secret("username").is_some()
        && load_secret("app_password").is_some()
}

// --- libsecret via secret-tool subprocess ---

fn store_secret(attribute: &str, value: &str) -> bool {
    use std::process::{Command, Stdio};
    use std::io::Write;

    let result = Command::new("secret-tool")
        .args(["store", "--label", &format!("SimpleSync {}", attribute),
               "application", SCHEMA_NAME, "type", attribute])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut child| {
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(value.as_bytes())?;
            }
            child.wait()
        });

    result.map(|s| s.success()).unwrap_or(false)
}

fn load_secret(attribute: &str) -> Option<String> {
    let output = std::process::Command::new("secret-tool")
        .args(["lookup", "application", SCHEMA_NAME, "type", attribute])
        .output()
        .ok()?;

    if output.status.success() {
        let val = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if val.is_empty() { None } else { Some(val) }
    } else {
        None
    }
}

fn clear_secret(attribute: &str) -> bool {
    std::process::Command::new("secret-tool")
        .args(["clear", "application", SCHEMA_NAME, "type", attribute])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
