use crate::db::Target;
use crate::webdav::WebDAVClient;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

#[derive(Debug, Clone, Default)]
pub struct PullSummary {
    pub downloaded: u32,
    pub skipped: u32,
    pub errors: Vec<String>,
    pub cancelled: bool,
}

#[derive(Debug, Clone)]
pub enum PullProgress {
    File {
        current_file: String,
        files_done: u32,
        files_total: u32,
    },
    Complete {
        #[allow(dead_code)]
        success: bool,
        summary: PullSummary,
    },
}

/// Run a pull operation for a single target on a background thread.
pub fn run_pull(
    client: WebDAVClient,
    target: Target,
    cancel: Arc<AtomicBool>,
    sender: mpsc::Sender<PullProgress>,
) {
    std::thread::spawn(move || {
        let result = do_pull(&client, &target, &cancel, &sender);
        match result {
            Ok(summary) => {
                let _ = sender.send(PullProgress::Complete {
                    success: summary.errors.is_empty() && !summary.cancelled,
                    summary,
                });
            }
            Err(e) => {
                let _ = sender.send(PullProgress::Complete {
                    success: false,
                    summary: PullSummary {
                        errors: vec![e],
                        ..Default::default()
                    },
                });
            }
        }
    });
}

fn do_pull(
    client: &WebDAVClient,
    target: &Target,
    cancel: &AtomicBool,
    sender: &mpsc::Sender<PullProgress>,
) -> Result<PullSummary, String> {
    let mut summary = PullSummary::default();

    if cancel.load(Ordering::Relaxed) {
        summary.cancelled = true;
        return Ok(summary);
    }

    let remote_files = client.list_directory_recursive(&target.remote_path)
        .map_err(|e| format!("Failed to list remote files: {}", e))?;

    let remote_base = target.remote_path.trim_end_matches('/');
    let files_total = remote_files.len() as u32;

    for (i, remote_file) in remote_files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }

        let rel_path = remote_file.trim_start_matches('/')
            .strip_prefix(remote_base.trim_start_matches('/'))
            .unwrap_or(remote_file)
            .trim_start_matches('/');

        if rel_path.is_empty() {
            continue;
        }

        let local_file = format!("{}/{}", target.local_path, rel_path);

        if Path::new(&local_file).exists() {
            summary.skipped += 1;
            continue;
        }

        let _ = sender.send(PullProgress::File {
            current_file: rel_path.to_string(),
            files_done: i as u32,
            files_total,
        });

        match client.download_file(remote_file.trim_start_matches('/'), &local_file) {
            Ok(()) => {
                summary.downloaded += 1;
            }
            Err(e) => {
                summary.errors.push(format!("{}: {}", rel_path, e));
            }
        }
    }

    Ok(summary)
}
