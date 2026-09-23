use nova_client::NovaClient;
use nova_core::error::Result;
use nova_server::{NovaServer, ServerConfig};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, Ordering};
use tempfile::TempDir;
use tokio::task::JoinHandle;

static NEXT_TEST_PORT: AtomicU16 = AtomicU16::new(18400);

/// An ephemeral test harness for launching, crashing, and restarting NOVA DB instances.
pub struct TestHarness {
    pub port: u16,
    pub dir: TempDir,
    server_task: Option<JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
}

impl TestHarness {
    /// Boot a new test harness with an ephemeral data directory and dynamic port.
    pub async fn new() -> Result<Self> {
        let dir = TempDir::new().expect("Failed to create tempdir");
        let port = NEXT_TEST_PORT.fetch_add(1, Ordering::SeqCst);

        let mut harness = Self {
            port,
            dir,
            server_task: None,
            shutdown_tx: None,
        };

        harness.start_server().await?;
        Ok(harness)
    }

    /// Data directory path.
    pub fn data_dir(&self) -> &Path {
        self.dir.path()
    }

    /// WAL directory path.
    pub fn wal_dir(&self) -> PathBuf {
        self.dir.path().join("wal")
    }

    /// Server address string.
    pub fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// Connect a new client to this test instance.
    pub async fn client(&self) -> Result<NovaClient> {
        NovaClient::connect(self.addr()).await
    }

    /// Start or restart the database server.
    pub async fn start_server(&mut self) -> Result<()> {
        let config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port: self.port,
            data_dir: self.data_dir().to_path_buf(),
            ..Default::default()
        };

        let server = NovaServer::new(config)?;
        self.shutdown_tx = Some(server.shutdown_handle());

        let task = tokio::spawn(async move {
            let _ = server.run().await;
        });

        self.server_task = Some(task);
        // Wait for server to bind
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        Ok(())
    }

    /// Simulate sudden server crash without flushing in-memory state.
    pub fn kill(&mut self) {
        if let Some(task) = self.server_task.take() {
            task.abort();
        }
        self.shutdown_tx = None;
    }

    /// Graceful server shutdown.
    pub async fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.server_task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        self.kill();
    }
}
