//! Current-user DPAPI helpers for secrets that must never be persisted as plaintext.

use base64::{engine::general_purpose::STANDARD, Engine};

use crate::{Error, Result};

const PREFIX: &str = "dpapi:v1:";

pub fn protect(bytes: &[u8]) -> Result<Vec<u8>> {
    #[cfg(target_os = "windows")]
    {
        use std::ptr;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{
            CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        };

        let input = CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        let ok = unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(Error::Config(format!(
                "DPAPI protect failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        let result =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
        unsafe {
            LocalFree(output.pbData as _);
        }
        Ok(result)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = bytes;
        Err(Error::Config("DPAPI is supported only on Windows".into()))
    }
}

pub fn unprotect(bytes: &[u8]) -> Result<Vec<u8>> {
    #[cfg(target_os = "windows")]
    {
        use std::ptr;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{
            CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        };

        let input = CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(Error::Config(format!(
                "DPAPI unprotect failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        let result =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
        unsafe {
            LocalFree(output.pbData as _);
        }
        Ok(result)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = bytes;
        Err(Error::Config("DPAPI is supported only on Windows".into()))
    }
}

pub fn protect_string(value: &str) -> Result<String> {
    Ok(format!(
        "{PREFIX}{}",
        STANDARD.encode(protect(value.as_bytes())?)
    ))
}

pub fn unprotect_string(value: &str) -> Result<String> {
    let encoded = value
        .strip_prefix(PREFIX)
        .ok_or_else(|| Error::Config("secret is not a supported DPAPI value".into()))?;
    let encrypted = STANDARD
        .decode(encoded)
        .map_err(|_| Error::Config("secret DPAPI encoding is invalid".into()))?;
    let plaintext = unprotect(&encrypted)?;
    String::from_utf8(plaintext)
        .map_err(|_| Error::Config("secret DPAPI plaintext is not UTF-8".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn non_windows_never_falls_back_to_plaintext() {
        #[cfg(not(target_os = "windows"))]
        assert!(protect_string("synthetic-secret").is_err());
    }
    #[cfg(windows)]
    #[test]
    fn dpapi_string_roundtrip_and_corruption_detection() {
        let secret = "synthetic-secret-测试";
        let protected = protect_string(secret).unwrap();
        assert!(!protected.contains(secret));
        assert_eq!(unprotect_string(&protected).unwrap(), secret);
        assert!(unprotect_string("plaintext").is_err());
        assert!(unprotect_string("dpapi:v1:!!!!").is_err());
        let mut corrupt = protect(secret.as_bytes()).unwrap();
        let index = corrupt.len() - 1;
        corrupt[index] ^= 0xff;
        assert!(unprotect(&corrupt).is_err());
    }
}
