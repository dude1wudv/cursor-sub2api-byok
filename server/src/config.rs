//! Loads and validates process-level server configuration.
use std::{env, fs, net::SocketAddr, path::PathBuf, time::Duration};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::{Error, Result};

const DATA_DIR_NAME: &str = "CursorSub2APIByok";
const DATABASE_FILE_NAME: &str = "cursor-sub2api.db";
const DEFAULT_CURSOR_USER_DATA_DIR: &str = "Cursor";
const DEFAULT_CURSOR_SETTINGS_FILE_NAME: &str = "settings.json";
const DEFAULT_PROVIDER_REQUEST_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const DEFAULT_PROVIDER_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimePaths {
    pub data_dir: PathBuf,
    pub cursor_settings: PathBuf,
}

impl RuntimePaths {
    pub fn resolve(
        data_dir: Option<PathBuf>,
        cursor_user_data_dir: Option<PathBuf>,
    ) -> Result<Self> {
        let data_dir = match data_dir {
            Some(path) => path,
            None => PathBuf::from(env::var_os("LOCALAPPDATA").ok_or_else(|| {
                Error::Config("LOCALAPPDATA is required for Cursor Sub2API BYOK".into())
            })?)
            .join(DATA_DIR_NAME),
        };
        validate_path(&data_dir, "data directory", true)?;

        let cursor_settings = match cursor_user_data_dir {
            Some(user_data_dir) => {
                validate_path(&user_data_dir, "Cursor user data directory", true)?;
                user_data_dir
                    .join("User")
                    .join(DEFAULT_CURSOR_SETTINGS_FILE_NAME)
            }
            None => {
                let app_data = env::var_os("APPDATA").ok_or_else(|| {
                    Error::Config("APPDATA is required for Cursor settings".into())
                })?;
                PathBuf::from(app_data)
                    .join(DEFAULT_CURSOR_USER_DATA_DIR)
                    .join("User")
                    .join(DEFAULT_CURSOR_SETTINGS_FILE_NAME)
            }
        };
        validate_path(&cursor_settings, "Cursor settings path", false)?;
        let data_dir = canonical_target(&data_dir)?;
        let cursor_settings = canonical_target(&cursor_settings)?;
        #[cfg(windows)]
        let (compare_data, compare_settings) = (
            PathBuf::from(data_dir.to_string_lossy().to_lowercase()),
            PathBuf::from(cursor_settings.to_string_lossy().to_lowercase()),
        );
        #[cfg(not(windows))]
        let (compare_data, compare_settings) = (data_dir.clone(), cursor_settings.clone());
        if compare_data == compare_settings
            || compare_data.starts_with(&compare_settings)
            || compare_settings.starts_with(&compare_data)
        {
            return Err(Error::Config(
                "data directory and Cursor settings path must be different".into(),
            ));
        }
        Ok(Self {
            data_dir,
            cursor_settings,
        })
    }
}

fn canonical_target(path: &std::path::Path) -> Result<PathBuf> {
    let existing = path
        .ancestors()
        .find(|part| part.exists())
        .ok_or_else(|| Error::Config("path has no existing parent".into()))?;
    let canonical = fs::canonicalize(existing)?;
    #[cfg(windows)]
    let canonical = {
        let value = canonical.to_string_lossy();
        if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{rest}"))
        } else {
            PathBuf::from(value.strip_prefix(r"\\?\").unwrap_or(&value))
        }
    };
    let suffix = path
        .strip_prefix(existing)
        .map_err(|_| Error::Config("path parent changed".into()))?;
    Ok(if suffix.as_os_str().is_empty() {
        canonical
    } else {
        canonical.join(suffix)
    })
}

pub(crate) fn validate_path(path: &std::path::Path, label: &str, directory: bool) -> Result<()> {
    if !path.is_absolute() {
        return Err(Error::Config(format!("{label} must be an absolute path")));
    }
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(Error::Config(format!(
            "{label} cannot contain parent-directory components"
        )));
    }
    for ancestor in path.ancestors() {
        let metadata = match fs::symlink_metadata(ancestor) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        #[cfg(windows)]
        let reparse = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let reparse = metadata.file_type().is_symlink();
        if reparse {
            return Err(Error::Config(format!(
                "{label} cannot traverse a reparse point"
            )));
        }
        if (ancestor != path || directory) && !metadata.is_dir() {
            return Err(Error::Config(format!(
                "{label} must have directory parents"
            )));
        }
        if ancestor == path && !directory && !metadata.is_file() {
            return Err(Error::Config(format!("{label} must be a regular file")));
        }
    }
    Ok(())
}

pub fn managed_data_dir() -> Result<PathBuf> {
    let root = env::var_os("LOCALAPPDATA")
        .ok_or_else(|| Error::Config("LOCALAPPDATA is required for Cursor Sub2API BYOK".into()))?;
    let data_dir = PathBuf::from(root).join(DATA_DIR_NAME);
    fs::create_dir_all(&data_dir)?;
    #[cfg(unix)]
    fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o700))?;
    Ok(data_dir)
}

pub fn default_cursor_settings_path() -> Result<PathBuf> {
    let app_data = env::var_os("APPDATA")
        .ok_or_else(|| Error::Config("APPDATA is required for Cursor settings".into()))?;
    Ok(PathBuf::from(app_data)
        .join(DEFAULT_CURSOR_USER_DATA_DIR)
        .join("User")
        .join(DEFAULT_CURSOR_SETTINGS_FILE_NAME))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    OpenAiChat,
    OpenAiResponses,
    Anthropic,
}

#[derive(Clone)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub request_url: String,
    pub api_key: String,
    pub custom_headers: reqwest::header::HeaderMap,
    pub max_output_tokens: Option<u64>,
    pub request_timeout: Duration,
    pub allowed_body_fields: Option<std::collections::HashSet<String>>,
}

pub struct Config {
    pub listen_addr: SocketAddr,
    pub database_url: String,
    pub provider_request_timeout: Duration,
    pub provider_stream_idle_timeout: Duration,
    pub console: Option<ConsoleSource>,
    pub use_persisted_ports: bool,
    pub app_version: String,
    pub runtime_paths: RuntimePaths,
}

#[derive(Clone)]
pub enum ConsoleSource {
    Directory(PathBuf),
    Proxy(url::Url),
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let listen_addr = env::var("CURSOR_LISTEN_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:3000".into())
            .parse()
            .map_err(|error| Error::Config(format!("invalid CURSOR_LISTEN_ADDR: {error}")))?;
        let request_timeout = match env::var("CURSOR_PROVIDER_TIMEOUT_SECONDS") {
            Ok(value) => Duration::from_secs(value.parse().map_err(|error| {
                Error::Config(format!("invalid CURSOR_PROVIDER_TIMEOUT_SECONDS: {error}"))
            })?),
            Err(env::VarError::NotPresent) => DEFAULT_PROVIDER_REQUEST_TIMEOUT,
            Err(error) => {
                return Err(Error::Config(format!(
                    "invalid CURSOR_PROVIDER_TIMEOUT_SECONDS: {error}"
                )))
            }
        };
        let console_dir = env::var_os("CURSOR_CONSOLE_DIR").map(PathBuf::from);
        let console_proxy = env::var("CURSOR_CONSOLE_PROXY")
            .ok()
            .map(|value| {
                value.parse().map_err(|error| {
                    Error::Config(format!("invalid CURSOR_CONSOLE_PROXY: {error}"))
                })
            })
            .transpose()?;
        let console = match (console_dir, console_proxy) {
            (Some(_), Some(_)) => {
                return Err(Error::Config(
                    "CURSOR_CONSOLE_DIR and CURSOR_CONSOLE_PROXY cannot both be set".into(),
                ))
            }
            (Some(directory), None) => Some(ConsoleSource::Directory(directory)),
            (None, Some(proxy)) => Some(ConsoleSource::Proxy(proxy)),
            (None, None) => None,
        };
        let runtime_paths = RuntimePaths::resolve(None, None)?;
        Ok(Self {
            listen_addr,
            database_url: database_url_from_env()?,
            provider_request_timeout: request_timeout,
            provider_stream_idle_timeout: DEFAULT_PROVIDER_STREAM_IDLE_TIMEOUT,
            console,
            use_persisted_ports: false,
            app_version: env!("CARGO_PKG_VERSION").into(),
            runtime_paths,
        })
    }

    pub fn desktop() -> Result<Self> {
        Self::desktop_with_paths(RuntimePaths::resolve(None, None)?)
    }

    pub fn desktop_with_paths(runtime_paths: RuntimePaths) -> Result<Self> {
        Ok(Self {
            listen_addr: "127.0.0.1:0"
                .parse()
                .expect("desktop listen address is static"),
            database_url: database_url_for_dir(&runtime_paths.data_dir)?,
            provider_request_timeout: DEFAULT_PROVIDER_REQUEST_TIMEOUT,
            provider_stream_idle_timeout: DEFAULT_PROVIDER_STREAM_IDLE_TIMEOUT,
            console: None,
            use_persisted_ports: true,
            app_version: env!("CARGO_PKG_VERSION").into(),
            runtime_paths,
        })
    }
}

fn database_url_from_env() -> Result<String> {
    match env::var("CURSOR_DATABASE_URL") {
        Ok(database_url) => Ok(database_url),
        Err(env::VarError::NotPresent) => default_database_url(),
        Err(error) => Err(Error::Config(format!(
            "invalid CURSOR_DATABASE_URL: {error}"
        ))),
    }
}

fn default_database_url() -> Result<String> {
    let data_dir = managed_data_dir()?;
    database_url_for_dir(&data_dir)
}

fn database_url_for_dir(data_dir: &std::path::Path) -> Result<String> {
    let database_path = data_dir.join(DATABASE_FILE_NAME);
    let database_path = database_path
        .to_str()
        .ok_or_else(|| Error::Config("database path is not valid UTF-8".into()))?;
    Ok(format!("sqlite://{database_path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_cursor_user_data_directory_uses_user_settings_and_accepts_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("controller");
        let cursor = dir.path().join("cursor-profile");
        fs::create_dir_all(cursor.join("User")).unwrap();
        let settings = cursor.join("User/settings.json");
        fs::write(&settings, "{}").unwrap();
        let paths = RuntimePaths::resolve(Some(data.clone()), Some(cursor)).unwrap();
        assert_eq!(
            fs::canonicalize(&paths.cursor_settings).unwrap(),
            fs::canonicalize(&settings).unwrap()
        );
        assert_eq!(paths.data_dir.file_name(), data.file_name());
        assert_eq!(
            fs::canonicalize(paths.data_dir.parent().unwrap()).unwrap(),
            fs::canonicalize(data.parent().unwrap()).unwrap()
        );
        assert_eq!(fs::read_to_string(settings).unwrap(), "{}");
    }

    #[test]
    fn runtime_paths_reject_relative_parent_and_overlapping_paths() {
        let dir = tempfile::tempdir().unwrap();
        let cursor = dir.path().join("cursor");
        for data in [
            PathBuf::from("relative"),
            dir.path().join("nested/../data"),
            dir.path().to_path_buf(),
        ] {
            assert!(RuntimePaths::resolve(Some(data), Some(cursor.clone())).is_err());
        }
    }

    #[test]
    fn provider_timeout_defaults_match_runtime_boundaries() {
        assert_eq!(
            DEFAULT_PROVIDER_STREAM_IDLE_TIMEOUT,
            Duration::from_secs(30 * 60)
        );
        assert_eq!(
            DEFAULT_PROVIDER_REQUEST_TIMEOUT,
            Duration::from_secs(60 * 60)
        );
    }
}
