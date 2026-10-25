//! `tierfall` command-line interface: proxy server, one-shot chat, model listing, status, init.

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use serde::Serialize;
use tierfall::config::{AccountConfig, ChainEntryConfig, RouteConfig};
use tierfall::{ChatRequest, Error, Tierfall};

#[derive(Parser)]
#[command(
    name = "tierfall",
    version,
    about = "One OpenAI-compatible endpoint over many free-tier LLM accounts, with automatic fallback."
)]
struct Cli {
    /// Config file (default: $TIERFALL_CONFIG, ./tierfall.toml, ~/.config/tierfall/config.toml).
    #[arg(long, short, global = true, value_name = "FILE")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the OpenAI-compatible proxy server.
    Serve {
        /// Address to bind (default from config: 127.0.0.1:4141).
        #[arg(long)]
        bind: Option<String>,
    },
    /// Send one prompt and print the reply (reads stdin when PROMPT is omitted).
    Chat {
        /// Route alias, model ID, or model@account.
        #[arg(long, short, default_value = "auto")]
        model: String,
        /// Optional system prompt.
        #[arg(long, short)]
        system: Option<String>,
        /// Stream tokens as they arrive.
        #[arg(long)]
        stream: bool,
        /// Print the raw JSON response.
        #[arg(long, conflicts_with = "stream")]
        json: bool,
        prompt: Vec<String>,
    },
    /// List routes and the ranked models per account.
    Models {
        /// Print the OpenAI-style `/v1/models` JSON.
        #[arg(long)]
        json: bool,
        /// Re-fetch model lists, ignoring the cache.
        #[arg(long)]
        refresh: bool,
    },
    /// Show accounts, model availability, usage counters and routes.
    Status {
        /// Print the full status report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Write a starter config file reflecting the detected accounts.
    Init {
        #[arg(long, short, default_value = "tierfall.toml")]
        output: PathBuf,
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
        /// Write the current route chains as explicit [[routes]] instead of commented examples.
        #[arg(long)]
        freeze_routes: bool,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(matches!(cli.command, Command::Serve { .. }));
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            if let Error::AllExhausted {
                attempts,
                retry_after,
                ..
            } = &e
            {
                for a in attempts {
                    let until = a.until.map(|t| format!(" until {t}")).unwrap_or_default();
                    eprintln!("  {}@{}: {}{until}", a.model, a.account, a.outcome);
                }
                if let Some(d) = retry_after {
                    eprintln!("  earliest retry in {}s", d.as_secs());
                }
            }
            if let Error::Upstream { body, .. } = &e {
                eprintln!("  {}", tierfall::classify::error_message(body));
            }
            ExitCode::FAILURE
        }
    }
}

fn init_logging(serving: bool) {
    let default = if serving {
        "tierfall=info"
    } else {
        "tierfall=warn"
    };
    let filter = std::env::var("TIERFALL_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .unwrap_or_else(|_| default.to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

async fn run(cli: Cli) -> tierfall::Result<()> {
    let tf = Tierfall::load(cli.config.as_deref()).await?;
    match cli.command {
        Command::Serve { bind } => {
            let bind = bind.unwrap_or_else(|| tf.config().server.bind.clone());
            print_summary(&tf);
            tierfall::server::serve(tf, &bind).await
        }
        Command::Chat {
            model,
            system,
            stream,
            json,
            prompt,
        } => chat(&tf, model, system, stream, json, prompt).await,
        Command::Models { json, refresh } => {
            if refresh {
                tf.refresh_models().await;
            }
            if json {
                print_json(&tf.models())
            } else {
                print_models(&tf);
                Ok(())
            }
        }
        Command::Status { json } => {
            if json {
                print_json(&tf.status())
            } else {
                print_status(&tf);
                Ok(())
            }
        }
        Command::Init {
            output,
            force,
            freeze_routes,
        } => init(&tf, &output, force, freeze_routes),
    }
}

async fn chat(
    tf: &Tierfall,
    model: String,
    system: Option<String>,
    stream: bool,
    json: bool,
    prompt: Vec<String>,
) -> tierfall::Result<()> {
    let mut text = prompt.join(" ");
    if text.trim().is_empty() {
        if std::io::stdin().is_terminal() {
            eprintln!("Enter a prompt, then Ctrl-D:");
        }
        std::io::stdin().read_to_string(&mut text)?;
    }
    if text.trim().is_empty() {
        return Err(Error::InvalidRequest("empty prompt".into()));
    }
    let mut request = ChatRequest::new(model);
    if let Some(system) = system {
        request = request.system(system);
    }
    request = request.user(text);
    if stream {
        let mut s = tf.chat_stream(request).await?;
        let served = s.served_by().clone();
        let mut out = std::io::stdout().lock();
        while let Some(chunk) = s.next().await {
            if let Some(t) = chunk?.text() {
                out.write_all(t.as_bytes())?;
                out.flush()?;
            }
        }
        writeln!(out)?;
        eprintln!(
            "[served by {} / {} after {} call(s)]",
            served.account, served.model, served.attempts
        );
    } else if json {
        let raw = tf.chat_raw(request).await?;
        println!("{}", String::from_utf8_lossy(&raw.body));
        eprintln!(
            "[served by {} / {} after {} call(s)]",
            raw.served_by.account, raw.served_by.model, raw.served_by.attempts
        );
    } else {
        let reply = tf.chat(request).await?;
        println!("{}", reply.text().unwrap_or_default());
        if let Some(served) = &reply.served_by {
            eprintln!(
                "[served by {} / {} after {} call(s)]",
                served.account, served.model, served.attempts
            );
        }
    }
    Ok(())
}

fn print_json(value: &impl Serialize) -> tierfall::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_summary(tf: &Tierfall) {
    let routes = tf.routes();
    let default = &tf.config().router.default_route;
    let accounts: Vec<&str> = tf.accounts().iter().map(|a| a.id.as_str()).collect();
    tracing::info!(target: "tierfall", "accounts: {}", accounts.join(", "));
    if let Some(chain) = routes.get(default) {
        let preview: Vec<String> = chain.iter().take(6).map(ToString::to_string).collect();
        tracing::info!(
            target: "tierfall",
            "route '{default}' ({} entries): {}{}",
            chain.len(),
            preview.join(" -> "),
            if chain.len() > 6 { " -> ..." } else { "" }
        );
    }
    for w in tf.warnings() {
        tracing::warn!(target: "tierfall", "{w}");
    }
}

fn print_models(tf: &Tierfall) {
    let routes = tf.routes();
    println!("Routes:");
    for (name, chain) in &routes {
        println!("  {name} ({} entries)", chain.len());
        for (i, t) in chain.iter().enumerate() {
            println!("    {:>2}. {t}", i + 1);
        }
    }
    println!("\nModels by account:");
    let catalogs = tf.catalogs();
    for account in tf.accounts() {
        let Some(c) = catalogs.get(&account.id) else {
            continue;
        };
        println!(
            "  {} [{}] source={:?}{}",
            account.id,
            account.provider,
            c.source,
            c.error
                .as_ref()
                .map(|e| format!(" error={e}"))
                .unwrap_or_default()
        );
        for m in &c.ranked {
            println!("    tier {}  {}", m.tier, m.id);
        }
    }
    print_warnings(tf);
}

fn print_status(tf: &Tierfall) {
    let status = tf.status();
    if let Some(p) = &status.config_path {
        println!("config: {}", p.display());
    } else {
        println!("config: (none, zero-config defaults)");
    }
    if let Some(p) = &status.state_path {
        println!("state:  {}", p.display());
    }
    println!("default route: {}\n", status.default_route);
    for a in &status.accounts {
        let disabled = a
            .disabled
            .as_ref()
            .map(|r| format!("  DISABLED: {r}"))
            .unwrap_or_default();
        println!(
            "{} [{}] discovery={}{}",
            a.id,
            a.provider,
            a.discovery
                .map(|d| format!("{d:?}").to_lowercase())
                .unwrap_or_else(|| "-".into()),
            disabled
        );
        if let Some(e) = &a.discovery_error {
            println!("    discovery error: {e}");
        }
        for m in &a.models {
            let tier = m
                .tier
                .map(|t| format!("t{t}"))
                .unwrap_or_else(|| "--".into());
            let until = m.until.map(|t| format!(" until {t}")).unwrap_or_default();
            let limits = match (m.rpm, m.rpd) {
                (None, None) => String::new(),
                (rpm, rpd) => format!(
                    " limit rpm={} rpd={}",
                    rpm.map_or("-".into(), |v| v.to_string()),
                    rpd.map_or("-".into(), |v| v.to_string())
                ),
            };
            println!(
                "    {tier} {:<45} {:<9}{until} req={} tok={}{limits}",
                m.model, m.availability, m.requests_today, m.tokens_today
            );
            if let Some(e) = &m.last_error {
                println!("         last error: {e}");
            }
        }
    }
    print_warnings(tf);
}

fn print_warnings(tf: &Tierfall) {
    let warnings = tf.warnings();
    if !warnings.is_empty() {
        println!("\nWarnings:");
        for w in warnings {
            println!("  - {w}");
        }
    }
}

#[derive(Serialize)]
struct AccountsDoc {
    accounts: Vec<AccountConfig>,
}

#[derive(Serialize)]
struct RoutesDoc {
    routes: Vec<RouteConfig>,
}

fn init(
    tf: &Tierfall,
    output: &std::path::Path,
    force: bool,
    freeze_routes: bool,
) -> tierfall::Result<()> {
    if output.exists() && !force {
        return Err(Error::Config(format!(
            "{} already exists (use --force to overwrite)",
            output.display()
        )));
    }
    let cfg = tf.config();
    let mut text = String::new();
    text.push_str(
        "# tierfall configuration (generated by `tierfall init`).\n\
         # API keys are read from the environment variables named here; never put keys in this file.\n\
         # Every setting is optional; delete anything you do not need.\n\n",
    );
    text.push_str(&format!(
        "[server]\nbind = \"{}\"\n# Clients must send `Authorization: Bearer <key>` when this env var is set.\napi_key_env = \"{}\"\n\n",
        cfg.server.bind, cfg.server.api_key_env
    ));
    let r = &cfg.router;
    text.push_str(&format!(
        "[router]\ndefault_route = \"{}\"\n# Wait in place for per-minute limits resetting within N seconds (0 = fall back immediately).\nshort_wait_secs = {}\nrequest_timeout_secs = {}\n# Pick up additional *_API_KEY* variables automatically.\nauto_detect = {}\ninclude_ollama = {}\nprovider_priority = [{}]\nmodels_per_tier = {}\ndiscovery_ttl_hours = {}\n\n",
        r.default_route,
        r.short_wait_secs,
        r.request_timeout_secs,
        r.auto_detect,
        r.include_ollama,
        r.provider_priority
            .iter()
            .map(|p| format!("\"{p}\""))
            .collect::<Vec<_>>()
            .join(", "),
        r.models_per_tier,
        r.discovery_ttl_hours,
    ));
    text.push_str("# Accounts, in fallback order within each quality tier.\n");
    let accounts = AccountsDoc {
        accounts: tf
            .accounts()
            .iter()
            .map(tierfall::Account::to_config)
            .collect(),
    };
    text.push_str(&toml::to_string(&accounts).map_err(|e| Error::Config(e.to_string()))?);
    text.push('\n');
    if freeze_routes {
        text.push_str("# Frozen route chains (tried top to bottom).\n");
        let routes = RoutesDoc {
            routes: tf
                .routes()
                .into_iter()
                .map(|(name, chain)| RouteConfig {
                    name,
                    description: None,
                    chain: chain
                        .into_iter()
                        .map(|t| ChainEntryConfig {
                            account: Some(t.account),
                            model: Some(t.model),
                            ..ChainEntryConfig::default()
                        })
                        .collect(),
                })
                .collect(),
        };
        text.push_str(&toml::to_string(&routes).map_err(|e| Error::Config(e.to_string()))?);
    } else {
        text.push_str(
            "# Custom routes (the built-in `auto` and `fast` routes are generated from discovery).\n\
             # [[routes]]\n\
             # name = \"smart\"\n\
             # chain = [\n\
             #   { provider = \"gemini\", model = \"gemini-flash-latest\" },\n\
             #   { account = \"groq\", model = \"openai/gpt-oss-120b\" },\n\
             #   { route = \"auto\" },\n\
             # ]\n\n\
             # Optional proactive limits (otherwise limits are learned from 429 responses).\n\
             # [[limits]]\n\
             # provider = \"gemini\"\n\
             # model = \"gemini-*-flash\"\n\
             # rpm = 5\n\
             # rpd = 20\n",
        );
    }
    tierfall::Config::from_toml_str(&text)?;
    std::fs::write(output, text)?;
    println!("wrote {}", output.display());
    Ok(())
}
