//! Maps request IDs to active transport handles.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use tokio::sync::{mpsc, Mutex, Notify};

use crate::{
    cursor::{
        conversation::ConversationRegistry, prompting::PromptCompiler,
        services::observability::CursorTraceService,
    },
    plugin::PluginRegistry,
    provider::Provider,
    search::WebCache,
    store::Store,
    Result,
};

use super::{OutputHub, TransportHandle};

#[derive(Clone)]
pub struct TransportRegistry {
    inner: Arc<RegistryInner>,
}

struct RegistryInner {
    local: Mutex<HashMap<String, LocalTransport>>,
    next_local_generation: AtomicU64,
    upstream: Mutex<HashMap<String, u64>>,
    finished: parking_lot::Mutex<HashMap<String, tokio::time::Instant>>,
    rejected: parking_lot::Mutex<HashMap<String, (tokio::time::Instant, &'static str)>>,
    route_changed: Notify,
    store: Store,
    traces: CursorTraceService,
    web_cache: WebCache,
    plugins: Option<PluginRegistry>,
    conversations: ConversationRegistry,
}

#[derive(Clone)]
struct LocalTransport {
    generation: u64,
    handle: TransportHandle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportRoute {
    Local,
    Upstream(u64),
}

impl TransportRegistry {
    pub fn new(store: Store, provider: Arc<dyn Provider>, compiler: PromptCompiler) -> Self {
        Self::with_web_cache(store, provider, compiler, WebCache::default())
    }

    pub fn with_web_cache(
        store: Store,
        provider: Arc<dyn Provider>,
        compiler: PromptCompiler,
        web_cache: WebCache,
    ) -> Self {
        Self::build(store, provider, compiler, web_cache, None)
    }

    pub fn with_plugins(
        store: Store,
        provider: Arc<dyn Provider>,
        compiler: PromptCompiler,
        web_cache: WebCache,
        plugins: PluginRegistry,
    ) -> Self {
        Self::build(store, provider, compiler, web_cache, Some(plugins))
    }

    fn build(
        store: Store,
        provider: Arc<dyn Provider>,
        compiler: PromptCompiler,
        web_cache: WebCache,
        plugins: Option<PluginRegistry>,
    ) -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                local: Mutex::new(HashMap::new()),
                next_local_generation: AtomicU64::new(1),
                upstream: Mutex::new(HashMap::new()),
                finished: parking_lot::Mutex::new(HashMap::new()),
                rejected: parking_lot::Mutex::new(HashMap::new()),
                route_changed: Notify::new(),
                traces: CursorTraceService::new(store.clone()),
                conversations: ConversationRegistry::new(
                    store.clone(),
                    provider,
                    compiler,
                    web_cache.clone(),
                ),
                store,
                web_cache,
                plugins,
            }),
        }
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    pub fn trace(
        &self,
        request_id: &str,
    ) -> crate::cursor::services::observability::CursorTraceRecorder {
        self.inner.traces.recorder(request_id)
    }

    pub fn web_cache(&self) -> &WebCache {
        &self.inner.web_cache
    }

    pub fn plugins(&self) -> Option<&PluginRegistry> {
        self.inner.plugins.as_ref()
    }

    pub fn conversations(&self) -> &ConversationRegistry {
        &self.inner.conversations
    }

    pub async fn get_or_create(&self, request_id: &str) -> Result<TransportHandle> {
        self.get_or_create_for_append(request_id).await
    }

    pub(crate) async fn get_or_create_for_append(
        &self,
        request_id: &str,
    ) -> Result<TransportHandle> {
        let mut local = self.inner.local.lock().await;
        if let Some(transport) = local.get(request_id) {
            // Higher sequence actions still share the live actor's OrderedInbox.
            // Never replace a closing actor with an empty inbox on a late replay.
            return if transport.handle.accepting_appends() {
                Ok(transport.handle.clone())
            } else {
                Err(crate::Error::RunNotFound(request_id.into()))
            };
        }
        let previously_executed: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs WHERE cursor_request_id = ?)")
                .bind(request_id)
                .fetch_one(self.inner.store.pool())
                .await?;
        if self.inner.finished.lock().contains_key(request_id) || previously_executed {
            return Err(crate::Error::RunNotFound(request_id.into()));
        }
        self.inner.rejected.lock().remove(request_id);
        let (commands, receiver) = mpsc::channel(128);
        let output = Arc::new(OutputHub::default());
        let trace = self.inner.traces.recorder(request_id);
        trace.resume();
        let handle = TransportHandle::new(request_id.into(), commands, output, trace);
        let generation = self
            .inner
            .next_local_generation
            .fetch_add(1, Ordering::Relaxed);
        local.insert(
            request_id.into(),
            LocalTransport {
                generation,
                handle: handle.clone(),
            },
        );
        drop(local);
        self.inner.route_changed.notify_waiters();
        self.inner
            .conversations
            .bind_transport(handle.clone(), receiver);

        let registry = Arc::downgrade(&self.inner);
        let request_id = request_id.to_string();
        let lifecycle = handle.clone();
        tokio::spawn(async move {
            lifecycle.wait_transport_closed().await;
            if let Some(registry) = registry.upgrade() {
                let mut local = registry.local.lock().await;
                if local
                    .get(&request_id)
                    .is_some_and(|transport| transport.generation == generation)
                {
                    let mut finished = registry.finished.lock();
                    let now = tokio::time::Instant::now();
                    finished.retain(|_, at| now.duration_since(*at).as_secs() < 60);
                    if finished.len() >= 1024 {
                        if let Some(oldest) = finished
                            .iter()
                            .min_by_key(|(_, at)| **at)
                            .map(|(id, _)| id.clone())
                        {
                            finished.remove(&oldest);
                        }
                    }
                    finished.insert(request_id.clone(), now);
                    local.remove(&request_id);
                    registry.route_changed.notify_waiters();
                }
            }
        });
        Ok(handle)
    }

    pub async fn local(&self, request_id: &str) -> Option<TransportHandle> {
        self.inner
            .local
            .lock()
            .await
            .get(request_id)
            .map(|transport| transport.handle.clone())
    }

    pub async fn mark_upstream(&self, request_id: &str) {
        let mut upstream = self.inner.upstream.lock().await;
        let generation = upstream.get(request_id).copied().unwrap_or_default() + 1;
        upstream.insert(request_id.into(), generation);
        drop(upstream);
        self.inner.route_changed.notify_waiters();
    }

    pub async fn upstream(&self, request_id: &str) -> bool {
        self.inner.upstream.lock().await.contains_key(request_id)
    }

    pub async fn wait_route(&self, request_id: &str) -> Result<TransportRoute> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let changed = self.inner.route_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.inner.local.lock().await.contains_key(request_id) {
                return Ok(TransportRoute::Local);
            }
            if let Some(generation) = self.inner.upstream.lock().await.get(request_id).copied() {
                return Ok(TransportRoute::Upstream(generation));
            }
            if self.inner.finished.lock().contains_key(request_id) {
                return Err(crate::Error::RunNotFound(request_id.into()));
            }
            if let Some((_, reason)) = self.inner.rejected.lock().get(request_id) {
                return Err(crate::Error::Protocol((*reason).into()));
            }
            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                return Err(crate::Error::RunNotFound(request_id.into()));
            }
        }
    }

    /// Wake a RunSSE already waiting for a rejected initial append. Keep only
    /// fixed diagnostic reasons, never request bodies or credential material.
    pub async fn reject_unrouted(&self, request_id: &str, reason: &'static str) {
        if self.local(request_id).await.is_some() || self.upstream(request_id).await {
            return;
        }
        let mut rejected = self.inner.rejected.lock();
        let now = tokio::time::Instant::now();
        rejected.retain(|_, (at, _)| now.duration_since(*at).as_secs() < 60);
        if rejected.len() >= 1024 {
            if let Some(oldest) = rejected
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(id, _)| id.clone())
            {
                rejected.remove(&oldest);
            }
        }
        rejected.insert(request_id.into(), (now, reason));
        self.inner.route_changed.notify_waiters();
    }

    pub fn finish_upstream(&self, request_id: String, generation: u64) {
        let registry = self.clone();
        tokio::spawn(async move {
            let mut upstream = registry.inner.upstream.lock().await;
            if upstream.get(&request_id) == Some(&generation) {
                upstream.remove(&request_id);
            }
        });
    }

    pub async fn shutdown(&self) {
        self.inner.conversations.shutdown().await;
        let handles = std::mem::take(&mut *self.inner.local.lock().await);
        self.inner.upstream.lock().await.clear();
        for transport in handles.into_values() {
            transport.handle.disconnect().await;
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                transport.handle.wait_transport_closed(),
            )
            .await;
        }
    }
}
