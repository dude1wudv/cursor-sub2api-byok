//! Provides the request-scoped input, subscription, and terminal interface.

use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    cursor::{
        conversation::TransportCommand,
        protocol::{connect, proto::agent::v1 as pb},
        services::observability::CursorTraceRecorder,
    },
    Error, Result,
};

use super::{OutputHub, TransportAdmission, TransportLifecycle};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportParent {
    pub request_id: String,
    pub tool_call_id: String,
}

#[derive(Clone)]
pub struct TransportHandle {
    request_id: String,
    commands: mpsc::Sender<TransportCommand>,
    output: Arc<OutputHub>,
    conversation_id: Arc<OnceLock<String>>,
    parent: Arc<OnceLock<TransportParent>>,
    trace: CursorTraceRecorder,
    lifecycle: TransportLifecycle,
    disconnect: CancellationToken,
    subscriptions: Arc<parking_lot::Mutex<HttpSubscriptions>>,
}

#[derive(Default)]
struct HttpSubscriptions {
    active: usize,
    generation: u64,
    expired: bool,
}

impl TransportHandle {
    pub(crate) fn new(
        request_id: String,
        commands: mpsc::Sender<TransportCommand>,
        output: Arc<OutputHub>,
        trace: CursorTraceRecorder,
    ) -> Self {
        Self {
            request_id,
            commands,
            output,
            conversation_id: Arc::new(OnceLock::new()),
            parent: Arc::new(OnceLock::new()),
            trace,
            lifecycle: TransportLifecycle::new(),
            disconnect: CancellationToken::new(),
            subscriptions: Arc::default(),
        }
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn set_conversation_id(&self, conversation_id: &str) -> Result<()> {
        if conversation_id.is_empty() {
            return Err(Error::Protocol("Cursor conversation id is required".into()));
        }
        if self
            .conversation_id
            .get()
            .is_some_and(|current| current != conversation_id)
        {
            return Err(Error::Protocol(format!(
                "conflicting conversation ids for request {}",
                self.request_id
            )));
        }
        let _ = self.conversation_id.set(conversation_id.into());
        Ok(())
    }

    pub fn conversation_id(&self) -> Option<&str> {
        self.conversation_id.get().map(String::as_str)
    }

    pub fn set_parent(&self, parent: TransportParent) -> Result<()> {
        if parent.request_id.is_empty() || parent.tool_call_id.is_empty() {
            return Err(Error::Protocol("Cursor parent ids are required".into()));
        }
        if self.parent.get().is_some_and(|current| current != &parent) {
            return Err(Error::Protocol(format!(
                "conflicting parent ids for request {}",
                self.request_id
            )));
        }
        let _ = self.parent.set(parent);
        Ok(())
    }

    pub fn parent(&self) -> Option<&TransportParent> {
        self.parent.get()
    }

    pub async fn command(&self, command: TransportCommand) -> Result<()> {
        self.commands
            .send(command)
            .await
            .map_err(|_| Error::RunNotFound(self.request_id.clone()))
    }

    pub async fn disconnect(&self) {
        let _ = self.commands.send(TransportCommand::Disconnect).await;
    }

    pub(crate) fn attach_http(&self) -> Result<()> {
        let mut state = self.subscriptions.lock();
        if state.expired {
            return Err(Error::RunNotFound(self.request_id.clone()));
        }
        state.active += 1;
        state.generation += 1;
        Ok(())
    }

    pub(crate) fn detach_http(&self, completed: bool) {
        let mut state = self.subscriptions.lock();
        state.active -= 1;
        if completed || state.active != 0 {
            return;
        }
        let generation = state.generation;
        let handle = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let _ = handle
                .command(TransportCommand::DisconnectIfUnsubscribed { generation })
                .await;
        });
    }

    // Checked by the actor, not the timer: a reconnect may arrive while the
    // disconnect command is queued. Attaching and expiry share the same lock.
    pub(crate) fn expire_http(&self, generation: u64) -> bool {
        let mut state = self.subscriptions.lock();
        if state.active != 0 || state.generation != generation {
            return false;
        }
        state.expired = true;
        true
    }

    pub fn subscribe(&self) -> tokio::sync::mpsc::UnboundedReceiver<Bytes> {
        self.output.subscribe()
    }

    pub fn emit_frame(&self, frame: Bytes) -> bool {
        self.output.emit(frame)
    }

    pub fn emit(&self, message: &pb::AgentServerMessage) -> Result<()> {
        if self.emit_frame(connect::encode_message(message)?) {
            Ok(())
        } else {
            Err(Error::RunNotFound(self.request_id.clone()))
        }
    }

    pub(crate) fn close_output(&self) -> bool {
        self.output.close()
    }

    pub(crate) fn trace(&self) -> Option<&CursorTraceRecorder> {
        Some(&self.trace)
    }

    pub(crate) fn accepting_appends(&self) -> bool {
        self.lifecycle.is_open()
    }

    pub(crate) fn admit(&self) -> Result<TransportAdmission> {
        self.lifecycle
            .admit()
            .ok_or_else(|| Error::RunNotFound(self.request_id.clone()))
    }

    pub(crate) fn begin_close(&self) {
        self.lifecycle.begin_close();
    }

    pub(crate) fn admissions_drained(&self) -> bool {
        self.lifecycle.admissions_drained()
    }

    pub(crate) async fn wait_admissions_drained(&self) {
        self.lifecycle.wait_admissions_drained().await;
    }

    pub(crate) fn mark_draining(&self) {
        self.lifecycle.mark_draining();
    }

    pub(crate) fn reopen(&self) {
        self.lifecycle.reopen();
    }

    pub(crate) fn close_transport(&self) {
        self.lifecycle.close();
    }

    pub(crate) async fn wait_transport_closed(&self) {
        self.lifecycle.wait_closed().await;
    }

    pub(crate) fn disconnect_token(&self) -> CancellationToken {
        self.disconnect.clone()
    }

    pub(crate) fn mark_disconnected(&self) {
        self.disconnect.cancel();
    }
}
