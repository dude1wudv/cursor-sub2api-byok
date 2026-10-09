//! Installs and manages the local certificate authority.
use std::{fs, path::PathBuf};

#[cfg(target_os = "windows")]
mod windows;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, RsaKeySize, PKCS_RSA_SHA256,
};
use time::{Duration, OffsetDateTime};
use x509_parser::prelude::FromDer;

use crate::{Error, Result};

use super::CaState;

#[derive(Clone)]
pub struct CaManager {
    dir: PathBuf,
}

pub struct LoadedCa {
    pub issuer: Issuer<'static, KeyPair>,
}

impl CaManager {
    pub fn at(data_dir: &std::path::Path) -> Result<Self> {
        Ok(Self {
            dir: data_dir.join("ca"),
        })
    }

    fn cert_path(&self) -> PathBuf {
        self.dir.join("ca.crt")
    }
    fn key_path(&self) -> PathBuf {
        self.dir.join("ca.key.dpapi")
    }

    pub fn state(&self) -> Result<CaState> {
        let cert = fs::read_to_string(self.cert_path());
        let key = self.read_key();
        match (cert, key) {
            (Err(cert_error), Err(crate::Error::Io(key_error)))
                if cert_error.kind() == std::io::ErrorKind::NotFound
                    && key_error.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(CaState::Missing)
            }
            (Ok(cert), Ok(key)) => {
                if parse_issuer(&cert, &key).is_err() {
                    return Ok(CaState::Invalid);
                }
                Ok(if is_installed(&cert)? {
                    CaState::Ready
                } else {
                    CaState::Untrusted
                })
            }
            _ => Ok(CaState::Invalid),
        }
    }

    pub fn load(&self) -> Result<LoadedCa> {
        let cert = fs::read_to_string(self.cert_path())?;
        let key = self.read_key()?;
        Ok(LoadedCa {
            issuer: parse_issuer(&cert, &key)?,
        })
    }

    fn read_key(&self) -> Result<String> {
        String::from_utf8(super::secrets::unprotect(&fs::read(self.key_path())?)?)
            .map_err(|_| Error::Config("CA private key is not UTF-8".into()))
    }
    pub fn der(&self) -> Result<Vec<u8>> {
        pem::parse(fs::read(self.cert_path())?)
            .map(|cert| cert.into_contents())
            .map_err(|error| Error::Config(format!("parse CA PEM: {error}")))
    }
    pub fn fingerprint(&self) -> Result<String> {
        use sha2::{Digest, Sha256};
        Ok(hex::encode_upper(Sha256::digest(self.der()?)))
    }
    pub fn install_current_user(&self) -> Result<bool> {
        self.load()?;
        #[cfg(windows)]
        {
            windows::install(&self.der()?)
        }
        #[cfg(not(windows))]
        {
            Err(Error::Config(
                "Windows CurrentUser trust is required".into(),
            ))
        }
    }
    pub fn remove_current_user(expected_der: &[u8]) -> Result<()> {
        #[cfg(windows)]
        {
            windows::remove(expected_der)
        }
        #[cfg(not(windows))]
        {
            let _ = expected_der;
            Err(Error::Config(
                "Windows CurrentUser trust is required".into(),
            ))
        }
    }
    pub fn trusted(der: &[u8]) -> Result<bool> {
        #[cfg(windows)]
        {
            windows::contains(der)
        }
        #[cfg(not(windows))]
        {
            let _ = der;
            Err(Error::Config(
                "Windows CurrentUser trust is required".into(),
            ))
        }
    }

    pub fn initialize_local(&self) -> Result<()> {
        match self.state()? {
            CaState::Invalid => {
                return Err(Error::Config("CA files are incomplete or invalid".into()))
            }
            CaState::Ready => return Ok(()),
            CaState::Missing => self.generate()?,
            CaState::Untrusted => {}
        }
        Ok(())
    }

    fn generate(&self) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        #[cfg(unix)]
        fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700))?;

        let key = KeyPair::generate_rsa_for(&PKCS_RSA_SHA256, RsaKeySize::_3072)
            .map_err(|error| Error::Config(format!("generate CA key: {error}")))?;
        let mut params = CertificateParams::new(Vec::<String>::new())
            .map_err(|error| Error::Config(format!("create CA parameters: {error}")))?;
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, "Cursor Sub2API BYOK Local CA");
        name.push(DnType::OrganizationName, "Cursor Sub2API BYOK");
        params.distinguished_name = name;
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
        ];
        params.not_before = OffsetDateTime::now_utc() - Duration::minutes(5);
        params.not_after = OffsetDateTime::now_utc() + Duration::days(3652);
        let cert = params
            .self_signed(&key)
            .map_err(|error| Error::Config(format!("generate CA certificate: {error}")))?;
        write_atomic(
            &self.key_path(),
            &super::secrets::protect(key.serialize_pem().as_bytes())?,
            0o600,
        )?;
        write_atomic(&self.cert_path(), cert.pem().as_bytes(), 0o644)?;
        Ok(())
    }
}

fn parse_issuer(cert: &str, key: &str) -> Result<Issuer<'static, KeyPair>> {
    let key =
        KeyPair::from_pem(key).map_err(|error| Error::Config(format!("parse CA key: {error}")))?;
    let pem = pem::parse(cert).map_err(|error| Error::Config(format!("parse CA PEM: {error}")))?;
    let (_, parsed) = x509_parser::certificate::X509Certificate::from_der(pem.contents())
        .map_err(|error| Error::Config(format!("parse CA X.509 certificate: {error}")))?;
    if parsed.public_key().subject_public_key.data.as_ref() != key.public_key_raw() {
        return Err(Error::Config(
            "CA certificate and private key do not match".into(),
        ));
    }
    if !parsed.validity().is_valid() {
        return Err(Error::Config(
            "CA certificate is outside its validity period".into(),
        ));
    }
    if !parsed
        .basic_constraints()
        .map_err(|error| Error::Config(format!("read CA constraints: {error}")))?
        .is_some_and(|constraints| constraints.value.ca)
    {
        return Err(Error::Config("certificate is not a CA".into()));
    }
    Issuer::from_ca_cert_pem(cert, key)
        .map_err(|error| Error::Config(format!("parse CA certificate: {error}")))
}

fn write_atomic(path: &std::path::Path, data: &[u8], _mode: u32) -> Result<()> {
    super::atomic_file::write(path, data)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(_mode))?;
    Ok(())
}

#[cfg(windows)]
fn is_installed(cert: &str) -> Result<bool> {
    windows::is_installed(cert)
}
#[cfg(not(windows))]
fn is_installed(_cert: &str) -> Result<bool> {
    Err(Error::Config(
        "Windows CurrentUser trust is required".into(),
    ))
}
