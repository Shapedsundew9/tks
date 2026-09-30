//! Dedicated background Git write actor.
//!
//! Encapsulates all blocking Git write operations within a dedicated background
//! OS thread owning the writable `git2::Repository` handle and communicating over
//! bounded Tokio `mpsc` and `oneshot` channels with panic recovery.

use std::path::PathBuf;
use tokio::sync::mpsc;

use super::{GitCommand, GitCommitResult, GitError, SPECS_BRANCH_REF, normalize_doc_path};

/// Runs the git actor supervisor loop, ensuring the background thread stays alive.
pub fn run_actor_supervisor(repo_path: PathBuf, mut rx: mpsc::Receiver<GitCommand>) {
    loop {
        let repo = match git2::Repository::open_bare(&repo_path) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(
                    "Git actor supervisor failed to open repository at {:?}: {}",
                    repo_path,
                    e
                );
                // Drain any pending commands with error
                while let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        GitCommand::CommitDocument { respond_to, .. } => {
                            let _ = respond_to.send(Err(GitError::RepoNotFound(format!(
                                "Cannot open bare repository at {:?}: {}",
                                repo_path, e
                            ))));
                        }
                    }
                }
                break;
            }
        };

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            actor_event_loop(&repo, &mut rx);
        }));

        if let Err(panic_payload) = result {
            let msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "Unknown panic in git write actor thread".to_string()
            };
            tracing::error!(
                "Git write actor crashed with panic; supervisor restarting: {}",
                msg
            );
        } else {
            tracing::debug!("Git write actor channel closed cleanly. Terminating actor loop.");
            break;
        }
    }
}

/// The inner event loop processing incoming commands sequentially.
fn actor_event_loop(repo: &git2::Repository, rx: &mut mpsc::Receiver<GitCommand>) {
    while let Some(command) = rx.blocking_recv() {
        match command {
            GitCommand::CommitDocument {
                doc_path,
                content,
                respond_to,
            } => {
                let commit_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if doc_path == "__PANIC_TEST__" {
                        panic!("Simulated panic in git actor for panic recovery verification");
                    }
                    commit_document_internal(repo, &doc_path, &content)
                }));

                let response = match commit_res {
                    Ok(res) => res,
                    Err(payload) => {
                        let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                            (*s).to_string()
                        } else if let Some(s) = payload.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "Unknown panic in git write actor".to_string()
                        };
                        Err(GitError::ActorPanicked(msg))
                    }
                };

                let _ = respond_to.send(response);
            }
        }
    }
}

/// Performs the internal commit sequence using libgit2 ODB direct writes.
fn commit_document_internal(
    repo: &git2::Repository,
    doc_path: &str,
    content: &str,
) -> Result<GitCommitResult, GitError> {
    let normalized_path = normalize_doc_path(doc_path)?;

    // 1. Write the raw blob directly into the repository ODB
    let blob_oid = repo
        .blob(content.as_bytes())
        .map_err(|e| GitError::Git2(format!("Failed to write blob to ODB: {e}")))?;
    let blob_hash = blob_oid.to_string();

    // 2. Resolve current parent commit on refs/heads/specs
    let parent_commit = match repo.find_reference(SPECS_BRANCH_REF) {
        Ok(reference) => reference.peel_to_commit().ok(),
        Err(_) => None,
    };

    // 3. Build the tree in-memory using git2::Index without touching disk or .git/index
    let mut index = git2::Index::new()
        .map_err(|e| GitError::Git2(format!("Failed to create in-memory index: {e}")))?;

    if let Some(ref parent) = parent_commit {
        let parent_tree = parent
            .tree()
            .map_err(|e| GitError::Git2(format!("Failed to read parent tree: {e}")))?;
        index
            .read_tree(&parent_tree)
            .map_err(|e| GitError::Git2(format!("Failed to load parent tree into index: {e}")))?;
    }

    let entry = git2::IndexEntry {
        ctime: git2::IndexTime::new(0, 0),
        mtime: git2::IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: 0o100644,
        uid: 0,
        gid: 0,
        file_size: content.len() as u32,
        id: blob_oid,
        flags: 0,
        flags_extended: 0,
        path: normalized_path.as_bytes().to_vec(),
    };

    index.add(&entry).map_err(|e| {
        GitError::Git2(format!(
            "Failed to add entry '{normalized_path}' to index: {e}"
        ))
    })?;

    let new_tree_oid = index
        .write_tree_to(repo)
        .map_err(|e| GitError::Git2(format!("Failed to write tree to ODB: {e}")))?;
    let new_tree = repo
        .find_tree(new_tree_oid)
        .map_err(|e| GitError::Git2(format!("Failed to locate tree in ODB: {e}")))?;

    // 4. Create the commit object advancing refs/heads/specs
    let sig = git2::Signature::now("TKS System", "system@tks.local")
        .map_err(|e| GitError::Git2(format!("Failed to create signature: {e}")))?;

    let parents: Vec<&git2::Commit> = parent_commit.as_ref().into_iter().collect();

    let commit_oid = repo
        .commit(
            Some(SPECS_BRANCH_REF),
            &sig,
            &sig,
            &format!("Commit document: {normalized_path}"),
            &new_tree,
            &parents,
        )
        .map_err(|e| GitError::Git2(format!("Failed to create commit: {e}")))?;

    Ok(GitCommitResult {
        commit_id: commit_oid.to_string(),
        blob_hash,
        doc_path: normalized_path,
    })
}
