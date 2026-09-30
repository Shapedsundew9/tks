//! Concurrent read-only ODB reader.
//!
//! Provides lock-free blob lookups and zero-copy byte slicing wrapped in
//! `tokio::task::spawn_blocking` without queuing on the Git write actor.

use std::path::{Path, PathBuf};

use super::GitError;

/// Handle for concurrent read-only Git ODB lookups.
#[derive(Clone, Debug)]
pub struct GitReadHandle {
    pub(crate) repo_path: PathBuf,
}

impl GitReadHandle {
    /// Creates a new `GitReadHandle` targeting the specified repository directory.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Returns the target repository path.
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    /// Extracts a verbatim byte slice from a Git blob identified by content hash.
    ///
    /// Alias for `read_blob_span`.
    ///
    /// # Errors
    ///
    /// Returns `GitError` if blob is not found, range is out of bounds, or slice is invalid UTF-8.
    pub async fn get_document_span(
        &self,
        blob_hash: &str,
        byte_start: usize,
        byte_end: usize,
    ) -> Result<String, GitError> {
        self.read_blob_span(blob_hash, byte_start, byte_end).await
    }

    /// Extracts a verbatim byte slice from a Git blob identified by content hash.
    ///
    /// # Errors
    ///
    /// Returns `GitError` if blob is not found, range is out of bounds, or slice is invalid UTF-8.
    pub async fn read_blob_span(
        &self,
        blob_hash: &str,
        byte_start: usize,
        byte_end: usize,
    ) -> Result<String, GitError> {
        let repo_path = self.repo_path.clone();
        let hash = blob_hash.to_string();

        tokio::task::spawn_blocking(move || {
            read_blob_span_sync(&repo_path, &hash, byte_start, byte_end)
        })
        .await
        .map_err(|e| GitError::Git2(format!("spawn_blocking join error: {e}")))?
    }

    /// Reads the entire contents of a Git blob as a UTF-8 string.
    ///
    /// # Errors
    ///
    /// Returns `GitError` if blob is not found or contents are invalid UTF-8.
    pub async fn read_blob(&self, blob_hash: &str) -> Result<String, GitError> {
        let repo_path = self.repo_path.clone();
        let hash = blob_hash.to_string();

        tokio::task::spawn_blocking(move || read_blob_sync(&repo_path, &hash))
            .await
            .map_err(|e| GitError::Git2(format!("spawn_blocking join error: {e}")))?
    }
}

/// Reads the entire blob synchronously from the ODB.
fn read_blob_sync(repo_path: &Path, blob_hash: &str) -> Result<String, GitError> {
    let bytes = read_blob_bytes_sync(repo_path, blob_hash)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|e| GitError::InvalidUtf8(format!("Invalid UTF-8 in blob '{blob_hash}': {e}")))?;
    Ok(text.to_string())
}

/// Reads a byte span synchronously from the ODB.
fn read_blob_span_sync(
    repo_path: &Path,
    blob_hash: &str,
    byte_start: usize,
    byte_end: usize,
) -> Result<String, GitError> {
    let bytes = read_blob_bytes_sync(repo_path, blob_hash)?;
    let total = bytes.len();

    if byte_start > byte_end || byte_end > total {
        return Err(GitError::InvalidByteRange {
            start: byte_start,
            end: byte_end,
            total,
        });
    }

    let slice = &bytes[byte_start..byte_end];
    let text = std::str::from_utf8(slice).map_err(|e| {
        GitError::InvalidUtf8(format!(
            "Invalid UTF-8 byte span [{byte_start}..{byte_end}] in blob '{blob_hash}': {e}"
        ))
    })?;

    Ok(text.to_string())
}

/// Reads raw bytes for a blob directly from the ODB without repository write locking.
fn read_blob_bytes_sync(repo_path: &Path, blob_hash: &str) -> Result<Vec<u8>, GitError> {
    let oid = git2::Oid::from_str(blob_hash)
        .map_err(|e| GitError::ObjectNotFound(format!("Invalid blob OID '{blob_hash}': {e}")))?;

    let repo = git2::Repository::open_bare(repo_path).map_err(|e| {
        GitError::RepoNotFound(format!(
            "Failed to open bare repository at {:?}: {e}",
            repo_path
        ))
    })?;

    let odb = repo
        .odb()
        .map_err(|e| GitError::Git2(format!("Failed to acquire ODB handle: {e}")))?;

    let obj = odb.read(oid).map_err(|e| {
        GitError::ObjectNotFound(format!("Blob '{blob_hash}' not found in ODB: {e}"))
    })?;

    if obj.kind() != git2::ObjectType::Blob {
        return Err(GitError::ObjectNotFound(format!(
            "Object '{blob_hash}' is not a blob (kind: {:?})",
            obj.kind()
        )));
    }

    Ok(obj.data().to_vec())
}
