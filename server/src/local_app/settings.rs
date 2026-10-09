//! Cursor settings JSONC parsing and reversible managed-key patches.
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use jsonc_parser::{
    cst::{CstContainerNode, CstInputValue, CstNode, CstRootNode},
    ParseOptions,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::atomic_file::write as write_atomic;
use crate::{Error, Result};

pub const NO_PROXY_KEY: &str = "http.noProxy";
pub const KEYS: [&str; 5] = [
    "http.proxy",
    "http.proxyKerberosServicePrincipal",
    "http.proxySupport",
    "cursor.general.disableHttp2",
    "http.experimental.systemCertificatesV2",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreOutcome {
    Restored,
    RestoredWithUserChanges(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct SettingsPatch {
    path: PathBuf,
    original_exists: bool,
    original_bytes: Vec<u8>,
    original_sha256: [u8; 32],
    patched_bytes: Option<Vec<u8>>,
    proxy_url: Option<String>,
}

impl SettingsPatch {
    pub fn prepare(path: &Path) -> Result<Self> {
        let (original_exists, original_bytes) = match fs::read(path) {
            Ok(bytes) => (true, bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (false, Vec::new()),
            Err(error) => return Err(error.into()),
        };
        if original_exists && fs::metadata(path)?.permissions().readonly() {
            return Err(Error::Config("Cursor settings is read-only".into()));
        }
        if original_exists && !original_bytes.is_empty() {
            let text = String::from_utf8(original_bytes.clone())
                .map_err(|_| Error::Config("Cursor settings must be UTF-8 JSONC".into()))?;
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            let root = CstRootNode::parse(text, &ParseOptions::default())
                .map_err(|error| Error::Config(format!("parse Cursor settings JSONC: {error}")))?;
            let object = root.object_value().ok_or_else(|| {
                Error::Config("Cursor settings JSONC root must be an object".into())
            })?;
            let mut names = HashSet::new();
            for child in object.children_exclude_trivia_and_tokens() {
                if let CstNode::Container(CstContainerNode::ObjectProp(property)) = child {
                    let name = property
                        .name()
                        .and_then(|name| name.decoded_value().ok())
                        .ok_or_else(|| {
                            Error::Config(
                                "Cursor settings contains an invalid property name".into(),
                            )
                        })?;
                    if !names.insert(name.clone()) {
                        return Err(Error::Config(format!(
                            "Cursor settings contains duplicate property: {name}"
                        )));
                    }
                }
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            original_exists,
            original_sha256: sha256(&original_bytes),
            original_bytes,
            patched_bytes: None,
            proxy_url: None,
        })
    }

    pub fn set_proxy_url(&mut self, proxy_url: &str) -> Result<()> {
        let text = String::from_utf8(self.original_bytes.clone())
            .map_err(|_| Error::Config("Cursor settings must be UTF-8 JSONC".into()))?;
        let had_bom = text.starts_with('\u{feff}');
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let root = CstRootNode::parse(text, &ParseOptions::default())
            .map_err(|error| Error::Config(format!("parse Cursor settings JSONC: {error}")))?;
        let object = root.object_value_or_set();
        set_string(&object, KEYS[0], proxy_url);
        set_string(&object, KEYS[1], proxy_url);
        set_string(&object, KEYS[2], "on");
        set_bool(&object, KEYS[3], true);
        set_bool(&object, KEYS[4], true);
        if let Some(property) = object.get(NO_PROXY_KEY) {
            property.set_value(CstInputValue::Array(vec![]));
        } else {
            object.append(NO_PROXY_KEY, CstInputValue::Array(vec![]));
        }
        let mut output = root.to_string().into_bytes();
        if had_bom {
            output.splice(0..0, [0xEF, 0xBB, 0xBF]);
        }
        self.proxy_url = Some(proxy_url.to_string());
        self.patched_bytes = Some(output);
        Ok(())
    }

    pub fn apply(&self) -> Result<()> {
        let patched = self
            .patched_bytes
            .as_ref()
            .ok_or_else(|| Error::Config("settings patch has no prepared write".into()))?;
        self.ensure_unchanged()?;
        write_atomic(&self.path, patched)
    }

    #[cfg(test)]
    pub fn restore(&self) -> Result<RestoreOutcome> {
        let patched = self
            .patched_bytes
            .as_ref()
            .ok_or_else(|| Error::Config("settings patch has no prepared write".into()))?;
        let current = read_optional(&self.path)?;
        let current = current.as_deref().unwrap_or_default();
        if sha256(&current) == sha256(patched) && current == patched.as_slice() {
            if self.original_exists {
                write_atomic(&self.path, &self.original_bytes)?;
            } else if self.path.exists() {
                fs::remove_file(&self.path)?;
            }
            return Ok(RestoreOutcome::Restored);
        }
        restore_bytes(
            &self.path,
            self.original_exists,
            &self.original_bytes,
            patched,
            self.proxy_url.as_deref().unwrap_or_default(),
        )
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn original_exists(&self) -> bool {
        self.original_exists
    }
    pub fn original_bytes(&self) -> &[u8] {
        &self.original_bytes
    }
    pub fn patched_bytes(&self) -> Option<&[u8]> {
        self.patched_bytes.as_deref()
    }
    pub fn proxy_url(&self) -> Option<&str> {
        self.proxy_url.as_deref()
    }
    fn ensure_unchanged(&self) -> Result<()> {
        let current = read_optional(&self.path)?;
        if current.is_some() != self.original_exists
            || sha256(current.as_deref().unwrap_or_default()) != self.original_sha256
        {
            return Err(Error::ControllerConflict {
                code: "CURSOR_SETTINGS_CHANGED",
                message: "Cursor settings changed before takeover could be applied".into(),
            });
        }
        Ok(())
    }
}

fn set_string(object: &jsonc_parser::cst::CstObject, name: &str, value: &str) {
    if let Some(property) = object.get(name) {
        property.set_value(CstInputValue::String(value.to_string()));
    } else {
        object.append(name, CstInputValue::String(value.to_string()));
    }
}

fn set_bool(object: &jsonc_parser::cst::CstObject, name: &str, value: bool) {
    if let Some(property) = object.get(name) {
        property.set_value(CstInputValue::Bool(value));
    } else {
        object.append(name, CstInputValue::Bool(value));
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn settings_match(path: &Path, proxy_url: &str) -> Result<bool> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let value = parse_value(&bytes)?;
    Ok(value.get(KEYS[0]) == Some(&Value::String(proxy_url.into()))
        && value.get(KEYS[1]) == Some(&Value::String(proxy_url.into()))
        && value.get(KEYS[2]) == Some(&Value::String("on".into()))
        && value.get(KEYS[3]) == Some(&Value::Bool(true))
        && value.get(KEYS[4]) == Some(&Value::Bool(true))
        && value.get(NO_PROXY_KEY) == Some(&serde_json::json!([])))
}

/// NotFound is the only read error that means the file is absent.
pub(super) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn parse_value(bytes: &[u8]) -> Result<Value> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::Config("Cursor settings must be UTF-8 JSONC".into()))?;
    jsonc_parser::parse_to_serde_value(
        text.trim_start_matches('\u{feff}'),
        &ParseOptions::default(),
    )
    .map_err(|error| Error::Config(format!("parse Cursor settings JSONC: {error}")))?
    .filter(Value::is_object)
    .ok_or_else(|| Error::Config("Cursor settings JSONC root must be an object".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bom_crlf_comments_and_unmanaged_values_survive_patch_and_restore() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = "\u{feff}{\r\n  // keep this comment\r\n  \"editor.fontSize\": 17, /* keep inline */\r\n  \"http.proxy\": \"http://old:80\",\r\n  \"http.noProxy\": [\"localhost\"],\r\n}\r\n";
        fs::write(&path, original).unwrap();
        let mut patch = SettingsPatch::prepare(&path).unwrap();
        patch.set_proxy_url("http://127.0.0.1:12345").unwrap();
        patch.apply().unwrap();
        let patched = fs::read_to_string(&path).unwrap();
        assert!(patched.starts_with('\u{feff}'));
        assert!(patched.contains("// keep this comment\r\n"));
        assert!(patched.contains("/* keep inline */"));
        assert!(!patched.replace("\r\n", "").contains('\n'));
        assert_eq!(
            parse_value(patched.as_bytes()).unwrap()["editor.fontSize"],
            17
        );
        assert!(settings_match(&path, "http://127.0.0.1:12345").unwrap());
        patch.restore().unwrap();
        assert_eq!(fs::read(&path).unwrap(), original.as_bytes());
    }

    #[test]
    fn rejects_duplicates_invalid_json_and_non_object_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        for content in [r#"{"a":1,"a":2}"#, "[1]", "{broken", "null"] {
            fs::write(&path, content).unwrap();
            assert!(SettingsPatch::prepare(&path).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), content);
        }
    }

    #[test]
    fn prepare_apply_conflict_preserves_external_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut patch = SettingsPatch::prepare(&path).unwrap();
        patch.set_proxy_url("http://127.0.0.1:12345").unwrap();
        fs::write(&path, "{}").unwrap();
        assert!(patch.apply().is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{}");
    }

    #[test]
    fn read_failure_is_not_an_empty_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_optional(dir.path()).is_err());
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct ManagedValue {
    pub present: bool,
    pub value: Value,
}

pub(super) fn managed_values(
    bytes: &[u8],
) -> Result<std::collections::BTreeMap<String, ManagedValue>> {
    let value = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        parse_value(bytes)?
    };
    Ok(KEYS
        .into_iter()
        .chain([NO_PROXY_KEY])
        .map(|key| {
            (
                key.into(),
                ManagedValue {
                    present: value.get(key).is_some(),
                    value: value.get(key).cloned().unwrap_or(Value::Null),
                },
            )
        })
        .collect())
}

pub(super) fn restore_bytes(
    path: &Path,
    original_exists: bool,
    original: &[u8],
    patched: &[u8],
    proxy_url: &str,
) -> Result<RestoreOutcome> {
    let current = read_optional(path)?;
    if current.is_some() == original_exists && current.as_deref().unwrap_or_default() == original {
        return Ok(RestoreOutcome::Restored);
    }
    if current.as_deref() == Some(patched) && !proxy_url.is_empty() {
        SettingsPatch::prepare(path).map_err(|_| recovery_error())?;
        if read_optional(path)? != current {
            return Err(recovery_error());
        }
        if original_exists {
            write_atomic(path, original)?;
        } else {
            fs::remove_file(path)?;
        }
        return Ok(RestoreOutcome::Restored);
    }
    let Some(bytes) = current.as_deref() else {
        return Err(recovery_error());
    };
    // Reuse strict prepare validation, including duplicate keys and read-only files.
    SettingsPatch::prepare(path).map_err(|_| recovery_error())?;
    let before = managed_values(original)?;
    let written = managed_values(patched)?;
    let actual = managed_values(bytes).map_err(|_| recovery_error())?;
    let text = std::str::from_utf8(bytes).map_err(|_| recovery_error())?;
    let bom = text.starts_with('\u{feff}');
    let root = CstRootNode::parse(
        text.trim_start_matches('\u{feff}'),
        &ParseOptions::default(),
    )
    .map_err(|_| recovery_error())?;
    let object = root.object_value().ok_or_else(recovery_error)?;
    let mut warnings = Vec::new();
    for key in KEYS.into_iter().chain([NO_PROXY_KEY]) {
        if actual[key] == before[key] {
            continue;
        }
        if actual[key] == written[key] && !proxy_url.is_empty() {
            if before[key].present {
                let value = cst_value(&before[key].value);
                if let Some(prop) = object.get(key) {
                    prop.set_value(value);
                } else {
                    object.append(key, value);
                }
            } else if let Some(prop) = object.get(key) {
                prop.remove();
            }
        } else {
            if matches!(key, "http.proxy" | "http.proxyKerberosServicePrincipal")
                && actual[key]
                    .value
                    .as_str()
                    .is_some_and(|value| same_proxy_port(value, proxy_url))
            {
                return Err(recovery_error());
            }
            warnings.push(key.to_string());
        }
    }
    let mut output = root.to_string();
    if bom {
        output.insert(0, '\u{feff}');
    }
    if read_optional(path)? != current {
        return Err(recovery_error());
    }
    if output.as_bytes() != bytes {
        write_atomic(path, output.as_bytes())?;
    }
    Ok(RestoreOutcome::RestoredWithUserChanges(warnings))
}

fn recovery_error() -> Error {
    Error::ControllerConflict { code: "RECOVERY_REQUIRED", message: "Cursor settings cannot be restored safely; preserve the journal and resolve the settings conflict".into() }
}

fn same_proxy_port(value: &str, proxy: &str) -> bool {
    match (url::Url::parse(value), url::Url::parse(proxy)) {
        (Ok(a), Ok(b)) => {
            (a.host_str() == b.host_str() || (loopback(&a) && loopback(&b)))
                && a.port_or_known_default() == b.port_or_known_default()
        }
        _ => false,
    }
}

fn cst_value(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(v) => CstInputValue::Bool(*v),
        Value::Number(v) => CstInputValue::Number(v.to_string()),
        Value::String(v) => CstInputValue::String(v.clone()),
        Value::Array(v) => CstInputValue::Array(v.iter().map(cst_value).collect()),
        Value::Object(v) => {
            CstInputValue::Object(v.iter().map(|(k, v)| (k.clone(), cst_value(v))).collect())
        }
    }
}

fn loopback(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(host)) => host == "localhost",
        _ => false,
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn third_values_and_unrelated_edits_survive_compare_restore() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original="\u{feff}{\r\n// retained\r\n\"http.proxy\":null,\r\n\"http.noProxy\":[\"old\"],\r\n\"editor.fontSize\":12\r\n}";
        fs::write(&path, original).unwrap();
        let mut patch = SettingsPatch::prepare(&path).unwrap();
        patch.set_proxy_url("http://127.0.0.1:12345").unwrap();
        patch.apply().unwrap();
        let edited = fs::read_to_string(&path)
            .unwrap()
            .replace("\"editor.fontSize\":12", "\"editor.fontSize\":19")
            .replace(
                "\"http.proxySupport\": \"on\"",
                "\"http.proxySupport\": \"off\"",
            );
        fs::write(&path, &edited).unwrap();
        let outcome = patch.restore().unwrap();
        assert!(matches!(
            outcome,
            RestoreOutcome::RestoredWithUserChanges(_)
        ));
        let after = fs::read_to_string(&path).unwrap();
        let value = parse_value(after.as_bytes()).unwrap();
        assert_eq!(value["http.proxy"], Value::Null);
        assert_eq!(value["http.noProxy"], serde_json::json!(["old"]));
        assert_eq!(value["editor.fontSize"], 19);
        assert_eq!(value["http.proxySupport"], "off");
        assert!(after.starts_with('\u{feff}') && after.contains("// retained\r\n"));
        assert!(!after.replace("\r\n", "").contains('\n'));
    }
    #[test]
    fn ambiguous_owned_proxy_and_readonly_file_are_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "{}").unwrap();
        let mut patch = SettingsPatch::prepare(&path).unwrap();
        patch.set_proxy_url("http://127.0.0.1:12345").unwrap();
        patch.apply().unwrap();
        let edited = fs::read_to_string(&path)
            .unwrap()
            .replace("http://127.0.0.1:12345", "http://localhost:12345/changed");
        fs::write(&path, &edited).unwrap();
        assert!(patch.restore().is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), edited);
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();
        assert!(SettingsPatch::prepare(&path).is_err());
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).unwrap();
    }
}
