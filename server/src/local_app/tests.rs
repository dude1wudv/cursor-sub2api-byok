//! Fault/restart tests use a synthetic process state and proxy task; no real CA store writes.
use super::*;
use std::fs;

async fn harness(dir: &Path) -> CursorHarness {
    let store = Store::connect(&format!("sqlite://{}", dir.join("test.db").display()))
        .await
        .unwrap();
    let h = CursorHarness::new(store, dir.join("settings.json"), dir.to_path_buf()).unwrap();
    *h.inner.cursor_running_override.write() = Some(false);
    h
}

#[tokio::test]
async fn cursor_running_blocks_enable_disable_and_exit_without_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path()).await;
    *h.inner.cursor_running_override.write() = Some(true);
    for result in [
        h.set_enabled(true).await.map(|_| ()),
        h.set_enabled(false).await.map(|_| ()),
        h.disable().await,
    ] {
        assert!(matches!(
            result,
            Err(Error::ControllerConflict {
                code: "CURSOR_RESTART_REQUIRED",
                ..
            })
        ));
    }
    assert!(!h.inner.settings_path.exists());
    assert!(!h.inner.journal_path.exists());
    assert!(h.proxy_port().await.is_none());
    h.inner.store.pool().close().await;
}

#[tokio::test]
async fn active_is_idempotent_and_failed_restore_keeps_proxy_and_journal() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path()).await;
    fs::write(&h.inner.settings_path, b"{\r\n// keep\r\n}\r\n").unwrap();
    let mut patch = settings::SettingsPatch::prepare(&h.inner.settings_path).unwrap();
    patch.set_proxy_url("http://127.0.0.1:12345").unwrap();
    patch.apply().unwrap();
    let mut record = journal::from_patch(&patch, vec![], true).unwrap();
    record.stage = journal::Stage::Active;
    journal::write(&h.inner.journal_path, &record).unwrap();
    *h.inner.proxy.lock().await = ProxyRuntime::fixture();
    assert!(matches!(h.status().await.unwrap().integration, IntegrationState::Degraded),
        "a running proxy with matching settings cannot be healthy without a trusted CA");
    let journal_before = fs::read(&h.inner.journal_path).unwrap();
    h.set_enabled(true).await.unwrap();
    h.set_enabled(true).await.unwrap();
    assert_eq!(fs::read(&h.inner.journal_path).unwrap(), journal_before);
    assert_eq!(h.proxy_port().await, Some(12345));
    fs::write(&h.inner.settings_path, b"{invalid").unwrap();
    assert!(h.disable().await.is_err());
    assert!(matches!(
        h.status().await.unwrap().integration,
        IntegrationState::RecoveryRequired
    ));
    assert!(h.inner.journal_path.exists());
    assert_eq!(h.proxy_port().await, Some(12345));
    assert_eq!(fs::read(&h.inner.settings_path).unwrap(), b"{invalid");
    fs::write(&h.inner.settings_path, &record.patched_bytes).unwrap();
    h.disable().await.unwrap();
    h.disable().await.unwrap();
    assert_eq!(
        fs::read(&h.inner.settings_path).unwrap(),
        record.original_bytes
    );
    assert!(!h.inner.journal_path.exists());
    assert!(h.proxy_port().await.is_none());
    h.inner.store.pool().close().await;
}

#[tokio::test]
async fn restart_recovers_all_journal_stages_without_starting_proxy_and_retains_corruption() {
    for stage in [
        journal::Stage::Prepared,
        journal::Stage::CaInstalled,
        journal::Stage::SettingsApplied,
        journal::Stage::Active,
        journal::Stage::Restoring,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let h = harness(dir.path()).await;
        let mut patch = settings::SettingsPatch::prepare(&h.inner.settings_path).unwrap();
        let mut record = journal::from_patch(&patch, vec![], true).unwrap();
        if matches!(
            stage,
            journal::Stage::SettingsApplied | journal::Stage::Active | journal::Stage::Restoring
        ) {
            patch.set_proxy_url("http://127.0.0.1:12345").unwrap();
            journal::set_patch(&mut record, &patch).unwrap();
            patch.apply().unwrap();
        }
        record.stage = stage;
        journal::write(&h.inner.journal_path, &record).unwrap();
        h.recover_pending().await.unwrap();
        h.recover_pending().await.unwrap();
        assert!(!h.inner.settings_path.exists());
        assert!(!h.inner.journal_path.exists());
        assert!(h.proxy_port().await.is_none());
        fs::write(&h.inner.journal_path, b"corrupt").unwrap();
        assert!(h.recover_pending().await.is_err());
        assert_eq!(fs::read(&h.inner.journal_path).unwrap(), b"corrupt");
        h.inner.store.pool().close().await;
    }
}

#[tokio::test]
async fn persistent_trust_is_retained_across_disable_and_recovery() {
    for recovery in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let h = harness(dir.path()).await;
        let mut patch = settings::SettingsPatch::prepare(&h.inner.settings_path).unwrap();
        patch.set_proxy_url("http://127.0.0.1:12345").unwrap();
        patch.apply().unwrap();
        // An invalid DER proves restoration does not even call the native removal API.
        let mut record = journal::from_patch(&patch, b"synthetic-not-a-certificate".to_vec(), false).unwrap();
        record.ca_persistent_trust = true;
        record.stage = journal::Stage::Active;
        journal::write(&h.inner.journal_path, &record).unwrap();
        if recovery { h.recover_pending().await.unwrap(); } else { h.disable().await.unwrap(); }
        assert!(!h.inner.settings_path.exists());
        assert!(!h.inner.journal_path.exists());
        assert!(h.accept_certificate(false, 1).await.is_err());
        assert!(h.accept_certificate(true, 999).await.is_err());
        assert!(!h.inner.ca.consent_accepted().unwrap());
        h.inner.store.pool().close().await;
    }
}
