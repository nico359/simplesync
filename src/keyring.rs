//! Credential storage.
//!
//! Uses [`oo7`], which automatically selects the right backend:
//!
//! - inside a sandbox (Flatpak) it derives a key from the
//!   `org.freedesktop.portal.Secret` portal and keeps an encrypted keyring file
//!   in the app's private data directory. This does **not** require access to
//!   `org.freedesktop.secrets`;
//! - outside a sandbox it falls back to the host Secret Service (D-Bus).
//!
//! Some sandbox backends (notably KDE without KWallet portal support) do not
//! implement the Secret portal. In that case we store the credentials in a
//! `0600` file inside the app's private data directory so login still works
//! without the broad secrets-bus permission.

use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const APP_ID: &str = "io.github.nico359.simplesync";

/// Upper bound for a keyring operation. A misbehaving portal or keyring backend
/// must never hang the app (or the login flow), so after this we fall back to
/// the local file store.
const KEYRING_TIMEOUT: Duration = Duration::from_secs(6);

/// Set once a keyring operation has timed out, so we don't pay the timeout on
/// every subsequent call.
static KEYRING_USABLE: AtomicBool = AtomicBool::new(true);

/// Cached per-app secret from the Secret portal.
///
/// `oo7`/`ashpd` hang on a second `RetrieveSecret` call in the same process
/// (the first succeeds, subsequent ones never return), so we retrieve it once
/// and reuse it.
static PORTAL_SECRET: std::sync::Mutex<Option<oo7::Secret>> = std::sync::Mutex::new(None);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Credentials {
    pub server_url: String,
    pub username: String,
    pub app_password: String,
}

/// Run an async future to completion on a small single-threaded runtime.
///
/// Secret operations happen rarely, so building the runtime on demand is fine
/// and avoids keeping one alive for the lifetime of the app.
fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build async runtime")
        .block_on(future)
}

/// Open the oo7 keyring.
///
/// When sandboxed we use the Secret portal explicitly instead of
/// [`oo7::Keyring::new`], which would fall back to the host Secret Service.
/// The sandbox is not allowed to reach that service (we deliberately do not
/// request `--talk-name=org.freedesktop.secrets`), and the blocked D-Bus call
/// can hang indefinitely.
async fn open_keyring() -> Option<oo7::Keyring> {
    if !KEYRING_USABLE.load(Ordering::Relaxed) {
        return None;
    }

    if !oo7::ashpd::is_sandboxed() {
        return oo7::Keyring::host().await.ok();
    }

    let secret = portal_secret().await?;

    let path = keyring_path();
    match oo7::Keyring::sandboxed_with_path(&path, secret.clone()).await {
        Ok(keyring) => Some(keyring),
        Err(oo7::Error::File(error)) if matches!(*error, oo7::file::Error::IncorrectSecret) => {
            // The keyring file was encrypted with a different secret (for
            // example after the login keyring was reset). It cannot be
            // recovered, so discard it and create a fresh one for this app.
            let _ = std::fs::remove_file(&path);
            oo7::Keyring::sandboxed_with_path(&path, secret).await.ok()
        }
        Err(_) => None,
    }
}

/// Retrieve the Secret portal's per-app secret, at most once per process.
///
/// The lock is intentionally held across the `await` so concurrent callers wait
/// for the first retrieval instead of issuing a second portal request (which
/// hangs).
async fn portal_secret() -> Option<oo7::Secret> {
    let mut guard = PORTAL_SECRET.lock().unwrap();
    if let Some(secret) = guard.as_ref() {
        return Some(secret.clone());
    }

    let secret = oo7::Secret::sandboxed().await.ok()?;
    *guard = Some(secret.clone());
    Some(secret)
}

/// Path of the app's own encrypted keyring file, under the private data dir.
fn keyring_path() -> PathBuf {
    gtk::glib::user_data_dir()
        .join("simplesync")
        .join("simplesync.keyring")
}

async fn store_one(keyring: &oo7::Keyring, kind: &str, value: &str) -> oo7::Result<()> {
    keyring
        .create_item(
            &format!("SimpleSync {}", kind),
            &[("application", APP_ID), ("type", kind)],
            value,
            true,
        )
        .await
}

async fn load_one(keyring: &oo7::Keyring, kind: &str) -> Option<String> {
    let items = keyring
        .search_items(&[("application", APP_ID), ("type", kind)])
        .await
        .ok()?;
    let item = items.into_iter().next()?;
    item.unlock().await.ok()?;
    let secret = item.secret().await.ok()?;
    let value = match secret.as_str() {
        Some(text) => text.to_owned(),
        None => String::from_utf8_lossy(secret.as_bytes()).into_owned(),
    };
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

// --- Fallback file store, used when no Secret portal backend is available. ---

fn fallback_path() -> PathBuf {
    gtk::glib::user_data_dir()
        .join("simplesync")
        .join("credentials.json")
}

fn store_fallback(creds: &Credentials) -> bool {
    let path = fallback_path();
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    let data = match serde_json::to_vec(creds) {
        Ok(data) => data,
        Err(_) => return false,
    };
    if std::fs::write(&path, data).is_err() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    true
}

fn load_fallback() -> Option<Credentials> {
    let data = std::fs::read(fallback_path()).ok()?;
    serde_json::from_slice(&data).ok()
}

fn clear_fallback() -> bool {
    match std::fs::remove_file(fallback_path()) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

/// Run a fallible blocking operation on its own thread, giving up after
/// [`KEYRING_TIMEOUT`]. This keeps a hung backend from stalling the caller (and
/// in particular the login flow).
fn run_with_timeout<T, F>(operation: F) -> Option<T>
where
    F: FnOnce() -> Option<T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(operation());
    });
    match rx.recv_timeout(KEYRING_TIMEOUT) {
        Ok(result) => result,
        Err(_) => {
            KEYRING_USABLE.store(false, Ordering::Relaxed);
            None
        }
    }
}

/// How credentials ended up being stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOutcome {
    /// Stored in the encrypted keyring.
    Keyring,
    /// Stored in the plaintext fallback because no keyring was available.
    Plaintext,
    /// Could not store the credentials at all.
    Failed,
}

pub fn store_credentials_sync(creds: &Credentials) -> StoreOutcome {
    let keyring_creds = creds.clone();
    let stored = run_with_timeout(move || {
        block_on(async {
            let keyring = open_keyring().await?;
            keyring.unlock().await.ok()?;
            store_one(&keyring, "server_url", &keyring_creds.server_url).await.ok()?;
            store_one(&keyring, "username", &keyring_creds.username).await.ok()?;
            store_one(&keyring, "app_password", &keyring_creds.app_password)
                .await
                .ok()?;
            Some(())
        })
    })
    .is_some();

    if stored {
        // The keyring now holds the credentials; drop any plaintext fallback.
        let _ = clear_fallback();
        StoreOutcome::Keyring
    } else if store_fallback(creds) {
        StoreOutcome::Plaintext
    } else {
        StoreOutcome::Failed
    }
}

pub fn load_credentials_sync() -> Option<Credentials> {
    let from_keyring = run_with_timeout(|| {
        block_on(async {
            let keyring = open_keyring().await?;
            keyring.unlock().await.ok()?;

            let server_url = load_one(&keyring, "server_url").await?;
            let username = load_one(&keyring, "username").await?;
            let app_password = load_one(&keyring, "app_password").await?;

            if server_url.is_empty() || username.is_empty() || app_password.is_empty() {
                return None;
            }

            Some(Credentials {
                server_url,
                username,
                app_password,
            })
        })
    });

    if let Some(creds) = from_keyring {
        // The keyring is authoritative; make sure no plaintext copy lingers.
        let _ = clear_fallback();
        return Some(creds);
    }

    let fallback = load_fallback();
    if let Some(creds) = &fallback {
        // The keyring is available now; move the plaintext fallback over.
        if KEYRING_USABLE.load(Ordering::Relaxed) {
            let _ = store_credentials_sync(creds);
        }
    }
    fallback
}

pub fn clear_credentials_sync() -> bool {
    let cleared_keyring = run_with_timeout(|| {
        block_on(async {
            let keyring = open_keyring().await?;
            keyring.unlock().await.ok()?;
            keyring.delete(&[("application", APP_ID)]).await.ok()
        })
    })
    .is_some();

    cleared_keyring || clear_fallback()
}

#[allow(dead_code)]
pub fn has_credentials() -> bool {
    load_credentials_sync().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the same store/lookup logic the app uses, but against the
    /// file backend so it runs without a portal or a running Secret Service.
    #[test]
    fn file_backend_roundtrip() {
        let path =
            std::env::temp_dir().join(format!("simplesync-keyring-{}.keyring", std::process::id()));
        let secret = oo7::Secret::random().expect("failed to generate a test secret");

        block_on(async {
            let keyring = oo7::Keyring::sandboxed_with_path(&path, secret)
                .await
                .expect("failed to open file keyring");
            keyring.unlock().await.expect("failed to unlock");

            store_one(&keyring, "server_url", "https://example.com")
                .await
                .unwrap();
            store_one(&keyring, "username", "alice").await.unwrap();
            store_one(&keyring, "app_password", "hunter2").await.unwrap();

            assert_eq!(
                load_one(&keyring, "server_url").await.as_deref(),
                Some("https://example.com")
            );
            assert_eq!(load_one(&keyring, "username").await.as_deref(), Some("alice"));
            assert_eq!(
                load_one(&keyring, "app_password").await.as_deref(),
                Some("hunter2")
            );
        });

        let _ = std::fs::remove_file(&path);
    }
}
