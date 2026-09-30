use std::process::Command;
use std::sync::Arc;
use tks::storage::git::{
    GitError, SPECS_BRANCH_REF, init_bare, normalize_doc_path, start_git_actor,
};
use uuid::Uuid;

struct TempRepoGuard {
    path: std::path::PathBuf,
}

impl TempRepoGuard {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("tks-test-git-{}", Uuid::new_v4()));
        Self { path }
    }
}

impl Drop for TempRepoGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[tokio::test]
async fn test_bare_repo_initialization_idempotence() {
    let guard = TempRepoGuard::new();
    let repo_path = &guard.path;

    // First initialization
    let repo1 = init_bare(repo_path).expect("First init_bare should succeed");
    let head1 = repo1.head().expect("HEAD should be set");
    assert_eq!(
        head1.name(),
        Some(SPECS_BRANCH_REF),
        "HEAD should point to refs/heads/specs"
    );

    let branch_ref1 = repo1
        .find_reference(SPECS_BRANCH_REF)
        .expect("refs/heads/specs reference should exist");
    let commit1 = branch_ref1
        .peel_to_commit()
        .expect("refs/heads/specs should resolve to a commit");

    // Second initialization on existing path (idempotency check)
    let repo2 = init_bare(repo_path).expect("Second init_bare should succeed");
    let branch_ref2 = repo2
        .find_reference(SPECS_BRANCH_REF)
        .expect("refs/heads/specs should still exist");
    let commit2 = branch_ref2
        .peel_to_commit()
        .expect("refs/heads/specs commit should exist");

    assert_eq!(
        commit1.id(),
        commit2.id(),
        "Idempotent init should not change the commit OID"
    );
}

#[tokio::test]
async fn test_commit_document_and_git_cli_verification() {
    let guard = TempRepoGuard::new();
    let repo_path = &guard.path;

    let (writer, reader) = start_git_actor(repo_path, 32).expect("Failed to start git actor");

    // 1. Commit doc1
    let content1 = "# Vision Document\n\nThis is the core substrate vision.\n";
    let res1 = writer
        .commit_document("specs/vision.md", content1)
        .await
        .expect("commit_document vision.md failed");
    assert_eq!(res1.doc_path, "specs/vision.md");
    assert!(!res1.blob_hash.is_empty());
    assert!(!res1.commit_id.is_empty());

    // 2. Commit doc2
    let content2 = "# Architecture\n\nComponent topology details.\n";
    let res2 = writer
        .commit_document("docs/architecture.md", content2)
        .await
        .expect("commit_document architecture.md failed");
    assert_eq!(res2.doc_path, "docs/architecture.md");

    // 3. Commit revision 2 of doc1
    let content1_rev2 = "# Vision Document\n\nThis is the updated substrate vision.\n";
    let res1_rev2 = writer
        .commit_document("specs/vision.md", content1_rev2)
        .await
        .expect("commit_document vision.md rev2 failed");
    assert_ne!(res1.blob_hash, res1_rev2.blob_hash);

    // Verify via reader
    let read_back = reader
        .read_blob(&res1_rev2.blob_hash)
        .await
        .expect("Failed to read updated blob");
    assert_eq!(read_back, content1_rev2);

    // 4. Standard Git CLI verification
    // git --git-dir <repo_path> log -n 1 --oneline refs/heads/specs
    let log_output = Command::new("git")
        .arg("--git-dir")
        .arg(repo_path)
        .arg("log")
        .arg("-n")
        .arg("1")
        .arg("--oneline")
        .arg(SPECS_BRANCH_REF)
        .output()
        .expect("Failed to execute git log");
    assert!(
        log_output.status.success(),
        "git log exited with error: {}",
        String::from_utf8_lossy(&log_output.stderr)
    );
    let log_str = String::from_utf8_lossy(&log_output.stdout);
    assert!(
        log_str.contains("Commit document: specs/vision.md"),
        "git log should show latest commit message, got: {log_str}"
    );

    // git --git-dir <repo_path> ls-tree -r refs/heads/specs
    let ls_output = Command::new("git")
        .arg("--git-dir")
        .arg(repo_path)
        .arg("ls-tree")
        .arg("-r")
        .arg(SPECS_BRANCH_REF)
        .output()
        .expect("Failed to execute git ls-tree");
    assert!(
        ls_output.status.success(),
        "git ls-tree exited with error: {}",
        String::from_utf8_lossy(&ls_output.stderr)
    );
    let ls_str = String::from_utf8_lossy(&ls_output.stdout);
    assert!(
        ls_str.contains("specs/vision.md"),
        "git ls-tree should contain specs/vision.md, got: {ls_str}"
    );
    assert!(
        ls_str.contains("docs/architecture.md"),
        "git ls-tree should contain docs/architecture.md, got: {ls_str}"
    );

    // git --git-dir <repo_path> diff HEAD~1 refs/heads/specs
    let diff_output = Command::new("git")
        .arg("--git-dir")
        .arg(repo_path)
        .arg("diff")
        .arg("HEAD~1")
        .arg(SPECS_BRANCH_REF)
        .output()
        .expect("Failed to execute git diff");
    assert!(
        diff_output.status.success(),
        "git diff exited with error: {}",
        String::from_utf8_lossy(&diff_output.stderr)
    );
    let diff_str = String::from_utf8_lossy(&diff_output.stdout);
    assert!(
        diff_str.contains("updated substrate vision"),
        "git diff should display the changes in revision 2, got: {diff_str}"
    );
}

#[tokio::test]
async fn test_read_blob_span_and_edge_cases() {
    let guard = TempRepoGuard::new();
    let repo_path = &guard.path;

    let (writer, reader) = start_git_actor(repo_path, 32).expect("Failed to start git actor");

    // Text containing ASCII and multi-byte UTF-8 character (em-dash is 3 bytes: 0xE2, 0x80, 0x94)
    let content = "Alpha: \u{2014} Beta Omega";
    let res = writer
        .commit_document("specs/spans.md", content)
        .await
        .expect("commit failed");

    // Full blob read
    let full = reader
        .read_blob(&res.blob_hash)
        .await
        .expect("read_blob failed");
    assert_eq!(full, content);

    // Span read exact prefix "Alpha: " (bytes 0..7)
    let span1 = reader
        .get_document_span(&res.blob_hash, 0, 7)
        .await
        .expect("get_document_span failed");
    assert_eq!(span1, "Alpha: ");

    // Span read em-dash "\u{2014}" (bytes 7..10)
    let em_dash = reader
        .get_document_span(&res.blob_hash, 7, 10)
        .await
        .expect("em-dash span failed");
    assert_eq!(em_dash, "\u{2014}");

    // Span read suffix " Beta Omega" (bytes 10..content.len())
    let suffix = reader
        .get_document_span(&res.blob_hash, 10, content.len())
        .await
        .expect("suffix span failed");
    assert_eq!(suffix, " Beta Omega");

    // Empty span (0..0)
    let empty = reader
        .get_document_span(&res.blob_hash, 0, 0)
        .await
        .expect("empty span failed");
    assert_eq!(empty, "");

    // Error: start > end
    let err_reversed = reader
        .get_document_span(&res.blob_hash, 5, 2)
        .await
        .expect_err("should fail when start > end");
    match err_reversed {
        GitError::InvalidByteRange { start, end, total } => {
            assert_eq!(start, 5);
            assert_eq!(end, 2);
            assert_eq!(total, content.len());
        }
        other => panic!("Expected InvalidByteRange, got {other:?}"),
    }

    // Error: end > total
    let err_out_of_bounds = reader
        .get_document_span(&res.blob_hash, 0, content.len() + 10)
        .await
        .expect_err("should fail when end > total");
    assert!(matches!(
        err_out_of_bounds,
        GitError::InvalidByteRange { .. }
    ));

    // Error: cutting inside multi-byte UTF-8 character (bytes 7..8)
    let err_utf8 = reader
        .get_document_span(&res.blob_hash, 7, 8)
        .await
        .expect_err("should fail with invalid utf-8");
    assert!(matches!(err_utf8, GitError::InvalidUtf8(_)));

    // Error: non-existent blob hash
    let err_not_found = reader
        .get_document_span("0000000000000000000000000000000000000000", 0, 5)
        .await
        .expect_err("should fail on non-existent OID");
    assert!(matches!(err_not_found, GitError::ObjectNotFound(_)));

    // Error: invalid hex OID
    let err_malformed_oid = reader
        .get_document_span("not-a-valid-hex-oid", 0, 5)
        .await
        .expect_err("should fail on malformed OID");
    assert!(matches!(err_malformed_oid, GitError::ObjectNotFound(_)));
}

#[tokio::test]
async fn test_actor_panic_recovery() {
    let guard = TempRepoGuard::new();
    let repo_path = &guard.path;

    let (writer, reader) = start_git_actor(repo_path, 32).expect("Failed to start git actor");

    // Trigger panic in actor using test hook
    let panic_err = writer
        .commit_document("__PANIC_TEST__", "should panic")
        .await
        .expect_err("Should return error on panic");

    match panic_err {
        GitError::ActorPanicked(msg) => {
            assert!(
                msg.contains("Simulated panic in git actor"),
                "Expected panic message, got: {msg}"
            );
        }
        other => panic!("Expected ActorPanicked, got: {other:?}"),
    }

    // Verify actor is still responsive for subsequent commands
    let recovered_content = "# Post-Panic Document\nActor recovered cleanly.\n";
    let res = writer
        .commit_document("specs/recovered.md", recovered_content)
        .await
        .expect("Actor should process commands after recovering from panic");

    assert_eq!(res.doc_path, "specs/recovered.md");

    let read_back = reader
        .read_blob(&res.blob_hash)
        .await
        .expect("Read handle should read recovered document");
    assert_eq!(read_back, recovered_content);
}

#[tokio::test]
async fn test_stress_concurrent_reads_and_writes() {
    let guard = TempRepoGuard::new();
    let repo_path = &guard.path;

    let (writer, reader) = start_git_actor(repo_path, 128).expect("Failed to start git actor");

    // Commit a baseline document that readers will read
    let base_content = "Substrate invariant baseline: atomic requirements must link to parent.";
    let base_res = writer
        .commit_document("specs/baseline.md", base_content)
        .await
        .expect("Initial commit failed");
    let base_hash = Arc::new(base_res.blob_hash);

    // Launch 50 concurrent reader tasks
    let mut reader_handles = Vec::with_capacity(50);
    for _ in 0..50 {
        let r = reader.clone();
        let hash = Arc::clone(&base_hash);
        let handle = tokio::spawn(async move {
            for _ in 0..10 {
                let slice = r
                    .get_document_span(&hash, 0, 9)
                    .await
                    .expect("Concurrent read failed");
                assert_eq!(slice, "Substrate");
            }
        });
        reader_handles.push(handle);
    }

    // Concurrently execute 10 write commits
    let writer_clone = writer.clone();
    let writer_handle = tokio::spawn(async move {
        let mut results = Vec::with_capacity(10);
        for i in 0..10 {
            let doc_path = format!("specs/batch_{i}.md");
            let content = format!("# Specification Batch {i}\nDetails for batch {i}.\n");
            let res = writer_clone
                .commit_document(&doc_path, &content)
                .await
                .expect("Concurrent write failed");
            results.push(res);
        }
        results
    });

    // Wait for all readers and writer to complete
    let write_results = writer_handle.await.expect("Writer task panicked");
    assert_eq!(write_results.len(), 10);

    for r_handle in reader_handles {
        r_handle.await.expect("Reader task panicked");
    }

    // Verify all written documents can be read
    for (i, res) in write_results.iter().enumerate() {
        let expected = format!("# Specification Batch {i}\nDetails for batch {i}.\n");
        let actual = reader
            .read_blob(&res.blob_hash)
            .await
            .expect("Failed to read back written document");
        assert_eq!(actual, expected);
    }
}

#[test]
fn test_path_normalization() {
    assert_eq!(
        normalize_doc_path("specs/vision.md").unwrap(),
        "specs/vision.md"
    );
    assert_eq!(
        normalize_doc_path("/specs/vision.md").unwrap(),
        "specs/vision.md"
    );
    assert_eq!(
        normalize_doc_path("./specs/vision.md").unwrap(),
        "specs/vision.md"
    );
    assert_eq!(
        normalize_doc_path("  specs/vision.md  ").unwrap(),
        "specs/vision.md"
    );

    // Errors
    assert!(normalize_doc_path("").is_err());
    assert!(normalize_doc_path("   ").is_err());
    assert!(normalize_doc_path("/").is_err());
    assert!(normalize_doc_path("../specs/vision.md").is_err());
    assert!(normalize_doc_path("specs/../vision.md").is_err());
}
