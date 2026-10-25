//! Keeps the shipped example configuration valid and meaningful.

use tierfall::config::{Config, Env};
use tierfall::{ProviderKind, Tierfall};

const EXAMPLE: &str = include_str!("../tierfall.example.toml");

#[test]
fn example_config_parses() {
    let cfg = Config::from_toml_str(EXAMPLE).unwrap();
    assert_eq!(cfg.server.bind, "127.0.0.1:4141");
    assert_eq!(cfg.accounts.len(), 3);
    assert_eq!(cfg.accounts[2].provider, ProviderKind::Openai);
    assert_eq!(cfg.routes.len(), 2);
    assert_eq!(cfg.limits.len(), 2);
    assert_eq!(cfg.rankings.len(), 2);
}

#[tokio::test]
async fn example_config_builds_routes() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::from_toml_str(EXAMPLE).unwrap();
    cfg.router.discovery = false;
    cfg.router.include_ollama = false;
    cfg.router.state_dir = Some(dir.path().to_path_buf());
    let env = Env::from_pairs([
        ("GEMINI_API_KEY_SS", "k1"),
        ("GEMINI_API_KEY_AM", "k2"),
        ("GROQ_API_KEY", "k3"),
    ]);
    let tf = Tierfall::with_env(cfg, env).await.unwrap();
    let ids: Vec<&str> = tf.accounts().iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["gemini-ss", "gemini-am", "groq"]);
    let smart: Vec<String> = tf.routes()["smart"]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        &smart[..3],
        [
            "gemini-3.8-flash@gemini-ss",
            "gemini-3.8-flash@gemini-am",
            "openai/gpt-oss-120b@groq"
        ]
    );
    assert!(smart.len() > 3, "auto is spliced in: {smart:?}");
    let cheap: Vec<String> = tf.routes()["cheap"]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(
        cheap.ends_with(&[
            "openai/gpt-oss-120b@groq".to_string(),
            "openai/gpt-oss-20b@groq".to_string()
        ]),
        "{cheap:?}"
    );
}
