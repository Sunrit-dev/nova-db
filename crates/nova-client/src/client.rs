use crate::collection::CollectionHandle;
use crate::stream::EventStream;
use futures::{SinkExt, StreamExt};
use nova_core::error::{NovaError, Result};
use nova_core::event::DataEvent;
use nova_protocol::{
    event_from_bytes, FrameType, NvpCodec, NvpFrame, RequestPayload, ResponsePayload,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::codec::Framed;
use tracing::warn;

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<ResponsePayload>>>>>;
type WatcherMap = Arc<Mutex<HashMap<u64, mpsc::Sender<DataEvent>>>>;

/// Official asynchronous Rust client for communicating with NOVA DB.
#[derive(Clone)]
pub struct NovaClient {
    tx: mpsc::Sender<NvpFrame>,
    pending: PendingMap,
    watchers: WatcherMap,
    next_req_id: Arc<AtomicU64>,
}

impl NovaClient {
    /// Connect to a NOVA DB instance at the specified address (e.g. "127.0.0.1:7400").
    pub async fn connect(addr: impl Into<String>) -> Result<Self> {
        Self::connect_internal(addr.into(), None).await
    }

    /// Connect with an authentication token.
    pub async fn with_auth(addr: impl Into<String>, token: impl Into<String>) -> Result<Self> {
        Self::connect_internal(addr.into(), Some(token.into())).await
    }

    async fn connect_internal(addr: String, token: Option<String>) -> Result<Self> {
        let stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| NovaError::protocol(format!("Failed to connect to {addr}: {e}")))?;

        let (mut sink, mut reader) = Framed::new(stream, NvpCodec::new()).split();
        let (tx, mut rx) = mpsc::channel::<NvpFrame>(512);

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let watchers: WatcherMap = Arc::new(Mutex::new(HashMap::new()));
        let next_req_id = Arc::new(AtomicU64::new(1));

        // Background write task
        tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                if let Err(e) = sink.send(frame).await {
                    warn!("Client failed to write frame: {e}");
                    break;
                }
            }
        });

        // Background read task
        let pending_clone = Arc::clone(&pending);
        let watchers_clone = Arc::clone(&watchers);

        tokio::spawn(async move {
            while let Some(res) = reader.next().await {
                match res {
                    Ok(frame) => {
                        let req_id = frame.request_id;
                        match frame.frame_type {
                            FrameType::Response => {
                                let mut pend = pending_clone.lock().await;
                                if let Some(sender) = pend.remove(&req_id) {
                                    let payload_res = ResponsePayload::from_bytes(&frame.payload);
                                    let _ = sender.send(payload_res);
                                }
                            }
                            FrameType::Error => {
                                let mut pend = pending_clone.lock().await;
                                if let Some(sender) = pend.remove(&req_id) {
                                    let err = ResponsePayload::from_bytes(&frame.payload).and_then(
                                        |resp| match resp {
                                            ResponsePayload::Error(e) => Err(e),
                                            _ => Err(NovaError::protocol("Unknown error format")),
                                        },
                                    );
                                    let _ = sender.send(err);
                                }
                            }
                            FrameType::Event => {
                                let watchers = watchers_clone.lock().await;
                                if let Some(stream_tx) = watchers.get(&req_id) {
                                    if let Ok(event) = event_from_bytes(&frame.payload) {
                                        let _ = stream_tx.send(event).await;
                                    }
                                }
                            }
                            FrameType::Pong => {
                                let mut pend = pending_clone.lock().await;
                                if let Some(sender) = pend.remove(&req_id) {
                                    let _ = sender.send(Ok(ResponsePayload::Pong));
                                }
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        warn!("Client connection dropped: {e}");
                        break;
                    }
                }
            }
        });

        let client = Self {
            tx,
            pending,
            watchers,
            next_req_id,
        };

        // Authenticate if token provided
        if let Some(tok) = token {
            let req = RequestPayload::Auth { token: tok };
            let resp = client.send_request(req).await?;
            match resp {
                ResponsePayload::Success { .. } => {}
                _ => {
                    return Err(NovaError::new(
                        nova_core::ErrorCode::AuthenticationFailed,
                        "Auth failed",
                    ))
                }
            }
        }

        Ok(client)
    }

    /// Access a collection in the default namespace.
    pub fn collection(&self, name: impl Into<String>) -> CollectionHandle {
        CollectionHandle::new(self.clone(), name.into())
    }

    /// Execute an arbitrary raw NQL statement string.
    pub async fn execute_nql(&self, nql: impl Into<String>) -> Result<ResponsePayload> {
        let req = RequestPayload::Query { nql: nql.into() };
        self.send_request(req).await
    }

    /// Ping server for liveness check.
    pub async fn ping(&self) -> Result<()> {
        let req = RequestPayload::Ping;
        let resp = self.send_request(req).await?;
        match resp {
            ResponsePayload::Pong => Ok(()),
            _ => Err(NovaError::protocol("Invalid ping response")),
        }
    }

    pub(crate) async fn send_request(&self, payload: RequestPayload) -> Result<ResponsePayload> {
        let req_id = self.next_req_id.fetch_add(1, Ordering::SeqCst);
        let payload_bytes = payload.to_bytes()?;
        let frame = NvpFrame::new(FrameType::Request, req_id, payload_bytes);

        let (resp_tx, resp_rx) = oneshot::channel();
        {
            let mut pend = self.pending.lock().await;
            pend.insert(req_id, resp_tx);
        }

        self.tx
            .send(frame)
            .await
            .map_err(|e| NovaError::protocol(format!("Failed to dispatch request: {e}")))?;

        match resp_rx.await {
            Ok(res) => res,
            Err(_) => Err(NovaError::protocol(
                "Server dropped connection before replying",
            )),
        }
    }

    /// Register a real-time event watcher with a raw NQL query.
    pub async fn register_watch(&self, nql: String) -> Result<EventStream> {
        let req_id = self.next_req_id.fetch_add(1, Ordering::SeqCst);
        let (stream_tx, stream_rx) = mpsc::channel(256);

        {
            let mut watchers = self.watchers.lock().await;
            watchers.insert(req_id, stream_tx);
        }

        let req = RequestPayload::Watch { nql };
        let payload_bytes = req.to_bytes()?;
        let frame = NvpFrame::new(FrameType::Request, req_id, payload_bytes);

        let (resp_tx, resp_rx) = oneshot::channel();
        {
            let mut pend = self.pending.lock().await;
            pend.insert(req_id, resp_tx);
        }

        self.tx
            .send(frame)
            .await
            .map_err(|e| NovaError::protocol(format!("Failed to send watch: {e}")))?;

        let resp = resp_rx
            .await
            .map_err(|_| NovaError::protocol("Server closed before acknowledging WATCH"))??;

        match resp {
            ResponsePayload::WatchAck { .. } => Ok(EventStream::new(stream_rx)),
            other => Err(NovaError::protocol(format!(
                "Unexpected watch response: {:?}",
                other
            ))),
        }
    }
}
