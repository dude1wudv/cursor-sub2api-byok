//! Explicit-path local subscription display override. Never switches credentials.
//! The caller owns lifecycle serialization and must ensure Cursor has fully exited.
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteSynchronous},
    Connection, Row, SqliteConnection,
};

use super::{atomic_file, secrets};
use crate::{Error, Result};

pub const KEYS: [&str; 2] = [
    "cursorAuth/stripeMembershipType",
    "cursorAuth/stripeSubscriptionStatus",
];
pub const AUTH_ID_KEY: &str = "cursorAuth/stripeMembershipAuthId";
pub const REACTIVE_USER_KEY: &str = "src.vs.platform.reactivestorage.browser.reactiveStorageServiceImpl.persistentStorage.applicationUser";
const LEAVES: [&str; 2] = ["membershipType", "subscriptionStatus"];
const OVERRIDES: [&str; 2] = ["ultra", "active"];
const MAX_JOURNAL_BYTES: u64 = 2 * 1024 * 1024;

/// Detect the fixed upstream's synthetic identity, without authenticating real tokens.
pub fn is_placeholder_token(token: &str) -> bool {
    let mut parts = token.split('.');
    let Some(payload) = parts.nth(1) else {
        return false;
    };
    if payload.len() > 64 * 1024 {
        return false;
    }
    let Ok(decoded) = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')) else {
        return false;
    };
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&decoded) else {
        return false;
    };
    payload.get("sub").and_then(serde_json::Value::as_str) == Some("cursor-local-user")
        && payload.get("iss").and_then(serde_json::Value::as_str) == Some("cursor-client")
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Value {
    Absent,
    Text(String),
    Blob(Vec<u8>),
    Other,
}
#[derive(Serialize, Deserialize)]
enum Leaf {
    Absent,
    Present(serde_json::Value),
}
impl Leaf {
    fn snapshot(value: Option<&serde_json::Value>) -> Self {
        value.cloned().map_or(Self::Absent, Self::Present)
    }
    fn as_value(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Absent => None,
            Self::Present(value) => Some(value),
        }
    }
}
#[derive(Serialize, Deserialize)]
enum Stage {
    Prepared,
    Active,
    Restoring,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    database: PathBuf,
    stage: Stage,
    original: [Value; 2],
    auth_id: Value,
    /// Only the two derived leaf snapshots; no account JSON or credential backup.
    reactive_original: [Leaf; 2],
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct RestoreOutcome {
    /// Key names only; original, injected and third-party values remain private.
    pub preserved_fields: Vec<String>,
    pub account_changed: bool,
}

fn failure(message: &'static str) -> Error {
    Error::Config(message.into())
}
fn sql_error(_: sqlx::Error) -> Error {
    // SQLite trigger messages can contain field values. Never forward DB diagnostics.
    failure("account database operation failed; close Cursor and retry recovery")
}

fn check_paths(database: &Path, journal: &Path) -> Result<()> {
    crate::config::validate_path(database, "account database", false)?;
    crate::config::validate_path(journal, "account journal", false)?;
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = database.as_os_str().to_os_string();
        sidecar.push(suffix);
        crate::config::validate_path(Path::new(&sidecar), "account database sidecar", false)?;
        if sidecar
            .to_string_lossy()
            .eq_ignore_ascii_case(&journal.to_string_lossy())
        {
            return Err(failure("account journal must not replace a SQLite sidecar"));
        }
    }
    if database
        .to_string_lossy()
        .eq_ignore_ascii_case(&journal.to_string_lossy())
        || database.parent() == Some(journal)
    {
        return Err(failure(
            "account database and journal paths must be separate",
        ));
    }
    Ok(())
}

async fn connect(database: &Path) -> Result<SqliteConnection> {
    // Open the live SQLite database, including its WAL; never copy a database file.
    // Do not change journal_mode, create a missing database, or migrate its schema.
    SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(database)
            .create_if_missing(false)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(Duration::from_secs(2)),
    )
    .await
    .map_err(sql_error)
}

async fn read_value(connection: &mut SqliteConnection, key: &str) -> Result<Value> {
    let row = sqlx::query("SELECT typeof(value) AS kind, value FROM ItemTable WHERE key = ?")
        .bind(key)
        .fetch_optional(connection)
        .await
        .map_err(sql_error)?;
    let Some(row) = row else {
        return Ok(Value::Absent);
    };
    match row
        .try_get::<String, _>("kind")
        .map_err(sql_error)?
        .as_str()
    {
        "text" => Ok(Value::Text(row.try_get("value").map_err(sql_error)?)),
        "blob" => Ok(Value::Blob(row.try_get("value").map_err(sql_error)?)),
        _ => Ok(Value::Other),
    }
}

async fn write_value(connection: &mut SqliteConnection, key: &str, value: &Value) -> Result<()> {
    match value {
        Value::Other => {
            return Err(failure(
                "account original has an unsupported SQLite storage type",
            ))
        }
        Value::Absent => {
            sqlx::query("DELETE FROM ItemTable WHERE key = ?")
                .bind(key)
                .execute(connection)
                .await
                .map_err(sql_error)?;
        }
        Value::Text(text) => {
            sqlx::query("INSERT INTO ItemTable(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
                .bind(key).bind(text).execute(connection).await.map_err(sql_error)?;
        }
        Value::Blob(bytes) => {
            sqlx::query("INSERT INTO ItemTable(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
                .bind(key).bind(bytes).execute(connection).await.map_err(sql_error)?;
        }
    }
    Ok(())
}

fn reactive_object(value: &Value) -> Result<serde_json::Map<String, serde_json::Value>> {
    let bytes = match value {
        Value::Absent => return Ok(serde_json::Map::new()),
        Value::Text(text) => text.as_bytes(),
        Value::Blob(bytes) => bytes,
        Value::Other => {
            return Err(failure(
                "derived account state has an unsupported SQLite type",
            ))
        }
    };
    let parsed: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| failure("derived account state is not valid JSON"))?;
    parsed
        .as_object()
        .cloned()
        .ok_or_else(|| failure("derived account state must be a JSON object"))
}

async fn restore_reactive(
    connection: &mut SqliteConnection,
    record: &Record,
    outcome: &mut RestoreOutcome,
) -> Result<()> {
    let current = read_value(connection, REACTIVE_USER_KEY).await?;
    if current == Value::Absent {
        return Ok(());
    }
    let Ok(mut object) = reactive_object(&current) else {
        // A third party replaced or damaged this row. Do not replace their object.
        outcome.preserved_fields.push(REACTIVE_USER_KEY.to_owned());
        return Ok(());
    };
    let mut changed = false;
    for (index, leaf) in LEAVES.iter().enumerate() {
        let value = object.get(*leaf);
        let original = record.reactive_original[index].as_value();
        if value.and_then(serde_json::Value::as_str) == Some(OVERRIDES[index]) {
            if value != original {
                match original {
                    Some(original) => {
                        object.insert((*leaf).to_owned(), original.clone());
                    }
                    None => {
                        object.remove(*leaf);
                    }
                }
                changed = true;
            }
        } else if value != original {
            outcome
                .preserved_fields
                .push(format!("{REACTIVE_USER_KEY}.{leaf}"));
        }
    }
    if changed {
        let bytes = serde_json::to_vec(&object)?;
        // Keep the current SQLite storage type; only the selected JSON leaves change.
        let value = match current {
            Value::Blob(_) => Value::Blob(bytes),
            _ => {
                Value::Text(String::from_utf8(bytes).map_err(|_| failure("invalid JSON encoding"))?)
            }
        };
        write_value(connection, REACTIVE_USER_KEY, &value).await?;
    }
    Ok(())
}

fn save(journal: &Path, record: &Record) -> Result<()> {
    crate::config::validate_path(journal, "account journal", false)?;
    let bytes = serde_json::to_vec(record)?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES / 2 {
        return Err(failure("account journal exceeds size limit"));
    }
    atomic_file::write(journal, &secrets::protect(&bytes)?)
}

fn load(journal: &Path) -> Result<Option<Record>> {
    crate::config::validate_path(journal, "account journal", false)?;
    let metadata = match fs::metadata(journal) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.len() > MAX_JOURNAL_BYTES {
        return Err(failure("account journal exceeds size limit"));
    }
    let bytes = secrets::unprotect(&fs::read(journal)?)?;
    let record: Record =
        serde_json::from_slice(&bytes).map_err(|_| failure("invalid account journal"))?;
    if record.version != 2 {
        return Err(failure("unsupported account journal version"));
    }
    Ok(Some(record))
}

/// Detects pending recovery without reading any Cursor profile or exposing fields.
pub fn pending(journal: &Path) -> Result<bool> {
    Ok(load(journal)?.is_some())
}

/// Explicit opt-in only. Caller must hold its lifecycle lock and enforce Cursor closed.
/// On any failure retain the journal and call restore before enabling again.
pub async fn inject(database: &Path, journal: &Path, consent: bool) -> Result<()> {
    if !consent {
        return Err(failure(
            "explicit local subscription display consent is required",
        ));
    }
    check_paths(database, journal)?;
    let mut connection = connect(database).await?;
    let mut transaction = connection
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(sql_error)?;
    // Check under the SQLite writer lock, preventing a second switch replacing history.
    if pending(journal)? {
        return Err(failure("account recovery is required before switching"));
    }
    let mut original = [Value::Absent, Value::Absent];
    for (index, key) in KEYS.iter().enumerate() {
        original[index] = read_value(&mut transaction, key).await?;
        if original[index] == Value::Other {
            return Err(failure(
                "account original has an unsupported SQLite storage type",
            ));
        }
    }
    let auth_id = read_value(&mut transaction, AUTH_ID_KEY).await?;
    if auth_id == Value::Other {
        return Err(failure(
            "account identity guard has an unsupported SQLite type",
        ));
    }
    let reactive = reactive_object(&read_value(&mut transaction, REACTIVE_USER_KEY).await?)?;
    let mut record = Record {
        version: 2,
        database: fs::canonicalize(database)?,
        stage: Stage::Prepared,
        original,
        auth_id,
        reactive_original: [
            Leaf::snapshot(reactive.get(LEAVES[0])),
            Leaf::snapshot(reactive.get(LEAVES[1])),
        ],
    };
    // Durable encrypted intent precedes any committed SQLite writes.
    save(journal, &record)?;
    for (index, key) in KEYS.iter().enumerate() {
        write_value(&mut transaction, key, &Value::Text(OVERRIDES[index].into())).await?;
    }
    transaction.commit().await.map_err(sql_error)?;
    record.stage = Stage::Active;
    save(journal, &record)
}

/// Common entry for disable, exit and startup recovery. Safe to retry after interruption.
/// Caller must hold its lifecycle lock and enforce Cursor closed.
pub async fn restore(database: &Path, journal: &Path) -> Result<RestoreOutcome> {
    check_paths(database, journal)?;
    let Some(mut record) = load(journal)? else {
        return Ok(RestoreOutcome::default());
    };
    if fs::canonicalize(database)? != record.database {
        return Err(failure("account journal belongs to another database"));
    }
    let mut connection = connect(database).await?;
    let mut transaction = connection
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(sql_error)?;
    record.stage = Stage::Restoring;
    save(journal, &record)?;
    let mut outcome = RestoreOutcome::default();
    if read_value(&mut transaction, AUTH_ID_KEY).await? != record.auth_id {
        outcome.account_changed = true;
        outcome
            .preserved_fields
            .extend(KEYS.iter().map(|key| (*key).to_owned()));
        outcome.preserved_fields.extend(
            LEAVES
                .iter()
                .map(|leaf| format!("{REACTIVE_USER_KEY}.{leaf}")),
        );
    } else {
        for (index, key) in KEYS.iter().enumerate() {
            let current = read_value(&mut transaction, key).await?;
            if current == Value::Text(OVERRIDES[index].into()) {
                write_value(&mut transaction, key, &record.original[index]).await?;
            } else if current != record.original[index] {
                outcome.preserved_fields.push((*key).to_owned());
            }
        }
        restore_reactive(&mut transaction, &record, &mut outcome).await?;
    }
    transaction.commit().await.map_err(sql_error)?;
    // A crash before deletion is harmless: originals compare equal on the next restore.
    crate::config::validate_path(journal, "account journal", false)?;
    fs::remove_file(journal)?;
    Ok(outcome)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn prepared_after_commit_and_restore_after_commit_are_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("state.vscdb");
        let journal = temp.path().join("account.dpapi");
        let mut connection = SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(&database)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        sqlx::query("CREATE TABLE ItemTable(key TEXT UNIQUE, value BLOB)")
            .execute(&mut connection)
            .await
            .unwrap();
        write_value(
            &mut connection,
            REACTIVE_USER_KEY,
            &Value::Text(r#"{"membershipType":"free","keep":"before"}"#.into()),
        )
        .await
        .unwrap();
        inject(&database, &journal, true).await.unwrap();
        let mut record = load(&journal).unwrap().unwrap();
        write_value(
            &mut connection,
            REACTIVE_USER_KEY,
            &Value::Text(
                r#"{"membershipType":"ultra","subscriptionStatus":"active","keep":"after"}"#.into(),
            ),
        )
        .await
        .unwrap();
        // Crash after the DB commit, before the Active journal replacement.
        record.stage = Stage::Prepared;
        save(&journal, &record).unwrap();
        restore(&database, &journal).await.unwrap();
        for key in KEYS {
            assert!(read_value(&mut connection, key).await.unwrap() == Value::Absent);
        }
        let object = reactive_object(
            &read_value(&mut connection, REACTIVE_USER_KEY)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(object.get("membershipType").unwrap(), "free");
        assert!(!object.contains_key("subscriptionStatus"));
        assert_eq!(object.get("keep").unwrap(), "after");
        // Crash after restore's DB commit, before journal deletion.
        record.stage = Stage::Restoring;
        save(&journal, &record).unwrap();
        assert_eq!(
            restore(&database, &journal).await.unwrap(),
            RestoreOutcome::default()
        );
        assert!(!journal.exists());
    }
}
