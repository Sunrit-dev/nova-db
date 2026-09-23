use futures::Stream;
use nova_core::event::DataEvent;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;

/// An asynchronous stream of real-time DataEvent occurrences.
pub struct EventStream {
    receiver: mpsc::Receiver<DataEvent>,
}

impl EventStream {
    pub fn new(receiver: mpsc::Receiver<DataEvent>) -> Self {
        Self { receiver }
    }

    /// Asynchronously receive the next event in the stream.
    pub async fn next(&mut self) -> Option<DataEvent> {
        self.receiver.recv().await
    }
}

impl Stream for EventStream {
    type Item = DataEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}
