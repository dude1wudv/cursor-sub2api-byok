//! Reversible, serialized Cursor takeover. All persisted secrets use CurrentUser DPAPI.
mod atomic_file;
mod ca;
pub mod instance;
mod journal;
mod process;
mod proxy;
pub(crate) mod secrets;
mod settings;

use self::{ca::CaManager, proxy::ProxyRuntime};
use crate::{store::Store, Error, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, MutexGuard};

pub(crate) fn proxy_host_allowed(host: &str) -> bool {
    proxy::is_cursor_host(host)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaState {
    Missing,
    Untrusted,
    Ready,
    Invalid,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationState {
    Disabled,
    Enabled,
    Degraded,
    RecoveryRequired,
}
#[derive(Clone, Debug, Serialize)]
pub struct CursorHarnessStatus {
    pub platform: &'static str,
    pub ca: CaState,
    pub ca_sha256: Option<String>,
    pub configured_models: usize,
    pub enabled_models: usize,
    pub integration: IntegrationState,
    pub settings_applied: bool,
    pub settings_path: PathBuf,
    pub proxy_url: Option<String>,
    pub restart_required: bool,
    pub recovery_error: Option<String>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetEnabled {
    pub enabled: bool,
}
#[derive(Clone)]
pub struct CursorHarness {
    inner: Arc<Inner>,
}
struct Inner {
    store: Store,
    settings_path: PathBuf,
    journal_path: PathBuf,
    ca: CaManager,
    transition: Mutex<()>,
    backend_addr: RwLock<Option<SocketAddr>>,
    proxy: Mutex<ProxyRuntime>,
    recovery_error: RwLock<Option<String>>,
    warnings: RwLock<Vec<String>>,
    #[cfg(test)]
    cursor_running_override: RwLock<Option<bool>>,
}

fn conflict(code: &'static str, message: &str) -> Error {
    Error::ControllerConflict {
        code,
        message: message.into(),
    }
}
async fn require_cursor_closed() -> Result<()> {
    if process::cursor_running().await? {
        return Err(conflict(
            "CURSOR_RESTART_REQUIRED",
            "请保存工作并完全退出 Cursor，再更改接管状态。",
        ));
    }
    Ok(())
}
fn check_target(record: &journal::Record, target: &Path) -> Result<()> {
    if record.target_path != target {
        return Err(conflict(
            "RECOVERY_REQUIRED",
            "journal 的 Cursor 路径与本次启动参数不一致，请使用原来的路径参数恢复。",
        ));
    }
    Ok(())
}
fn restore_ca(record: &journal::Record) -> Result<()> {
    if record.ca_install_intended && !record.ca_prior_trust {
        CaManager::remove_current_user(&record.ca_der)?;
    }
    Ok(())
}

/// Offline recovery entry point; it constructs no database, HTTP client or proxy.
pub async fn restore_paths(paths: &crate::config::RuntimePaths) -> Result<()> {
    let path = journal::path(&paths.data_dir);
    let Some(mut record) = journal::read(&path)? else {
        return Ok(());
    };
    check_target(&record, &paths.cursor_settings)?;
    require_cursor_closed().await?;
    record.stage = journal::Stage::Restoring;
    journal::write(&path, &record)?;
    journal::restore(&record)?;
    restore_ca(&record)?;
    journal::remove(&path)
}

impl CursorHarness {
    async fn cursor_running(&self) -> Result<bool> {
        #[cfg(test)]
        if let Some(value) = *self.inner.cursor_running_override.read() {
            return Ok(value);
        }
        process::cursor_running().await
    }
    async fn require_cursor_closed(&self) -> Result<()> {
        if self.cursor_running().await? {
            return Err(conflict(
                "CURSOR_RESTART_REQUIRED",
                "请完全退出 Cursor 后重试。",
            ));
        }
        Ok(())
    }

    pub fn new(store: Store, settings_path: PathBuf, data_dir: PathBuf) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(Inner {
                store,
                settings_path,
                journal_path: journal::path(&data_dir),
                ca: CaManager::at(&data_dir)?,
                transition: Mutex::new(()),
                backend_addr: RwLock::new(None),
                proxy: Mutex::new(ProxyRuntime::default()),
                recovery_error: RwLock::new(None),
                warnings: RwLock::new(Vec::new()),
                #[cfg(test)]
                cursor_running_override: RwLock::new(None),
            }),
        })
    }
    pub fn set_backend_addr(&self, addr: SocketAddr) {
        *self.inner.backend_addr.write() = Some(addr);
    }
    pub async fn proxy_port(&self) -> Option<u16> {
        self.inner.proxy.lock().await.port()
    }

    pub async fn status(&self) -> Result<CursorHarnessStatus> {
        let configured_models = self.inner.store.models().await?.len();
        let ca = self.inner.ca.state()?;
        let pending = journal::read(&self.inner.journal_path);
        let proxy = self.inner.proxy.lock().await;
        let proxy_url = proxy.url();
        let matched = proxy_url
            .as_deref()
            .map(|url| settings::settings_match(&self.inner.settings_path, url))
            .transpose();
        let settings_applied = matched
            .as_ref()
            .ok()
            .and_then(|value| *value)
            .unwrap_or(false);
        let recovery_error = self
            .inner
            .recovery_error
            .read()
            .clone()
            .or_else(|| {
                pending
                    .as_ref()
                    .err()
                    .map(|_| "journal 无法读取或解密，已保留以供恢复。".into())
            })
            .or_else(|| {
                matched
                    .as_ref()
                    .err()
                    .map(|_| "Cursor settings 无法解析，需要恢复。".into())
            })
            .or_else(|| {
                if matches!(pending, Ok(Some(_))) && !proxy.running() {
                    Some("发现未完成的接管，请完全退出 Cursor 后恢复。".into())
                } else {
                    None
                }
            });
        let integration = if recovery_error.is_some() {
            IntegrationState::RecoveryRequired
        } else if proxy.running() && settings_applied {
            IntegrationState::Enabled
        } else if proxy.running() || settings_applied {
            IntegrationState::Degraded
        } else {
            IntegrationState::Disabled
        };
        drop(proxy);
        Ok(CursorHarnessStatus {
            platform: std::env::consts::OS,
            ca,
            ca_sha256: self.inner.ca.fingerprint().ok(),
            configured_models,
            enabled_models: configured_models,
            integration,
            settings_applied,
            settings_path: self.inner.settings_path.clone(),
            proxy_url,
            restart_required: self.cursor_running().await?,
            recovery_error,
            warnings: self.inner.warnings.read().clone(),
        })
    }

    /// The same guard serializes connection/model writes with takeover transitions.
    pub async fn configuration_guard(&self) -> Result<MutexGuard<'_, ()>> {
        let guard = self.inner.transition.lock().await;
        if self.inner.proxy.lock().await.running() {
            return Err(conflict(
                "TAKEOVER_ACTIVE",
                "请先关闭 Cursor 接管，再修改连接或模型。",
            ));
        }
        if journal::read(&self.inner.journal_path)?.is_some() {
            return Err(conflict("RECOVERY_REQUIRED", "请先恢复未完成的接管。"));
        }
        Ok(guard)
    }

    pub async fn initialize_ca(&self) -> Result<CursorHarnessStatus> {
        let _guard = self.configuration_guard().await?;
        self.inner.ca.initialize_local()?;
        self.status().await
    }

    pub async fn recover_pending(&self) -> Result<()> {
        let _guard = self.inner.transition.lock().await;
        self.restore_transaction().await
    }
    pub async fn set_enabled(&self, enabled: bool) -> Result<CursorHarnessStatus> {
        let _guard = self.inner.transition.lock().await;
        self.require_cursor_closed().await?;
        if enabled {
            self.enable().await?;
        } else {
            self.restore_transaction().await?;
        }
        self.status().await
    }
    pub async fn disable(&self) -> Result<()> {
        let _guard = self.inner.transition.lock().await;
        self.require_cursor_closed().await?;
        self.restore_transaction().await
    }

    async fn enable(&self) -> Result<()> {
        if self.inner.proxy.lock().await.running() {
            if matches!(journal::read(&self.inner.journal_path)?, Some(ref record) if matches!(record.stage, journal::Stage::Active) && settings::settings_match(&self.inner.settings_path, &record.proxy_url).unwrap_or(false))
            {
                return Ok(());
            }
            return Err(conflict("RECOVERY_REQUIRED", "当前接管事务未完成。"));
        }
        if journal::read(&self.inner.journal_path)?.is_some() {
            return Err(conflict("RECOVERY_REQUIRED", "请先恢复未完成的接管。"));
        }
        self.require_cursor_closed().await?;
        if !self.inner.store.sub2api_connection().await?.has_api_key
            || self.inner.store.models().await?.is_empty()
        {
            return Err(Error::Config(
                "至少配置一个有效 Sub2API 模型后才能开启接管。".into(),
            ));
        }
        let backend = self
            .inner
            .backend_addr
            .read()
            .ok_or_else(|| Error::Config("management server is not ready".into()))?;
        let mut patch = settings::SettingsPatch::prepare(&self.inner.settings_path)?;
        self.inner.ca.initialize_local()?;
        let ca_der = self.inner.ca.der()?;
        let prior = CaManager::trusted(&ca_der)?;
        let mut record = journal::from_patch(&patch, ca_der, prior)?;
        journal::write(&self.inner.journal_path, &record)?;
        let result: Result<()> = async {
            if !prior {
                self.inner.ca.install_current_user()?;
            }
            record.stage = journal::Stage::CaInstalled;
            journal::write(&self.inner.journal_path, &record)?;
            let ca = self.inner.ca.load()?;
            let port = self.inner.store.port_settings().await?.proxy_port;
            let (url, port) = self
                .inner
                .proxy
                .lock()
                .await
                .start(backend, ca, port)
                .await?;
            patch.set_proxy_url(&url)?;
            journal::set_patch(&mut record, &patch)?;
            journal::write(&self.inner.journal_path, &record)?;
            patch.apply()?;
            record.stage = journal::Stage::SettingsApplied;
            journal::write(&self.inner.journal_path, &record)?;
            self.inner.store.set_proxy_port(port).await?;
            self.inner.store.set_cursor_takeover_enabled(true).await?;
            record.stage = journal::Stage::Active;
            journal::write(&self.inner.journal_path, &record)?;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            if self.restore_transaction().await.is_err() {
                return Err(conflict(
                    "RECOVERY_REQUIRED",
                    "接管失败且恢复未完成，请保留 journal 后重试恢复。",
                ));
            }
            return Err(error);
        }
        *self.inner.recovery_error.write() = None;
        self.inner.warnings.write().clear();
        Ok(())
    }

    async fn restore_transaction(&self) -> Result<()> {
        let result: Result<()> = async {
            let Some(mut record) = journal::read(&self.inner.journal_path)? else {
                if self.inner.proxy.lock().await.running() {
                    return Err(conflict(
                        "RECOVERY_REQUIRED",
                        "代理仍在运行，但恢复 journal 缺失；退出已阻止。",
                    ));
                }
                self.inner.store.set_cursor_takeover_enabled(false).await?;
                return Ok(());
            };
            check_target(&record, &self.inner.settings_path)?;
            self.require_cursor_closed().await?;
            record.stage = journal::Stage::Restoring;
            journal::write(&self.inner.journal_path, &record)?;
            if let settings::RestoreOutcome::RestoredWithUserChanges(keys) =
                journal::restore(&record)?
            {
                *self.inner.warnings.write() = keys;
            }
            self.inner.proxy.lock().await.stop().await;
            restore_ca(&record)?;
            self.inner.store.set_cursor_takeover_enabled(false).await?;
            journal::remove(&self.inner.journal_path)?;
            Ok(())
        }
        .await;
        match &result {
            Ok(()) => *self.inner.recovery_error.write() = None,
            Err(Error::ControllerConflict {
                code: "CURSOR_RESTART_REQUIRED",
                ..
            }) => {}
            Err(_) => {
                *self.inner.recovery_error.write() =
                    Some("恢复未完成，journal 已保留；请解决冲突后重试。".into())
            }
        }
        result
    }
}

#[cfg(test)]
mod tests;
