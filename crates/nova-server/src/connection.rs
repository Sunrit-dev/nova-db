use crate::engine::DatabaseEngine;
use futures::{SinkExt, StreamExt};
use nova_core::error::{ErrorCode, NovaError, Result};
use nova_protocol::{
    event_to_bytes, FrameType, NvpCodec, NvpFrame, RequestPayload, ResponsePayload,
};
use nova_query::ast::Statement;
use nova_query::evaluator::Evaluator;
use nova_query::parse_nql;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_util::codec::Framed;
use tracing::{debug, info, warn};

/// Manages a single client connection, framing, request routing, and event streaming.
pub struct ConnectionHandler {
    engine: Arc<DatabaseEngine>,
    addr: SocketAddr,
    authenticated: bool,
}

impl ConnectionHandler {
    pub fn new(engine: Arc<DatabaseEngine>, addr: SocketAddr) -> Self {
        let auth_needed = engine.config.auth_token.is_some();
        Self {
            engine,
            addr,
            authenticated: !auth_needed,
        }
    }

    /// Process frames from this client until disconnect or error.
    pub async fn run(mut self, stream: TcpStream) {
        let (mut sink, mut reader) = Framed::new(stream, NvpCodec::new()).split();
        self.engine.metrics.inc_connections();

        info!(client = ?self.addr, "Client connected");

        // Channel for outgoing frames (responses and live event streams)
        let (out_tx, mut out_rx) = mpsc::channel::<NvpFrame>(1024);

        // Writer task sending outgoing frames to TCP socket
        let write_task = tokio::spawn(async move {
            while let Some(frame) = out_rx.recv().await {
                if let Err(e) = sink.send(frame).await {
                    debug!("Failed to send frame to client: {e}");
                    break;
                }
            }
        });

        // Reader loop
        while let Some(result) = reader.next().await {
            match result {
                Ok(frame) => {
                    let req_id = frame.request_id;
                    match self.handle_frame(frame, &out_tx).await {
                        Ok(Some(response_frame)) => {
                            if out_tx.send(response_frame).await.is_err() {
                                break;
                            }
                        }
                        Ok(None) => {}
                        Err(err) => {
                            let resp = ResponsePayload::Error(err);
                            if let Ok(bytes) = resp.to_bytes() {
                                let err_frame = NvpFrame::new(FrameType::Error, req_id, bytes);
                                let _ = out_tx.send(err_frame).await;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(client = ?self.addr, error = ?e, "Protocol or framing error from client");
                    break;
                }
            }
        }

        self.engine.metrics.dec_connections();
        write_task.abort();
        info!(client = ?self.addr, "Client disconnected");
    }

    async fn handle_frame(
        &mut self,
        frame: NvpFrame,
        out_tx: &mpsc::Sender<NvpFrame>,
    ) -> Result<Option<NvpFrame>> {
        match frame.frame_type {
            FrameType::Ping => Ok(Some(NvpFrame::pong(frame.request_id))),
            FrameType::Pong => Ok(None),
            FrameType::Request => {
                let payload = RequestPayload::from_bytes(&frame.payload)?;
                self.handle_request(frame.request_id, payload, out_tx).await
            }
            other => Err(NovaError::protocol(format!(
                "Unexpected frame type {:?} sent from client",
                other
            ))),
        }
    }

    async fn handle_request(
        &mut self,
        req_id: u64,
        payload: RequestPayload,
        out_tx: &mpsc::Sender<NvpFrame>,
    ) -> Result<Option<NvpFrame>> {
        match payload {
            RequestPayload::Auth { token } => {
                if let Some(ref expected) = self.engine.config.auth_token {
                    if token == *expected {
                        self.authenticated = true;
                        let resp = ResponsePayload::Success {
                            message: "Authentication successful".to_string(),
                            affected: 0,
                        };
                        return Ok(Some(NvpFrame::new(
                            FrameType::Response,
                            req_id,
                            resp.to_bytes()?,
                        )));
                    } else {
                        return Err(NovaError::new(
                            ErrorCode::AuthenticationFailed,
                            "Invalid authentication token",
                        ));
                    }
                } else {
                    self.authenticated = true;
                    let resp = ResponsePayload::Success {
                        message: "No authentication required".to_string(),
                        affected: 0,
                    };
                    return Ok(Some(NvpFrame::new(
                        FrameType::Response,
                        req_id,
                        resp.to_bytes()?,
                    )));
                }
            }
            RequestPayload::Ping => {
                let resp = ResponsePayload::Pong;
                return Ok(Some(NvpFrame::new(
                    FrameType::Response,
                    req_id,
                    resp.to_bytes()?,
                )));
            }
            _ => {}
        }

        // Require authentication if enabled
        if !self.authenticated {
            return Err(NovaError::new(
                ErrorCode::PermissionDenied,
                "Authentication required before issuing queries",
            ));
        }

        match payload {
            RequestPayload::Query { nql } => {
                let stmt = parse_nql(&nql)?;
                if let Statement::Watch { collection, filter } = stmt {
                    return self.start_watch(req_id, collection, filter, out_tx).await;
                }
                let resp = self.engine.execute(stmt, "default").await?;
                Ok(Some(NvpFrame::new(
                    FrameType::Response,
                    req_id,
                    resp.to_bytes()?,
                )))
            }
            RequestPayload::Watch { nql } => {
                let stmt = parse_nql(&nql)?;
                if let Statement::Watch { collection, filter } = stmt {
                    self.start_watch(req_id, collection, filter, out_tx).await
                } else {
                    Err(NovaError::invalid_query("Expected WATCH statement"))
                }
            }
            RequestPayload::Unwatch { .. } => {
                // Handled gracefully
                let resp = ResponsePayload::Success {
                    message: "Unwatched".to_string(),
                    affected: 1,
                };
                Ok(Some(NvpFrame::new(
                    FrameType::Response,
                    req_id,
                    resp.to_bytes()?,
                )))
            }
            RequestPayload::Auth { .. } | RequestPayload::Ping => unreachable!(),
        }
    }

    async fn start_watch(
        &self,
        req_id: u64,
        collection: String,
        filter: Option<nova_query::ast::Expr>,
        out_tx: &mpsc::Sender<NvpFrame>,
    ) -> Result<Option<NvpFrame>> {
        let sub_id = req_id;
        let mut rx = self.engine.subscribe_events();
        let target_tx = out_tx.clone();

        info!(client = ?self.addr, collection = %collection, sub_id = sub_id, "Started WATCH subscription");

        // Spawn watcher background task
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        // Check if event matches target collection
                        if event.collection == collection {
                            // Check if event satisfies filter predicate (evaluate on updated/created doc)
                            let doc_ref = event.after.as_ref().or(event.before.as_ref());
                            let matches = if let Some(doc) = doc_ref {
                                Evaluator::matches_filter(&filter, doc).unwrap_or(false)
                            } else {
                                true
                            };

                            if matches {
                                if let Ok(payload) = event_to_bytes(&event) {
                                    let frame = NvpFrame::new(FrameType::Event, sub_id, payload);
                                    if target_tx.send(frame).await.is_err() {
                                        break; // Client closed connection
                                    }
                                }
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        warn!(
                            sub_id = sub_id,
                            missed = missed,
                            "Watcher lagged behind broadcast stream"
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        });

        let ack = ResponsePayload::WatchAck {
            subscription_id: sub_id,
        };
        Ok(Some(NvpFrame::new(
            FrameType::Response,
            req_id,
            ack.to_bytes()?,
        )))
    }
}
