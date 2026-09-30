//! Bare Git document storage module.
//!
//! Provides bare repository management on dedicated branch `refs/heads/specs`,
//! channel-isolated background write actor with panic recovery, and lock-free
//! concurrent ODB blob read access.

pub mod actor;
pub mod reader;

use std::fmt;
use std::path::{Path, PathBuf};
use tokio::sync::{mpsc, oneshot};

pub use actor::run_actor_supervisor;
pub use reader::GitReadHandle;

/// Dedicated Git reference for specifications branch.
pub const SPECS_BRANCH_REF: &str = "refs/heads/specs";

/// Default capacity for the actor command channel.
pub const DEFAULT_GIT_CHANNEL_BOUND: usize = 256;

/// Commands accepted by the dedicated Git write actor.
#[derive(Debug)]
pub enum GitCommand {
    /// Commit a document into the bare repository at `doc_path`.
    CommitDocument {
        /// Relative persistent document path (e.g. `specs/vision.md`).
        doc_path: String,
        /// Raw document text to commit.
        content: String,
        /// Channel to return the commit result or error.
        respond_to: oneshot::Sender<Result<GitCommitResult, GitError>>,
    },
}

/// Metadata describing a successful Git document commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommitResult {
    /// Hexadecimal SHA-1 hash of the resulting commit object.
    pub commit_id: String,
    /// Hexadecimal SHA-1 hash of the committed blob object.
    pub blob_hash: String,
    /// Normalized document path where the blob was placed.
    pub doc_path: String,
}

/// Handle for dispatching write operations to the dedicated Git write actor.
#[derive(Clone, Debug)]
pub struct GitWriteHandle {
    sender: mpsc::Sender<GitCommand>,
}

impl GitWriteHandle {
    /// Creates a new `GitWriteHandle` wrapping an mpsc sender.
    pub fn new(sender: mpsc::Sender<GitCommand>) -> Self {
        Self { sender }
    }

    /// Commits a document at `doc_path` with `content` onto `refs/heads/specs`.
    ///
    /// # Errors
    ///
    /// Returns `GitError` if the actor channel is closed, the actor panics,
    /// or libgit2 encounters an error writing the blob, tree, or commit.
    pub async fn commit_document(
        &self,
        doc_path: &str,
        content: &str,
    ) -> Result<GitCommitResult, GitError> {
        let (respond_to, rx) = oneshot::channel();
        self.sender
            .send(GitCommand::CommitDocument {
                doc_path: doc_path.to_string(),
                content: content.to_string(),
                respond_to,
            })
            .await
            .map_err(|_| GitError::ChannelClosed)?;

        rx.await.map_err(|_| GitError::ChannelClosed)?
    }
}

/// Errors originating from Git storage operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// Bare repository path was not found or could not be accessed.
    RepoNotFound(String),
    /// libgit2 underlying error or failure.
    Git2(String),
    /// Actor channel was closed or dropped.
    ChannelClosed,
    /// The Git write actor thread panicked while executing a command.
    ActorPanicked(String),
    /// Requested byte span was outside the boundaries of the target blob.
    InvalidByteRange {
        /// Requested starting byte offset.
        start: usize,
        /// Requested ending byte offset.
        end: usize,
        /// Total byte length of the blob.
        total: usize,
    },
    /// Extracted byte span or blob content is not valid UTF-8.
    InvalidUtf8(String),
    /// Document path is malformed or invalid.
    InvalidPath(String),
    /// Blob or commit object not found in the repository ODB.
    ObjectNotFound(String),
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RepoNotFound(msg) => write!(f, "Git repository not found: {msg}"),
            Self::Git2(msg) => write!(f, "libgit2 error: {msg}"),
            Self::ChannelClosed => write!(f, "Git actor channel closed"),
            Self::ActorPanicked(msg) => write!(f, "Git write actor panicked: {msg}"),
            Self::InvalidByteRange { start, end, total } => {
                write!(
                    f,
                    "Invalid byte range [{start}..{end}] for blob with length {total}"
                )
            }
            Self::InvalidUtf8(msg) => write!(f, "Invalid UTF-8: {msg}"),
            Self::InvalidPath(msg) => write!(f, "Invalid document path: {msg}"),
            Self::ObjectNotFound(msg) => write!(f, "Git object not found: {msg}"),
        }
    }
}

impl std::error::Error for GitError {}

/// Resolves the Git repository directory from environment or devcontainer defaults.
///
/// Precedence:
/// 1. `TKS_GIT_DIR` environment variable
/// 2. `/git/tks.git` if parent directory `/git` exists (devcontainer volume)
/// 3. `target/git/tks.git` fallback for local execution and tests
pub fn resolve_git_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("TKS_GIT_DIR") {
        return PathBuf::from(dir);
    }

    let container_git = PathBuf::from("/git/tks.git");
    if container_git.parent().map(|p| p.is_dir()).unwrap_or(false) {
        container_git
    } else {
        PathBuf::from("target/git/tks.git")
    }
}

/// Normalizes and validates a document path.
///
/// Ensures the path is non-empty, strips leading slashes or `./`, and rejects
/// directory traversal segments (`..`).
pub fn normalize_doc_path(path: &str) -> Result<String, GitError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(GitError::InvalidPath("Path cannot be empty".to_string()));
    }

    let stripped = trimmed
        .trim_start_matches('/')
        .trim_start_matches("./")
        .trim();

    if stripped.is_empty() {
        return Err(GitError::InvalidPath("Path cannot be empty".to_string()));
    }

    for component in stripped.split('/') {
        if component == ".." {
            return Err(GitError::InvalidPath(format!(
                "Path cannot contain directory traversal '..': {path}"
            )));
        }
    }

    Ok(stripped.to_string())
}

/// Initializes a bare Git repository idempotently, ensuring `refs/heads/specs` exists.
///
/// If the branch does not yet exist, an initial root commit with an empty tree
/// is created and `HEAD` is pointed to `refs/heads/specs`.
///
/// # Errors
///
/// Returns `GitError` if bare repository creation or opening fails.
pub fn init_bare(repo_path: impl AsRef<Path>) -> Result<git2::Repository, GitError> {
    let path = repo_path.as_ref();
    let repo = if path.exists() {
        match git2::Repository::open_bare(path) {
            Ok(r) => r,
            Err(_) => {
                git2::Repository::init_bare(path).map_err(|e| GitError::Git2(e.to_string()))?
            }
        }
    } else {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        git2::Repository::init_bare(path).map_err(|e| GitError::Git2(e.to_string()))?
    };

    // Ensure refs/heads/specs branch exists idempotently
    if repo.find_reference(SPECS_BRANCH_REF).is_err() {
        let sig = git2::Signature::now("TKS System", "system@tks.local")
            .map_err(|e| GitError::Git2(e.to_string()))?;
        let mut index = git2::Index::new().map_err(|e| GitError::Git2(e.to_string()))?;
        let tree_id = index
            .write_tree_to(&repo)
            .map_err(|e| GitError::Git2(e.to_string()))?;
        let tree = repo
            .find_tree(tree_id)
            .map_err(|e| GitError::Git2(e.to_string()))?;

        repo.commit(
            Some(SPECS_BRANCH_REF),
            &sig,
            &sig,
            "Initial commit on refs/heads/specs",
            &tree,
            &[],
        )
        .map_err(|e| GitError::Git2(e.to_string()))?;

        let _ = repo.set_head(SPECS_BRANCH_REF);
    }

    Ok(repo)
}

/// Starts the dedicated background Git write actor thread and returns write and read handles.
///
/// # Errors
///
/// Returns `GitError` if bare repository initialization fails or supervisor thread cannot be spawned.
pub fn start_git_actor(
    repo_path: impl AsRef<Path>,
    channel_bound: usize,
) -> Result<(GitWriteHandle, GitReadHandle), GitError> {
    let repo_path = repo_path.as_ref().to_path_buf();
    init_bare(&repo_path)?;

    let (tx, rx) = mpsc::channel(channel_bound);
    let write_handle = GitWriteHandle::new(tx);
    let read_handle = GitReadHandle::new(repo_path.clone());

    std::thread::Builder::new()
        .name("git-writer-supervisor".to_string())
        .spawn(move || {
            actor::run_actor_supervisor(repo_path, rx);
        })
        .map_err(|e| GitError::Git2(format!("Failed to spawn git actor thread: {e}")))?;

    Ok((write_handle, read_handle))
}
