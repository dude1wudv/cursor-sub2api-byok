//! CurrentUser Root access. Certificates are compared by exact DER, never subject name.
use crate::{Error, Result};
use std::{io, ptr, slice};
use windows_sys::Win32::{
    Foundation::{GetLastError, CRYPT_E_NOT_FOUND},
    Security::Cryptography::*,
};

const ROOT_STORE: [u16; 5] = [b'R' as u16, b'o' as u16, b'o' as u16, b't' as u16, 0];

struct Store(HCERTSTORE);
impl Drop for Store {
    fn drop(&mut self) {
        unsafe {
            CertCloseStore(self.0, 0);
        }
    }
}

impl Store {
    fn current_user(readonly: bool) -> Result<Self> {
        let flags = CERT_SYSTEM_STORE_CURRENT_USER
            | CERT_STORE_OPEN_EXISTING_FLAG
            | if readonly {
                CERT_STORE_READONLY_FLAG
            } else {
                0
            };
        let store = unsafe {
            CertOpenStore(
                CERT_STORE_PROV_SYSTEM_W,
                0,
                0,
                flags,
                ROOT_STORE.as_ptr().cast(),
            )
        };
        if store.is_null() {
            return Err(last_error("open Windows CurrentUser Root store"));
        }
        Ok(Self(store))
    }

    /// Returns an owned context. Enumeration frees each previous context.
    fn find(&self, der: &[u8]) -> Result<Option<*const CERT_CONTEXT>> {
        let mut context = ptr::null();
        loop {
            context = unsafe { CertEnumCertificatesInStore(self.0, context) };
            if context.is_null() {
                let error = unsafe { GetLastError() };
                return if error == CRYPT_E_NOT_FOUND as u32 {
                    Ok(None)
                } else {
                    Err(Error::Io(io::Error::from_raw_os_error(error as i32)))
                };
            }
            let encoded = unsafe {
                slice::from_raw_parts((*context).pbCertEncoded, (*context).cbCertEncoded as usize)
            };
            if encoded == der {
                return Ok(Some(context));
            }
        }
    }

    fn contains(&self, der: &[u8]) -> Result<bool> {
        if let Some(context) = self.find(der)? {
            unsafe {
                CertFreeCertificateContext(context);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn install(&self, der: &[u8]) -> Result<()> {
        if self.contains(der)? {
            return Ok(());
        }
        if unsafe {
            CertAddEncodedCertificateToStore(
                self.0,
                X509_ASN_ENCODING,
                der.as_ptr(),
                der.len() as u32,
                CERT_STORE_ADD_NEW,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(last_error("install exact CA in CurrentUser Root"));
        }
        if !self.contains(der)? {
            return Err(Error::Config("CA install could not be verified".into()));
        }
        Ok(())
    }

    fn remove(&self, der: &[u8]) -> Result<()> {
        while let Some(context) = self.find(der)? {
            // The API consumes the context even on failure.
            if unsafe { CertDeleteCertificateFromStore(context) } == 0 {
                return Err(last_error("remove exact CA from CurrentUser Root"));
            }
        }
        Ok(())
    }
}

fn last_error(action: &str) -> Error {
    Error::Config(format!("{action}: {}", io::Error::last_os_error()))
}

fn certificate_der(cert: &str) -> Result<Vec<u8>> {
    pem::parse(cert)
        .map(|pem| pem.into_contents())
        .map_err(|error| Error::Config(format!("parse CA PEM: {error}")))
}

pub(super) fn is_installed(cert: &str) -> Result<bool> {
    Store::current_user(true)?.contains(&certificate_der(cert)?)
}

pub(super) fn install(der: &[u8]) -> Result<bool> {
    let store = Store::current_user(false)?;
    let already_present = store.contains(der)?;
    tracing::info!(already_present, "checking CurrentUser Root CA before install");
    if !already_present {
        store.install(der)?;
    }
    // A context visible in the write handle is not proof of durable system trust.
    drop(store);
    let trusted = contains(der)?;
    tracing::info!(trusted, "verified CurrentUser Root CA with fresh read handle");
    require_persisted_trust(trusted)?;
    Ok(!already_present)
}
fn require_persisted_trust(trusted: bool) -> Result<()> {
    if trusted { Ok(()) } else {
        Err(Error::Config("Windows 未保留证书信任，接管未开启。请完成系统安装确认后重试。".into()))
    }
}

pub(super) fn remove(der: &[u8]) -> Result<()> {
    Store::current_user(false)?.remove(der)
}
pub(super) fn contains(der: &[u8]) -> Result<bool> {
    Store::current_user(true)?.contains(der)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ca() -> Vec<u8> {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Synthetic same-name CA");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let key = rcgen::KeyPair::generate().unwrap();
        params.self_signed(&key).unwrap().der().to_vec()
    }

    #[test]
    fn a_successful_write_without_persisted_trust_is_rejected() {
        assert!(require_persisted_trust(false).is_err());
        assert!(require_persisted_trust(true).is_ok());
    }

    #[test]
    fn install_and_remove_match_exact_der_in_isolated_memory_store() {
        let handle = unsafe {
            CertOpenStore(
                CERT_STORE_PROV_MEMORY,
                0,
                0,
                CERT_STORE_CREATE_NEW_FLAG,
                ptr::null(),
            )
        };
        assert!(!handle.is_null());
        let store = Store(handle);
        let ours = test_ca();
        let unrelated_same_name = test_ca();
        assert!(!store.contains(&ours).unwrap());
        store.install(&ours).unwrap();
        store.install(&ours).unwrap();
        store.install(&unrelated_same_name).unwrap();
        assert!(store.contains(&ours).unwrap());
        store.remove(&ours).unwrap();
        store.remove(&ours).unwrap();
        assert!(!store.contains(&ours).unwrap());
        assert!(store.contains(&unrelated_same_name).unwrap());
    }
}
