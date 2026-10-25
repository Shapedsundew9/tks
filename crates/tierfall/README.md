# tierfall

One OpenAI-compatible endpoint over many free-tier LLM accounts, with automatic fallback.

Free tiers have low per-minute and per-day limits. tierfall puts any number of accounts
behind a single API: Google Gemini (several keys are fine), Groq, OpenRouter, any
OpenAI-compatible server, and local Ollama. Each request walks a quality-ordered chain of
`(account, model)` targets. When a target is rate-limited, out of daily quota, failing, or
has a rejected key, tierfall moves on to the next one, remembers the limit (across restarts),
and skips that target until it resets.

It is both a **Rust library** (`tierfall::Tierfall`) and a **proxy binary**
(`tierfall serve`) that any OpenAI SDK, LangChain, curl, editor plugin and so on can use
by changing only the base URL.

## Quick start (zero config)

Export your keys. Any variable named `GEMINI_API_KEY*`, `GROQ_API_KEY*` or
`OPENROUTER_API_KEY*` becomes an account. The suffix names the account, so
`GEMINI_API_KEY_SS` becomes `gemini-ss`.

```bash
export GEMINI_API_KEY_SS=... GEMINI_API_KEY_AM=... GEMINI_API_KEY_JB=...
export GROQ_API_KEY=... OPENROUTER_API_KEY=...
export TIERFALL_ACCOUNT_ORDER=gemini-ss,gemini-am,gemini-jb   # optional

cargo run -p tierfall -- status                  # accounts, ranked models, quota state
cargo run -p tierfall -- chat "Why is the sky blue?"
cargo run -p tierfall -- chat --stream -m fast "Tell me a joke"
cargo run -p tierfall -- serve                   # http://127.0.0.1:4141/v1
```

Then point any OpenAI client at the proxy:

```python
from openai import OpenAI
client = OpenAI(base_url="http://127.0.0.1:4141/v1", api_key="unused")
print(client.chat.completions.create(
    model="auto", messages=[{"role": "user", "content": "hi"}]).choices[0].message.content)
```

In this repository's devcontainer the key variables, `TIERFALL_ACCOUNT_ORDER` and
`OLLAMA_HOST` are already forwarded by `.devcontainer/devcontainer.json`. Rebuild the
container after adding keys on the host.

## Library

```rust
use futures_util::StreamExt;
use tierfall::{ChatRequest, Tierfall};

async fn demo() -> tierfall::Result<()> {
    let tf = Tierfall::from_env().await?;        // or Tierfall::from_config_file("tierfall.toml")
    let reply = tf.chat(ChatRequest::new("auto").system("Be terse.").user("Hi")).await?;
    println!("{}", reply.text().unwrap_or_default());
    let served = reply.served_by.unwrap();       // account, provider, model, attempts
    eprintln!("via {} / {}", served.account, served.model);

    let mut stream = tf.chat_stream(ChatRequest::new("fast").user("Count to 5")).await?;
    while let Some(chunk) = stream.next().await {
        print!("{}", chunk?.text().unwrap_or_default());
    }
    Ok(())
}
```

`ChatRequest` is the OpenAI request body. Unknown fields (`tools`, `response_format`,
`temperature`, `seed`, and so on) are passed through untouched, as is the upstream
response. For byte-exact pass-through use `complete_json` / `stream_json` with a raw JSON
object. `tf.status()`, `tf.models()` and `tf.routes()` expose the live state.

## Choosing a model

The request's `model` field accepts:

| Value | Meaning |
| --- | --- |
| `auto` | Default. Every ranked model on every account, best quality first. |
| `fast` | Low-latency first (Groq, then Flash-Lite-class models), then everything else. |
| `default` or empty | `router.default_route` (initially `auto`). |
| a route name | A `[[routes]]` chain from your config. |
| `gemini-3.8-flash` | That model on every account that offers it, in account order. |
| `gemini-3.8-flash@gemini-am` | Exactly that account; no fallback. |

`GET /v1/models` or `tierfall models` lists the routes first, then every concrete model.

## How fallback works

**Chain order.** Models are grouped by quality tier (1 = best). Within a tier, groups
follow `router.provider_priority` (Gemini, Groq, OpenRouter, OpenAI-compatible, Ollama),
then model rank, then accounts in `account_order`. So all three Gemini accounts serve the
best Gemini Flash model before anything drops to a lower tier.

**Tiers.** These come from built-in rules applied to each account's live `/models` list.
The list is cached for 24 hours. If discovery fails, a small static list is used instead.

| Tier | Examples |
| --- | --- |
| 1 | `gemini-*-flash`; Groq `gpt-oss-120b`, Kimi and 200B+ models; OpenRouter 100B+ `:free` models |
| 2 | `gemini-*-flash-lite`; Groq `gpt-oss-20b`, Qwen and 30B+ models; OpenRouter 20B+ `:free` models |
| 3 | Gemma, other Groq and OpenRouter `:free` models, `openrouter/free` |
| 4 | Local Ollama (preferred: `$TIERFALL_OLLAMA_MODEL`, `$TKS_LLM_MODEL`, or `qwen3:8b`) |

Pro and thinking-only Gemini models, embedding, audio, image, guard and `*-latest` alias
models are excluded from automatic routes but remain callable by name. OpenRouter is limited
to zero-priced models. Add your own `[[rankings]]` rules (checked first), set
`preferred_models`, or pin `models` per account.

**What each failure does.**

| Upstream result | tierfall response |
| --- | --- |
| 429 with a per-minute limit | Cooldown until `retry-after` / `retryDelay` / rate-limit reset headers, then try the next target. If `short_wait_secs` covers the wait, wait once and retry the same target. |
| 429 or quota error with a daily limit | Skip the target until the provider's daily reset: midnight Pacific for Gemini, midnight UTC elsewhere. Persisted. |
| `x-ratelimit-remaining-*: 0` on a success | Learned proactively, so the next request skips the target. |
| 401/403 or a rejected key (Gemini returns 400) | Disable the account for this process and log a warning. |
| 404 or a decommissioned model | Mark the model gone and re-run discovery. |
| 5xx, timeout, connection error, empty stream | Cooldown with exponential backoff (30 s base), then try the next target. |
| Context too long or 413 | Try the next target, which may have a larger window. |
| Other 4xx (a bad request) | Return it to the caller unchanged; no fallback. |
| Error JSON inside an HTTP 200 (OpenRouter) | Classified as above. |

Streaming falls back only before the first token. After that the stream is committed to
one target, and a mid-stream failure ends it with an error event. When every target is
unavailable the request fails with HTTP 429 `all_providers_exhausted`, a `Retry-After` set
to the earliest reset, and the full attempt log.

Optional `[[limits]]` (`rpm`/`rpd`) enforce known quotas proactively. Without them,
limits are learned from responses.

## Proxy API

| Endpoint | Notes |
| --- | --- |
| `POST /v1/chat/completions` | Also `/chat/completions`. Non-streaming and SSE (`"stream": true`). |
| `GET /v1/models` | Also `/models`. Routes and models in OpenAI list format. |
| `GET /status` | Also `/v1/status`. Accounts, discovery, availability, usage counters, routes. |
| `GET /health` | Unauthenticated liveness probe. |

Successful responses carry `x-tierfall-account`, `x-tierfall-provider`,
`x-tierfall-model` and `x-tierfall-attempts` headers. Errors use the OpenAI error shape.
Upstream client errors keep their status and add `error.tierfall_served_by`. Unknown models
return 404 `model_not_found`.

The server binds `127.0.0.1:4141` by default. If `TIERFALL_API_KEY` (configurable via
`server.api_key_env`) is set, clients must send it as `Authorization: Bearer ...` or
`x-api-key`. Binding a non-loopback address without a key logs a warning.

## Configuration

Everything is optional. A config file is looked up in `--config`, `$TIERFALL_CONFIG`,
`./tierfall.toml`, then `~/.config/tierfall/config.toml`. `tierfall init` writes a starter
file for your environment (`--freeze-routes` writes the current chains explicitly).
[tierfall.example.toml](tierfall.example.toml) documents every option: server, router
policy, explicit `[[accounts]]` (including any OpenAI-compatible `base_url`), named
`[[routes]]` chains, `[[limits]]` and `[[rankings]]`. API keys are only ever read from
environment variables named in the config. They are never written to config, state or
logs.

| Environment variable | Purpose |
| --- | --- |
| `GEMINI_API_KEY*`, `GROQ_API_KEY*`, `OPENROUTER_API_KEY*` | Auto-detected accounts |
| `TIERFALL_ACCOUNT_ORDER` | Comma-separated account order (overrides config) |
| `TIERFALL_CONFIG` | Config file path |
| `TIERFALL_STATE_DIR` | Quota state and model cache (default `~/.cache/tierfall`) |
| `TIERFALL_API_KEY` | Require this bearer token on the proxy |
| `OLLAMA_HOST`, `TIERFALL_OLLAMA_MODEL` | Local fallback endpoint and preferred model |
| `TIERFALL_LOG` or `RUST_LOG` | Log filter, for example `tierfall=debug` |

## Testing

```bash
cargo test -p tierfall                                   # unit + mock-upstream tests
TIERFALL_LIVE=1 cargo test -p tierfall --test live -- --nocapture --test-threads=1
```

The live tests use your real keys and a throwaway state directory. They spend one request per
cloud account plus two on `auto`. At least one cloud account must be configured; an
Ollama-only run is not cloud verification. Missing explicitly configured keys, disabled
accounts and failed discovery fail the run rather than silently reducing coverage. Explicitly
pinned models skip discovery, just as they do in normal use.

Each cloud account is checked with account-pinned requests to its top three `auto` models,
in order. The next model is tried only after a transient failure (such as Gemini's
model-wide "high demand" 503) or a quota response. An account fails if none of those
models answers or returns a genuine quota response. Authentication, missing-model and
other errors fail immediately.

To require all five cloud accounts configured in this devcontainer, use:

```bash
TIERFALL_LIVE=1 \
TIERFALL_LIVE_ACCOUNTS=gemini-ss,gemini-am,gemini-jb,groq,openrouter \
cargo test -p tierfall --test live -- --nocapture --test-threads=1
```

This checks for the required accounts before any live requests. Rebuild/reopen the
devcontainer after setting the corresponding host environment variables. Remove stale,
invalid keys from the environment or disable those accounts in config; all detected cloud
accounts are checked, not just the required list. Never paste keys into test output or chat.

An actual upstream quota response is accepted and reported as `QUOTA VERIFIED`, separately
from a successful completion. Empty attempt logs, skipped accounts, network errors and 5xx
responses are not quota verification. Successful auto-route or proxy requests through Ollama
do not replace the per-cloud-account checks. A busy upstream can hold a request until
`router.request_timeout_secs` (120 s by default) before tierfall falls back, so a run can
take several minutes.

## Notes

- Gemini free-tier quotas apply per Google Cloud project, not per key. Keys that share a
  project share one quota.
- Using several free accounts is subject to each provider's terms of service. Check them
  for your use.
- Embeddings, images and audio are not proxied. The router is chat-completions only.
