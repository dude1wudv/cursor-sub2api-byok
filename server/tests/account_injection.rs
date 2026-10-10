//! Temporary databases and synthetic metadata only; never discover a profile.
#![cfg(windows)]
use cursor_server::local_app::account::{self, AUTH_ID_KEY, KEYS, REACTIVE_USER_KEY};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
    Connection, Row, SqliteConnection,
};
use std::path::{Path, PathBuf};

async fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, SqliteConnection) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.vscdb");
    let journal = temp.path().join("account.dpapi");
    let mut connection = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(&db)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TABLE ItemTable(key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)")
        .execute(&mut connection)
        .await
        .unwrap();
    (temp, db, journal, connection)
}
async fn put(connection: &mut SqliteConnection, key: &str, value: &str) {
    sqlx::query("INSERT OR REPLACE INTO ItemTable VALUES (?, ?)")
        .bind(key)
        .bind(value)
        .execute(connection)
        .await
        .unwrap();
}
async fn text(connection: &mut SqliteConnection, key: &str) -> Option<String> {
    sqlx::query_scalar("SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = ?")
        .bind(key)
        .fetch_optional(connection)
        .await
        .unwrap()
}
async fn reactive(connection: &mut SqliteConnection) -> serde_json::Value {
    serde_json::from_str(&text(connection, REACTIVE_USER_KEY).await.unwrap()).unwrap()
}

#[tokio::test]
async fn only_two_fields_change_dpapi_restores_blob_absent_and_credentials_are_untouched() {
    let (_temp, db, journal, mut connection) = fixture().await;
    sqlx::query("INSERT INTO ItemTable VALUES (?, ?)")
        .bind(KEYS[0])
        .bind(vec![0u8, 255, 4])
        .execute(&mut connection)
        .await
        .unwrap();
    let untouched = [
        ("cursorAuth/accessToken", "synthetic-access-never-read"),
        ("cursorAuth/refreshToken", "synthetic-refresh-never-read"),
        ("cursorAuth/cachedEmail", "synthetic@example.invalid"),
        ("cursorAuth/cachedSignUpType", "Google"),
        (AUTH_ID_KEY, "synthetic-identity-guard"),
    ];
    for (key, value) in untouched {
        put(&mut connection, key, value).await;
    }
    let before = r#"{"membershipType":"free","subscriptionStatus":null,"nested":{"keep":true}}"#;
    put(&mut connection, REACTIVE_USER_KEY, before).await;
    account::inject(&db, &journal, true).await.unwrap();
    assert_eq!(
        text(&mut connection, KEYS[0]).await.as_deref(),
        Some("ultra")
    );
    assert_eq!(
        text(&mut connection, KEYS[1]).await.as_deref(),
        Some("active")
    );
    assert_eq!(
        text(&mut connection, REACTIVE_USER_KEY).await.as_deref(),
        Some(before)
    );
    let encrypted = std::fs::read(&journal).unwrap();
    let sensitive = b"synthetic-identity-guard";
    assert!(!encrypted
        .windows(sensitive.len())
        .any(|bytes| bytes == sensitive));
    put(&mut connection, "unrelated-session", "keep-new-data").await;
    assert_eq!(
        account::restore(&db, &journal).await.unwrap(),
        Default::default()
    );
    let row = sqlx::query("SELECT typeof(value) AS kind, value FROM ItemTable WHERE key = ?")
        .bind(KEYS[0])
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("kind"), "blob");
    assert_eq!(row.get::<Vec<u8>, _>("value"), vec![0, 255, 4]);
    assert_eq!(text(&mut connection, KEYS[1]).await, None);
    for (key, value) in untouched {
        assert_eq!(text(&mut connection, key).await.as_deref(), Some(value));
    }
    assert_eq!(
        text(&mut connection, "unrelated-session").await.as_deref(),
        Some("keep-new-data")
    );
    assert!(!account::pending(&journal).unwrap());
    assert_eq!(
        account::restore(&db, &journal).await.unwrap(),
        Default::default()
    );
}

#[tokio::test]
async fn mirrored_json_restores_matching_leaves_preserving_blob_type_and_unrelated_changes() {
    let (_temp, db, journal, mut connection) = fixture().await;
    put(&mut connection, KEYS[0], "pro").await;
    put(
        &mut connection,
        REACTIVE_USER_KEY,
        r#"{"membershipType":"pro","subscriptionStatus":null,"other":"old"}"#,
    )
    .await;
    account::inject(&db, &journal, true).await.unwrap();
    let current = br#"{"membershipType":"ultra","subscriptionStatus":"active","other":"changed","nested":{"new":42}}"#;
    sqlx::query("UPDATE ItemTable SET value = ? WHERE key = ?")
        .bind(current.as_slice())
        .bind(REACTIVE_USER_KEY)
        .execute(&mut connection)
        .await
        .unwrap();
    assert_eq!(
        account::restore(&db, &journal).await.unwrap(),
        Default::default()
    );
    assert_eq!(
        reactive(&mut connection).await,
        serde_json::json!({"membershipType":"pro","subscriptionStatus":null,"other":"changed","nested":{"new":42}})
    );
    let kind: String = sqlx::query_scalar("SELECT typeof(value) FROM ItemTable WHERE key = ?")
        .bind(REACTIVE_USER_KEY)
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(kind, "blob");
}

#[tokio::test]
async fn absent_derived_leaves_are_removed_without_deleting_new_json_data() {
    let (_temp, db, journal, mut connection) = fixture().await;
    account::inject(&db, &journal, true).await.unwrap();
    put(
        &mut connection,
        REACTIVE_USER_KEY,
        r#"{"membershipType":"ultra","subscriptionStatus":"active","newSession":"keep"}"#,
    )
    .await;
    account::restore(&db, &journal).await.unwrap();
    assert_eq!(
        reactive(&mut connection).await,
        serde_json::json!({"newSession":"keep"})
    );
}

#[tokio::test]
async fn third_party_stripe_and_derived_changes_are_preserved_independently() {
    let (_temp, db, journal, mut connection) = fixture().await;
    put(&mut connection, KEYS[0], "free").await;
    put(&mut connection, KEYS[1], "inactive").await;
    put(
        &mut connection,
        REACTIVE_USER_KEY,
        r#"{"membershipType":"free","subscriptionStatus":"inactive"}"#,
    )
    .await;
    account::inject(&db, &journal, true).await.unwrap();
    put(&mut connection, KEYS[0], "pro").await;
    sqlx::query("DELETE FROM ItemTable WHERE key = ?")
        .bind(KEYS[1])
        .execute(&mut connection)
        .await
        .unwrap();
    put(
        &mut connection,
        REACTIVE_USER_KEY,
        r#"{"membershipType":"pro","subscriptionStatus":"active","new":1}"#,
    )
    .await;
    let outcome = account::restore(&db, &journal).await.unwrap();
    assert_eq!(
        outcome.preserved_fields,
        vec![
            KEYS[0].to_owned(),
            KEYS[1].to_owned(),
            format!("{REACTIVE_USER_KEY}.membershipType")
        ]
    );
    assert!(!outcome.account_changed);
    assert_eq!(text(&mut connection, KEYS[0]).await.as_deref(), Some("pro"));
    assert_eq!(text(&mut connection, KEYS[1]).await, None);
    assert_eq!(
        reactive(&mut connection).await,
        serde_json::json!({"membershipType":"pro","subscriptionStatus":"inactive","new":1})
    );
}

#[tokio::test]
async fn changed_auth_id_preserves_new_account_even_when_values_match_override() {
    let (_temp, db, journal, mut connection) = fixture().await;
    put(&mut connection, AUTH_ID_KEY, "synthetic-user-a").await;
    put(&mut connection, KEYS[0], "free").await;
    put(
        &mut connection,
        REACTIVE_USER_KEY,
        r#"{"membershipType":"free"}"#,
    )
    .await;
    account::inject(&db, &journal, true).await.unwrap();
    put(&mut connection, AUTH_ID_KEY, "synthetic-user-b").await;
    put(
        &mut connection,
        REACTIVE_USER_KEY,
        r#"{"membershipType":"ultra","subscriptionStatus":"active","newAccount":true}"#,
    )
    .await;
    let outcome = account::restore(&db, &journal).await.unwrap();
    assert!(outcome.account_changed);
    assert_eq!(outcome.preserved_fields.len(), 4);
    assert_eq!(
        text(&mut connection, AUTH_ID_KEY).await.as_deref(),
        Some("synthetic-user-b")
    );
    assert_eq!(
        text(&mut connection, KEYS[0]).await.as_deref(),
        Some("ultra")
    );
    assert_eq!(reactive(&mut connection).await["membershipType"], "ultra");
    assert!(!journal.exists());
}

#[tokio::test]
async fn prepared_rollback_and_restore_failure_are_atomic_with_redacted_errors() {
    let (_temp, db, journal, mut connection) = fixture().await;
    sqlx::query("CREATE TRIGGER fail_injection BEFORE INSERT ON ItemTable WHEN NEW.key = 'cursorAuth/stripeSubscriptionStatus' BEGIN SELECT RAISE(ABORT, 'synthetic-sensitive-trigger-error'); END")
        .execute(&mut connection).await.unwrap();
    let error = account::inject(&db, &journal, true).await.unwrap_err();
    assert!(!error
        .to_string()
        .contains("synthetic-sensitive-trigger-error"));
    assert!(account::pending(&journal).unwrap());
    assert_eq!(text(&mut connection, KEYS[0]).await, None);
    account::restore(&db, &journal).await.unwrap();
    sqlx::query("DROP TRIGGER fail_injection")
        .execute(&mut connection)
        .await
        .unwrap();
    account::inject(&db, &journal, true).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_restore BEFORE DELETE ON ItemTable WHEN OLD.key = 'cursorAuth/stripeSubscriptionStatus' BEGIN SELECT RAISE(ABORT, 'synthetic-sensitive-restore-error'); END")
        .execute(&mut connection).await.unwrap();
    let error = account::restore(&db, &journal).await.unwrap_err();
    assert!(!error
        .to_string()
        .contains("synthetic-sensitive-restore-error"));
    assert!(journal.exists());
    assert_eq!(
        text(&mut connection, KEYS[0]).await.as_deref(),
        Some("ultra")
    );
    sqlx::query("DROP TRIGGER fail_restore")
        .execute(&mut connection)
        .await
        .unwrap();
    account::restore(&db, &journal).await.unwrap();
    assert!(!journal.exists());
}

#[tokio::test]
async fn invalid_reactive_state_blocks_inject_but_later_damage_is_preserved() {
    let (_temp, db, journal, mut connection) = fixture().await;
    put(&mut connection, REACTIVE_USER_KEY, "not json").await;
    assert!(account::inject(&db, &journal, true).await.is_err());
    assert!(!journal.exists());
    assert_eq!(text(&mut connection, KEYS[0]).await, None);
    put(&mut connection, REACTIVE_USER_KEY, "{}").await;
    account::inject(&db, &journal, true).await.unwrap();
    put(
        &mut connection,
        REACTIVE_USER_KEY,
        "third party invalid json",
    )
    .await;
    let outcome = account::restore(&db, &journal).await.unwrap();
    assert_eq!(outcome.preserved_fields, vec![REACTIVE_USER_KEY]);
    assert_eq!(
        text(&mut connection, REACTIVE_USER_KEY).await.as_deref(),
        Some("third party invalid json")
    );
    assert_eq!(text(&mut connection, KEYS[0]).await, None);
}

#[tokio::test]
async fn reactive_write_failure_rolls_back_stripe_restoration_and_retains_journal() {
    let (_temp, db, journal, mut connection) = fixture().await;
    put(
        &mut connection,
        REACTIVE_USER_KEY,
        r#"{"membershipType":"free"}"#,
    )
    .await;
    account::inject(&db, &journal, true).await.unwrap();
    put(
        &mut connection,
        REACTIVE_USER_KEY,
        r#"{"membershipType":"ultra","subscriptionStatus":"active","keep":1}"#,
    )
    .await;
    sqlx::query("CREATE TRIGGER fail_mirror BEFORE UPDATE ON ItemTable WHEN NEW.key = 'src.vs.platform.reactivestorage.browser.reactiveStorageServiceImpl.persistentStorage.applicationUser' BEGIN SELECT RAISE(ABORT, 'synthetic-private-mirror'); END")
        .execute(&mut connection).await.unwrap();
    let error = account::restore(&db, &journal).await.unwrap_err();
    assert!(!error.to_string().contains("synthetic-private-mirror"));
    assert!(journal.exists());
    assert_eq!(
        text(&mut connection, KEYS[0]).await.as_deref(),
        Some("ultra")
    );
    assert_eq!(
        text(&mut connection, KEYS[1]).await.as_deref(),
        Some("active")
    );
    assert_eq!(reactive(&mut connection).await["membershipType"], "ultra");
    sqlx::query("DROP TRIGGER fail_mirror")
        .execute(&mut connection)
        .await
        .unwrap();
    account::restore(&db, &journal).await.unwrap();
    assert_eq!(
        reactive(&mut connection).await,
        serde_json::json!({"membershipType":"free","keep":1})
    );
    assert!(!journal.exists());
}

#[tokio::test]
async fn consent_pending_wrong_target_corrupt_journal_and_unsafe_paths_fail_without_writes() {
    let (temp, db, journal, mut connection) = fixture().await;
    assert!(account::inject(&db, &journal, false).await.is_err());
    assert!(!journal.exists());
    account::inject(&db, &journal, true).await.unwrap();
    let original = std::fs::read(&journal).unwrap();
    assert!(account::inject(&db, &journal, true).await.is_err());
    let (_other, other_db, _, _) = fixture().await;
    assert!(account::restore(&other_db, &journal).await.is_err());
    assert_eq!(std::fs::read(&journal).unwrap(), original);
    account::restore(&db, &journal).await.unwrap();
    std::fs::write(&journal, b"corrupt").unwrap();
    assert!(account::inject(&db, &journal, true).await.is_err());
    assert!(account::restore(&db, &journal).await.is_err());
    assert_eq!(std::fs::read(&journal).unwrap(), b"corrupt");
    assert_eq!(text(&mut connection, KEYS[0]).await, None);
    let missing = temp.path().join("missing.vscdb");
    assert!(
        account::inject(&missing, &temp.path().join("new.dpapi"), true)
            .await
            .is_err()
    );
    assert!(!missing.exists());
    assert!(account::inject(Path::new("relative.vscdb"), &journal, true)
        .await
        .is_err());
    assert!(account::inject(&db, &db, true).await.is_err());
    assert!(
        account::inject(&db, &temp.path().join("state.vscdb-wal"), true)
            .await
            .is_err()
    );
}

#[test]
fn upstream_placeholder_detection_remains_available_for_official_forwarding_guard() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let token = format!(
        "e30.{}.cursor-local-user",
        URL_SAFE_NO_PAD.encode(br#"{"sub":"cursor-local-user","iss":"cursor-client"}"#)
    );
    assert!(account::is_placeholder_token(&token));
    assert!(!account::is_placeholder_token("test-valid-token-prefix"));
    assert!(!account::is_placeholder_token("opaque-token"));
}
