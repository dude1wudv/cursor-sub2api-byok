//! Persistent CurrentUser trust is opt-in, with a DPAPI receipt for exact cleanup.
use super::CaManager;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{fs, io::ErrorKind};

#[derive(Serialize, Deserialize)]
struct Consent {
    version: u32,
    accepted_at_ms: i64,
    ca_der: Vec<u8>,
}

impl CaManager {
    fn consent_path(&self) -> std::path::PathBuf { self.dir.join("trust-consent.dpapi") }
    fn read_consent(&self) -> Result<Option<Consent>> {
        let bytes = match fs::read(self.consent_path()) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let record: Consent = serde_json::from_slice(&super::super::secrets::unprotect(&bytes)?)
            .map_err(|_| Error::Config("证书同意记录无效，请保留数据目录后检查".into()))?;
        if record.version != 1 || record.ca_der.is_empty() {
            return Err(Error::Config("证书同意记录版本或证书无效".into()));
        }
        Ok(Some(record))
    }
    pub fn consent_accepted(&self) -> Result<bool> {
        match self.read_consent()? {
            None => Ok(false),
            Some(record) => Ok(record.ca_der == self.der()?),
        }
    }
    pub fn accept_persistent_trust(&self) -> Result<()> {
        // Never overwrite an unreadable receipt or lose the DER needed for cleanup.
        let prior = self.read_consent()?;
        self.initialize_local()?;
        let ca_der = self.der()?;
        if prior.as_ref().is_some_and(|record| record.ca_der != ca_der) {
            return Err(Error::Config("CA 与同意记录不一致，请先卸载旧证书".into()));
        }
        if prior.is_none() {
            let record = Consent { version: 1, accepted_at_ms: chrono::Utc::now().timestamp_millis(), ca_der };
            // Receipt precedes the OS write, so a crash always leaves exact cleanup evidence.
            super::super::atomic_file::write(&self.consent_path(), &super::super::secrets::protect(&serde_json::to_vec(&record)?)?)?;
        }
        self.install_current_user()?;
        Ok(())
    }
    pub fn uninstall_persistent_trust(&self) -> Result<()> {
        if let Some(record) = self.read_consent()? {
            Self::remove_current_user(&record.ca_der)?;
            // Keep the receipt if Windows declines deletion; a retry is safe.
            fs::remove_file(self.consent_path())?;
        }
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn receipt_is_dpapi_protected_bound_to_der_and_corruption_is_retained() {
        let dir = tempfile::tempdir().unwrap();
        let ca = CaManager::at(dir.path()).unwrap();
        ca.initialize_local().unwrap();
        assert!(!ca.consent_accepted().unwrap());
        let der = ca.der().unwrap();
        let record = Consent { version: 1, accepted_at_ms: 1, ca_der: der.clone() };
        let raw = serde_json::to_vec(&record).unwrap();
        super::super::super::atomic_file::write(&ca.consent_path(), &super::super::super::secrets::protect(&raw).unwrap()).unwrap();
        assert_ne!(fs::read(ca.consent_path()).unwrap(), raw);
        assert!(ca.consent_accepted().unwrap());
        fs::write(ca.cert_path(), pem::encode(&pem::Pem::new("CERTIFICATE", b"other".to_vec()))).unwrap();
        assert!(!ca.consent_accepted().unwrap());
        fs::write(ca.consent_path(), b"corrupt").unwrap();
        assert!(ca.accept_persistent_trust().is_err());
        assert!(ca.uninstall_persistent_trust().is_err());
        assert_eq!(fs::read(ca.consent_path()).unwrap(), b"corrupt");
    }
}
