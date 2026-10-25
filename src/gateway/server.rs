//! Axum server lifecycle initialization and graceful shutdown (WP-1.5, D-19, D-53).

use std::net::SocketAddr;

use tokio_util::sync::CancellationToken;

use crate::gateway::{AppState, create_router};

/// Starts the unified Axum server bound to `addr` with cooperative cancellation shutdown.
///
/// # Errors
///
/// Returns `std::io::Error` if socket binding fails or server encounters an unrecoverable error.
pub async fn run_server(
    addr: SocketAddr,
    state: AppState,
    cancel_token: CancellationToken,
) -> Result<(), std::io::Error> {
    let app = create_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("TKS Gateway server listening on http://{addr}");

    let cancel_token_clone = cancel_token.clone();
    tokio::select! {
        res = axum::serve(listener, app).with_graceful_shutdown(async move {
            cancel_token_clone.cancelled().await;
            tracing::info!("TKS Gateway server received shutdown signal.");
        }) => {
            res
        }
        _ = async {
            cancel_token.cancelled().await;
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            tracing::warn!("Graceful shutdown timeout reached (3s); forcing server shutdown.");
        } => {
            Ok(())
        }
    }
}
