//! Crash-recovery journal for the reversible Cursor takeover transaction.
use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    atomic_file::write as write_atomic,
    secrets,
    settings::{RestoreOutcome, SettingsPatch},
};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const VERSION: u32 = 1;
const FILE_NAME: &str = "takeover-journal.dpapi";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Prepared,
    CaInstalled,
    SettingsApplied,
    Active,
    Restoring,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub version: u32,
    pub target_path: PathBuf,
    pub original_exists: bool,
    pub original_bytes: Vec<u8>,
    pub original_sha256: [u8; 32],
    pub patched_bytes: Vec<u8>,
    pub patched_sha256: [u8; 32],
    pub proxy_url: String,
    pub stage: Stage,
    pub managed_original: std::collections::BTreeMap<String, super::settings::ManagedValue>,
    pub managed_written: std::collections::BTreeMap<String, super::settings::ManagedValue>,
    pub ca_der: Vec<u8>,
    pub ca_sha256: [u8; 32],
    pub ca_prior_trust: bool,
    pub ca_install_intended: bool,
    /// Old journals still restore their original temporary trust transaction.
    #[serde(default)]
    pub ca_persistent_trust: bool,
}

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}
pub fn from_patch(patch: &SettingsPatch, ca_der: Vec<u8>, ca_prior_trust: bool) -> Result<Record> {
    let patched_bytes = patch
        .patched_bytes()
        .unwrap_or(patch.original_bytes())
        .to_vec();
    let proxy_url = patch.proxy_url().unwrap_or_default().to_string();
    Ok(Record {
        version: VERSION,
        target_path: patch.path().to_path_buf(),
        original_exists: patch.original_exists(),
        original_bytes: patch.original_bytes().to_vec(),
        original_sha256: sha256(patch.original_bytes()),
        patched_sha256: sha256(&patched_bytes),
        patched_bytes: patched_bytes.clone(),
        proxy_url,
        stage: Stage::Prepared,
        managed_original: super::settings::managed_values(patch.original_bytes())?,
        managed_written: super::settings::managed_values(&patched_bytes)?,
        ca_sha256: sha256(&ca_der),
        ca_der,
        ca_prior_trust,
        ca_install_intended: !ca_prior_trust,
        ca_persistent_trust: false,
    })
}

pub fn write(path: &Path, record: &Record) -> Result<()> {
    validate(record)?;
    let serialized = serde_json::to_vec(record)?;
    let protected = secrets::protect(&serialized)?;
    write_atomic(path, &protected)
}

pub fn read(path: &Path) -> Result<Option<Record>> {
    let protected = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let serialized = secrets::unprotect(&protected)?;
    let record: Record = serde_json::from_slice(&serialized)
        .map_err(|error| Error::Config(format!("invalid takeover journal: {error}")))?;
    validate(&record)?;
    Ok(Some(record))
}

fn validate(record: &Record) -> Result<()> {
    if record.version != VERSION
        || sha256(&record.original_bytes) != record.original_sha256
        || sha256(&record.patched_bytes) != record.patched_sha256
        || sha256(&record.ca_der) != record.ca_sha256
        || super::settings::managed_values(&record.original_bytes)? != record.managed_original
        || super::settings::managed_values(&record.patched_bytes)? != record.managed_written
        || !record.target_path.is_absolute()
    {
        return Err(Error::Config(
            "takeover journal integrity check failed".into(),
        ));
    }
    Ok(())
}

pub fn set_patch(record: &mut Record, patch: &SettingsPatch) -> Result<()> {
    record.patched_bytes = patch
        .patched_bytes()
        .ok_or_else(|| Error::Config("missing prepared settings".into()))?
        .to_vec();
    record.patched_sha256 = sha256(&record.patched_bytes);
    record.proxy_url = patch.proxy_url().unwrap_or_default().into();
    record.managed_written = super::settings::managed_values(&record.patched_bytes)?;
    Ok(())
}

pub fn restore(record: &Record) -> Result<RestoreOutcome> {
    validate(record)?;
    super::settings::restore_bytes(
        &record.target_path,
        record.original_exists,
        &record.original_bytes,
        &record.patched_bytes,
        &record.proxy_url,
    )
}

pub fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(path: &Path, original: Option<&[u8]>) -> (SettingsPatch, Record) {
        if let Some(bytes) = original {
            fs::write(path, bytes).unwrap();
        }
        let mut patch = SettingsPatch::prepare(path).unwrap();
        patch.set_proxy_url("http://127.0.0.1:12345").unwrap();
        let record = from_patch(&patch, vec![], true).unwrap();
        (patch, record)
    }
    #[test]
    fn every_stage_restores_idempotently_and_distinguishes_absent_file() {
        for stage in [
            Stage::Prepared,
            Stage::CaInstalled,
            Stage::SettingsApplied,
            Stage::Active,
            Stage::Restoring,
        ] {
            for original in [None, Some(b"{}".as_slice()), Some(b"".as_slice())] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("settings.json");
                let (patch, mut record) = fixture(&path, original);
                record.stage = stage.clone();
                if matches!(
                    stage,
                    Stage::SettingsApplied | Stage::Active | Stage::Restoring
                ) {
                    patch.apply().unwrap();
                }
                restore(&record).unwrap();
                restore(&record).unwrap();
                assert_eq!(
                    super::super::settings::read_optional(&path)
                        .unwrap()
                        .as_deref(),
                    original
                );
            }
        }
    }
    #[test]
    fn integrity_invalid_json_and_lost_file_fail_without_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let (patch, mut record) = fixture(&path, Some(b"{}"));
        patch.apply().unwrap();
        let written = fs::read(&path).unwrap();
        record.original_bytes = b"tampered".to_vec();
        assert!(restore(&record).is_err());
        assert_eq!(fs::read(&path).unwrap(), written);
        record.original_bytes = b"{}".to_vec();
        fs::write(&path, b"{broken").unwrap();
        assert!(restore(&record).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{broken");
        fs::remove_file(&path).unwrap();
        assert!(restore(&record).is_err());
        assert!(!path.exists());
    }
    #[cfg(windows)]
    #[test]
    fn dpapi_journal_replaces_existing_and_rejects_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.dpapi");
        let (_, mut record) = fixture(
            &dir.path().join("settings.json"),
            Some(br#"{"private":"synthetic-secret"}"#),
        );
        write(&path, &record).unwrap();
        assert!(!String::from_utf8_lossy(&fs::read(&path).unwrap()).contains("synthetic-secret"));
        record.stage = Stage::Restoring;
        write(&path, &record).unwrap();
        assert!(matches!(
            read(&path).unwrap().unwrap().stage,
            Stage::Restoring
        ));
        fs::write(&path, b"corrupt journal").unwrap();
        assert!(read(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"corrupt journal");
    }
}
