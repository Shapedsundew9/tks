//! Asynchronous event notification bus wrapping PostgreSQL `LISTEN`/`NOTIFY`
//! and Tokio broadcast channels (WP-3.1, PHASE3-002, TB-1).

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Notification channel name for substrate graph change events in PostgreSQL.
pub const GRAPH_EVENTS_CHANNEL: &str = "tks_graph_events";

/// Default capacity for the in-memory Tokio broadcast channel.
pub const DEFAULT_EVENT_BUS_CAPACITY: usize = 4096;

/// Real-time change event emitted when graph mutations, invalidation cascades,
/// or promotions occur (WP-3.1, WP-3.4).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphChangeEvent {
    pub event_seq: i64,
    pub batch_id: Uuid,
    pub event_type: String,
    pub entity_id: Uuid,
    pub entity_type: String,
    pub actor_id: String,
    pub timestamp: DateTime<Utc>,
}

/// Substrate asynchronous event notification bus managing in-memory Tokio broadcast
/// channels connected to PostgreSQL `LISTEN tks_graph_events`.
#[derive(Clone, Debug)]
pub struct GraphEventBus {
    sender: broadcast::Sender<GraphChangeEvent>,
}

impl Default for GraphEventBus {
    fn default() -> Self {
        Self::new(DEFAULT_EVENT_BUS_CAPACITY)
    }
}

impl GraphEventBus {
    /// Creates a new `GraphEventBus` with the specified buffer capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(16));
        Self { sender }
    }

    /// Subscribes to the broadcast channel receiving live `GraphChangeEvent` notifications.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<GraphChangeEvent> {
        self.sender.subscribe()
    }

    /// Publishes a `GraphChangeEvent` to all active in-memory subscribers.
    ///
    /// Returns the number of active receivers that received the message.
    pub fn publish(&self, event: GraphChangeEvent) -> usize {
        self.sender.send(event).unwrap_or(0)
    }

    /// Returns the current number of active broadcast receivers.
    #[must_use]
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

/// Subscribes to graph change events emitted on `bus`.
///
/// Standard contract function for WP-3.1.
pub async fn subscribe_graph_events(bus: &GraphEventBus) -> broadcast::Receiver<GraphChangeEvent> {
    bus.subscribe()
}

/// Runs a persistent PostgreSQL listener loop subscribing to `LISTEN tks_graph_events`
/// and forwarding received notifications to `bus`.
///
/// Automatically reconnects with exponential backoff on connection drops.
pub async fn run_pg_listener(
    database_url: &str,
    bus: GraphEventBus,
    cancel_token: CancellationToken,
) {
    let mut backoff_ms = 100u64;

    while !cancel_token.is_cancelled() {
        tracing::debug!("Connecting to PostgreSQL for LISTEN {GRAPH_EVENTS_CHANNEL}...");
        match tokio_postgres::connect(database_url, tokio_postgres::NoTls).await {
            Ok((client, mut connection)) => {
                let listen_sql = format!("LISTEN {GRAPH_EVENTS_CHANNEL};");
                let listen_ok = {
                    let listen_fut = client.execute(&listen_sql, &[]);
                    tokio::pin!(listen_fut);
                    let mut ok = false;
                    loop {
                        tokio::select! {
                            _ = cancel_token.cancelled() => return,
                            res = &mut listen_fut => {
                                match res {
                                    Ok(_) => {
                                        ok = true;
                                        break;
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to execute LISTEN {GRAPH_EVENTS_CHANNEL}: {e}");
                                        break;
                                    }
                                }
                            }
                            msg = futures_util::future::poll_fn(|cx| connection.poll_message(cx)) => {
                                match msg {
                                    Some(Err(e)) => {
                                        tracing::error!("Connection error during LISTEN setup: {e}");
                                        break;
                                    }
                                    None => {
                                        tracing::error!("Connection closed during LISTEN setup");
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    ok
                };

                if !listen_ok {
                    tokio::select! {
                        _ = cancel_token.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_millis(backoff_ms)) => {}
                    }
                    backoff_ms = (backoff_ms * 2).min(5000);
                    continue;
                }

                backoff_ms = 100;
                tracing::info!("Listening on PostgreSQL channel '{GRAPH_EVENTS_CHANNEL}'");

                // Keep client handle alive to prevent connection closure
                let _client_guard = client;

                loop {
                    tokio::select! {
                        _ = cancel_token.cancelled() => {
                            tracing::info!("PostgreSQL listener loop received cancellation.");
                            return;
                        }
                        msg = futures_util::future::poll_fn(|cx| connection.poll_message(cx)) => {
                            match msg {
                                Some(Ok(tokio_postgres::AsyncMessage::Notification(n))) => {
                                    if n.channel() == GRAPH_EVENTS_CHANNEL {
                                        match serde_json::from_str::<GraphChangeEvent>(n.payload()) {
                                            Ok(event) => {
                                                bus.publish(event);
                                            }
                                            Err(err) => {
                                                tracing::warn!(
                                                    "Failed to parse GraphChangeEvent from notification: {err}. Payload: {}",
                                                    n.payload()
                                                );
                                            }
                                        }
                                    }
                                }
                                Some(Ok(_)) => {} // Ignore Notice and other non-notification messages
                                Some(Err(e)) => {
                                    tracing::warn!("PostgreSQL notification connection error: {e}");
                                    break;
                                }
                                None => {
                                    tracing::warn!("PostgreSQL notification stream closed by remote server");
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to establish PostgreSQL connection for LISTEN: {e}");
                tokio::select! {
                    _ = cancel_token.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_millis(backoff_ms)) => {}
                }
                backoff_ms = (backoff_ms * 2).min(5000);
            }
        }
    }
}

/// Spawns the PostgreSQL notification listener task in the background.
pub fn start_pg_listener(
    database_url: &str,
    bus: GraphEventBus,
    cancel_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let url = database_url.to_string();
    tokio::spawn(async move {
        run_pg_listener(&url, bus, cancel_token).await;
    })
}
