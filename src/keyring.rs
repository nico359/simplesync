//! Credential storage.
//!
//! Uses [`oo7`], which automatically selects the right backend:
//!
//! - inside a sandbox (Flatpak) it derives a key from the
//!   `org.freedesktop.portal.Secret` portal and keeps an encrypted keyring file
//!   in the app's private data directory. This does **not** require access to
//!   `org.freedesktop.secrets`;
//! - outside a sandbox it falls back to the host Secret Service (D-Bus).

use std::future::Future;

const APP_ID: &str = "io.github.nico359.simplesync";

#[derive(Debug, Clone)]
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

pub fn store_credentials_sync(creds: &Credentials) -> bool {
    block_on(async {
        let keyring = oo7::Keyring::new().await?;
        keyring.unlock().await?;
        store_one(&keyring, "server_url", &creds.server_url).await?;
        store_one(&keyring, "username", &creds.username).await?;
        store_one(&keyring, "app_password", &creds.app_password).await?;
        Ok::<(), oo7::Error>(())
    })
    .is_ok()
}

pub fn load_credentials_sync() -> Option<Credentials> {
    block_on(async {
        let keyring = oo7::Keyring::new().await.ok()?;
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
}

pub fn clear_credentials_sync() -> bool {
    block_on(async {
        let keyring = oo7::Keyring::new().await?;
        keyring.unlock().await?;
        keyring.delete(&[("application", APP_ID)]).await
    })
    .is_ok()
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
