//! Task operations CLI client (WP-2.5, D-57, D-63).

use clap::{Args, Subcommand};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Subcommands for execution task operations.
#[derive(Debug, Subcommand)]
pub enum TaskSubcommand {
    /// Create a new execution subtask under a parent requirement or specification.
    Create(CreateTaskArgs),
    /// Update the execution status of an active task.
    Update(UpdateTaskArgs),
    /// List execution tasks, optionally filtered by parent or status.
    List(ListTasksArgs),
    /// Ingest a pre-merge CI test run execution result onto active tasks.
    TestRun(TestRunArgs),
}

/// Arguments for `tks task create --parent <id_or_key> --title <title> [--content <text>]`.
#[derive(Debug, Args)]
pub struct CreateTaskArgs {
    /// Polymorphic identifier of the parent requirement or specification (UUID or node_key).
    #[arg(long)]
    pub parent: String,

    /// Human-readable title of the subtask.
    #[arg(long)]
    pub title: String,

    /// Optional intent / description of the subtask.
    #[arg(long)]
    pub content: Option<String>,
}

/// Arguments for `tks task update <id_or_key> --status <status> [--notes <notes>]`.
#[derive(Debug, Args)]
pub struct UpdateTaskArgs {
    /// Polymorphic identifier of the task (UUID or node_key).
    pub id: String,

    /// Execution status (OPEN, IN_PROGRESS, BLOCKED, COMPLETED).
    #[arg(long)]
    pub status: String,

    /// Optional operational notes or completion evidence.
    #[arg(long)]
    pub notes: Option<String>,
}

/// Arguments for `tks task list [--parent <id_or_key>] [--status <status>] [--limit <limit>]`.
#[derive(Debug, Args)]
pub struct ListTasksArgs {
    /// Optional parent node identifier filter.
    #[arg(long)]
    pub parent: Option<String>,

    /// Optional status filter (e.g. OPEN, IN_PROGRESS, BLOCKED, COMPLETED).
    #[arg(long)]
    pub status: Option<String>,

    /// Maximum number of tasks to return (default: 50).
    #[arg(long, default_value = "50")]
    pub limit: u32,
}

/// Arguments for `tks task test-run [--suite <suite>] [--passed] --pr-commit-sha <sha> --task-ids <ids> [--duration-ms <ms>] [--manifest-hash <hash>]`.
#[derive(Debug, Args, Clone)]
pub struct TestRunArgs {
    /// Test suite name (e.g. cargo_test).
    #[arg(long, default_value = "cargo_test")]
    pub suite: String,

    /// Whether the test run passed.
    #[arg(long)]
    pub passed: bool,

    /// Candidate pull request commit SHA.
    #[arg(long)]
    pub pr_commit_sha: String,

    /// Target task identifiers (comma-separated UUIDs).
    #[arg(long, value_delimiter = ',')]
    pub task_ids: Vec<String>,

    /// Execution duration in milliseconds.
    #[arg(long)]
    pub duration_ms: Option<u64>,

    /// Cryptographic hash of test manifest or suite configuration.
    #[arg(long)]
    pub manifest_hash: Option<String>,
}

#[derive(Serialize)]
struct CreateSubtaskPayload<'a> {
    title: &'a str,
    content: Option<&'a str>,
    attributes: Option<Value>,
}

#[derive(Deserialize)]
struct CreateSubtaskResult {
    task_id: Value,
    node_key: Option<String>,
    status: Option<String>,
    batch_id: Option<Value>,
    event_seq: Option<i64>,
    governance_policy: Option<String>,
}

#[derive(Serialize)]
struct UpdateStatusPayload<'a> {
    status: &'a str,
    notes: Option<&'a str>,
}

#[derive(Deserialize)]
struct UpdateStatusResult {
    task_id: Value,
    status: Option<String>,
    execution_status: Option<String>,
    batch_id: Option<Value>,
    event_seq: Option<i64>,
}

#[derive(Deserialize)]
struct TaskListItem {
    id: Value,
    node_key: Option<String>,
    title: Option<String>,
    lifecycle_state: Option<String>,
    execution_status: Option<String>,
    governance_policy: Option<String>,
}

/// Executes the `tks task` CLI subcommand.
///
/// # Errors
///
/// Returns an error if the HTTP request fails or server returns an error.
pub async fn run_task(
    subcmd: TaskSubcommand,
    server_url: &str,
    auth_token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let base_url = server_url.trim_end_matches('/');

    match subcmd {
        TaskSubcommand::Create(args) => {
            let url = format!("{base_url}/api/v1/nodes/{}/subtasks", args.parent);
            let payload = CreateSubtaskPayload {
                title: &args.title,
                content: args.content.as_deref(),
                attributes: None,
            };

            let resp = client
                .post(&url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .header(CONTENT_TYPE, "application/json")
                .json(&payload)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to create subtask ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let result: CreateSubtaskResult = resp.json().await?;
            println!("Task created successfully:");
            println!("  task_id:           {}", result.task_id);
            if let Some(key) = result.node_key {
                println!("  node_key:          {key}");
            }
            if let Some(status) = result.status {
                println!("  status:            {status}");
            }
            if let Some(batch_id) = result.batch_id {
                println!("  batch_id:          {batch_id}");
            }
            if let Some(event_seq) = result.event_seq {
                println!("  event_seq:         {event_seq}");
            }
            if let Some(policy) = result.governance_policy {
                println!("  governance_policy: {policy}");
            }
        }

        TaskSubcommand::Update(args) => {
            let url = format!("{base_url}/api/v1/nodes/{}/status", args.id);
            let payload = UpdateStatusPayload {
                status: &args.status,
                notes: args.notes.as_deref(),
            };

            let resp = client
                .patch(&url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .header(CONTENT_TYPE, "application/json")
                .json(&payload)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to update task ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let result: UpdateStatusResult = resp.json().await?;
            println!("Task updated successfully:");
            println!("  task_id:          {}", result.task_id);
            if let Some(status) = result.status {
                println!("  status:           {status}");
            }
            if let Some(exec_status) = result.execution_status {
                println!("  execution_status: {exec_status}");
            }
            if let Some(batch_id) = result.batch_id {
                println!("  batch_id:         {batch_id}");
            }
            if let Some(event_seq) = result.event_seq {
                println!("  event_seq:        {event_seq}");
            }
        }

        TaskSubcommand::List(args) => {
            let mut url = format!("{base_url}/api/v1/tasks?limit={}", args.limit);
            if let Some(ref p) = args.parent {
                url.push_str(&format!("&parent_id={p}"));
            }
            if let Some(ref s) = args.status {
                url.push_str(&format!("&status={s}"));
            }

            let resp = client
                .get(&url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to list tasks ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let items: Vec<TaskListItem> = resp.json().await?;
            println!("Tasks ({}):", items.len());
            for item in items {
                let key = item.node_key.as_deref().unwrap_or("<no-key>");
                let title = item.title.as_deref().unwrap_or("<no-title>");
                let status = item
                    .execution_status
                    .or(item.lifecycle_state)
                    .unwrap_or_else(|| "ACTIVE".to_string());
                let policy = item.governance_policy.as_deref().unwrap_or("DEFAULT");
                println!(
                    "  [{}] {} | {} | \"{}\" (policy: {})",
                    item.id, key, status, title, policy
                );
            }
        }

        TaskSubcommand::TestRun(args) => {
            run_test_run_inner(&client, base_url, auth_token, args).await?;
        }
    }

    Ok(())
}

/// Helper executing pre-merge test run submission.
pub async fn run_test_run_inner(
    client: &reqwest::Client,
    base_url: &str,
    auth_token: &str,
    args: TestRunArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("{base_url}/api/v1/verification/test-run");
    let payload = serde_json::json!({
        "test_suite": args.suite,
        "passed": args.passed,
        "pr_commit_sha": args.pr_commit_sha,
        "target_task_ids": args.task_ids,
        "duration_ms": args.duration_ms,
        "manifest_hash": args.manifest_hash,
    });

    let resp = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {auth_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&payload)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_text = resp.text().await.unwrap_or_default();
        eprintln!("Failed to record test run ({status}): {err_text}");
        return Err(format!("Server returned {status}: {err_text}").into());
    }

    let result: Value = resp.json().await?;
    println!("Pre-merge test run recorded successfully:");
    println!(
        "  status:        {}",
        result
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("RECORDED")
    );
    println!(
        "  test_suite:    {}",
        result
            .get("test_suite")
            .and_then(|v| v.as_str())
            .unwrap_or(&args.suite)
    );
    println!(
        "  passed:        {}",
        result
            .get("passed")
            .and_then(|v| v.as_bool())
            .unwrap_or(args.passed)
    );
    println!(
        "  pr_commit_sha: {}",
        result
            .get("pr_commit_sha")
            .and_then(|v| v.as_str())
            .unwrap_or(&args.pr_commit_sha)
    );
    if let Some(tasks) = result.get("recorded_task_ids").and_then(|v| v.as_array()) {
        println!("  recorded_tasks: {:?}", tasks);
    }

    Ok(())
}
