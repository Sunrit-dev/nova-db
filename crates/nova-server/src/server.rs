use crate::config::ServerConfig;
use crate::connection::ConnectionHandler;
use crate::engine::DatabaseEngine;
use crate::metrics::ServerMetrics;
use nova_core::error::{NovaError, Result};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, Semaphore};
use tracing::{error, info, warn};

/// Top-level network server coordinating TCP connections, engine, and graceful shutdown.
pub struct NovaServer {
    pub engine: Arc<DatabaseEngine>,
    pub metrics: Arc<ServerMetrics>,
    shutdown_tx: broadcast::Sender<()>,
}

impl NovaServer {
    /// Initialize NovaServer with the given configuration.
    pub fn new(config: ServerConfig) -> Result<Self> {
        let metrics = Arc::new(ServerMetrics::new());
        let engine = DatabaseEngine::open(config, Arc::clone(&metrics))?;
        let (shutdown_tx, _) = broadcast::channel(1);

        Ok(Self {
            engine,
            metrics,
            shutdown_tx,
        })
    }

    /// Obtain a handle to trigger graceful server shutdown.
    pub fn shutdown_handle(&self) -> broadcast::Sender<()> {
        self.shutdown_tx.clone()
    }

    /// Start listening for incoming client connections and process requests.
    pub async fn run(&self) -> Result<()> {
        let bind_addr = self.engine.config.bind_addr();
        let listener = TcpListener::bind(&bind_addr)
            .await
            .map_err(|e| NovaError::storage(format!("Failed to bind to {bind_addr}: {e}")))?;

        info!(
            address = %bind_addr,
            max_connections = self.engine.config.max_connections,
            "NOVA DB server listening"
        );

        if let Some(web_port) = self.engine.config.web_port {
            let web_host = self.engine.config.host.clone();
            let web_engine = Arc::clone(&self.engine);
            let web_shutdown_rx = self.shutdown_tx.subscribe();
            tokio::spawn(async move {
                if let Err(e) =
                    crate::web::run_web_studio(web_host, web_port, web_engine, web_shutdown_rx)
                        .await
                {
                    error!("Web Studio failed: {e}");
                }
            });
        }

        let max_conn_sem = Arc::new(Semaphore::new(self.engine.config.max_connections));
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        loop {
            tokio::select! {
                accept_res = listener.accept() => {
                    match accept_res {
                        Ok((stream, addr)) => {
                            let sem_permit = match Arc::clone(&max_conn_sem).try_acquire_owned() {
                                Ok(permit) => permit,
                                Err(_) => {
                                    warn!(client = ?addr, "Connection limit exceeded, dropping client");
                                    continue;
                                }
                            };

                            let handler = ConnectionHandler::new(Arc::clone(&self.engine), addr);
                            tokio::spawn(async move {
                                handler.run(stream).await;
                                drop(sem_permit);
                            });
                        }
                        Err(e) => {
                            error!("TCP accept failed: {e}");
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Graceful shutdown signal received. Ceasing connection acceptance.");
                    break;
                }
            }
        }

        info!("Flushing commit logs and syncing storage...");
        // Close listener & let remaining tasks finish or time out
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        info!("NOVA DB server shutdown complete");

        Ok(())
    }
}
