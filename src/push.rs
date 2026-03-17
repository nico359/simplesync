use crate::db::Target;
use crate::webdav::WebDAVClient;
use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc;

#[derive(Debug, Clone, Default)]
pub struct PushSummary {
    pub uploaded: u32,
    pub skipped: u32,
    pub deleted: u32,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum PushProgress {
    File {
        current_file: String,
        files_done: u32,
        files_total: u32,
    },
    Complete {
        success: bool,
        summary: PushSummary,
    },
}

/// Walk local directory and collect files with metadata
fn collect_local_files(local_path: &str) -> Result<Vec<(String, f64, i64)>, String> {
    let base = Path::new(local_path);
    if !base.is_dir() {
        return Err(format!("Local path does not exist: {}", local_path));
    }

    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(base)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let abs_path = entry.path();
            let rel_path = abs_path.strip_prefix(base)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string();

            let metadata = abs_path.metadata().map_err(|e| e.to_string())?;
            let mtime = metadata.modified()
                .map_err(|e| e.to_string())?
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            let size = metadata.len() as i64;

            files.push((rel_path, mtime, size));
        }
    }

    Ok(files)
}

/// Run a push operation for a single target on a background thread.
/// Sends progress updates via the provided sender.
pub fn run_push(
    client: WebDAVClient,
    target: Target,
    db_path: std::path::PathBuf,
    force: bool,
    sender: mpsc::Sender<PushProgress>,
) {
    std::thread::spawn(move || {
        let result = do_push(&client, &target, &db_path, force, &sender);
        match result {
            Ok(summary) => {
                let _ = sender.send(PushProgress::Complete {
                    success: summary.errors.is_empty(),
                    summary,
                });
            }
            Err(e) => {
                let _ = sender.send(PushProgress::Complete {
                    success: false,
                    summary: PushSummary {
                        errors: vec![e],
                        ..Default::default()
                    },
                });
            }
        }
    });
}

fn do_push(
    client: &WebDAVClient,
    target: &Target,
    db_path: &Path,
    force: bool,
    sender: &mpsc::Sender<PushProgress>,
) -> Result<PushSummary, String> {
    // Open a dedicated DB connection for this thread
    let conn = rusqlite::Connection::open(db_path)
        .map_err(|e| format!("DB error: {}", e))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;").ok();

    let mut summary = PushSummary::default();

    // Step 1: Collect local files
    let local_files = collect_local_files(&target.local_path)?;
    let _files_total = local_files.len() as u32;

    if force {
        conn.execute("DELETE FROM file_state WHERE target_id = ?1",
            rusqlite::params![target.id]).ok();
    }

    // Step 2: Filter files that need uploading (changed since last push)
    let mut to_upload: Vec<(String, f64, i64)> = Vec::new();
    for (rel_path, mtime, size) in &local_files {
        let existing: Option<(f64, i64)> = conn.query_row(
            "SELECT mtime, size FROM file_state WHERE target_id = ?1 AND rel_path = ?2",
            rusqlite::params![target.id, rel_path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).ok();

        if let Some((old_mtime, old_size)) = existing {
            if (old_mtime - mtime).abs() < 0.001 && old_size == *size {
                summary.skipped += 1;
                continue;
            }
        }
        to_upload.push((rel_path.clone(), *mtime, *size));
    }

    // Step 3: Create remote directories
    let mut dirs: Vec<String> = to_upload.iter()
        .filter_map(|(rel, _, _)| {
            Path::new(rel).parent().map(|p| p.to_string_lossy().to_string())
        })
        .filter(|d| !d.is_empty())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    dirs.sort();

    for dir in &dirs {
        let remote_dir = format!("{}/{}", target.remote_path.trim_end_matches('/'), dir);
        // Create parent directories from top to bottom
        let parts: Vec<&str> = remote_dir.trim_start_matches('/').split('/').collect();
        let mut path_so_far = String::new();
        for part in parts {
            path_so_far = if path_so_far.is_empty() {
                part.to_string()
            } else {
                format!("{}/{}", path_so_far, part)
            };
            if let Err(e) = client.create_directory(&path_so_far) {
                summary.errors.push(format!("mkdir {}: {}", path_so_far, e));
            }
        }
    }

    // Step 4: Upload files
    let upload_total = to_upload.len() as u32;
    for (i, (rel_path, mtime, size)) in to_upload.iter().enumerate() {
        let local_file = format!("{}/{}", target.local_path, rel_path);
        let remote_file = format!("{}/{}", target.remote_path.trim_end_matches('/'), rel_path);

        let _ = sender.send(PushProgress::File {
            current_file: rel_path.clone(),
            files_done: i as u32,
            files_total: upload_total,
        });

        match client.upload_file(&local_file, &remote_file) {
            Ok(()) => {
                summary.uploaded += 1;
                // Record file state
                let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
                conn.execute(
                    "INSERT INTO file_state (target_id, rel_path, mtime, size, uploaded_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(target_id, rel_path) DO UPDATE SET mtime = ?3, size = ?4, uploaded_at = ?5",
                    rusqlite::params![target.id, rel_path, mtime, size, now],
                ).ok();
            }
            Err(e) => {
                summary.errors.push(format!("{}: {}", rel_path, e));
            }
        }
    }

    // Step 5: Mirror mode - delete remote files not in local
    if target.mode == "mirror" {
        match client.list_directory_recursive(&target.remote_path) {
            Ok(remote_files) => {
                let local_set: HashSet<String> = local_files.iter()
                    .map(|(rel, _, _)| {
                        format!("{}/{}", target.remote_path.trim_end_matches('/'), rel)
                    })
                    .collect();

                for remote_file in remote_files {
                    if !local_set.contains(&remote_file) {
                        match client.delete(&remote_file) {
                            Ok(()) => summary.deleted += 1,
                            Err(e) => summary.errors.push(format!("delete {}: {}", remote_file, e)),
                        }
                    }
                }
            }
            Err(e) => {
                summary.errors.push(format!("Mirror listing failed: {}", e));
            }
        }
    }

    // Step 6: Update last_push
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    conn.execute(
        "UPDATE targets SET last_push = ?1 WHERE id = ?2",
        rusqlite::params![now, target.id],
    ).ok();

    Ok(summary)
}
