//! Public release metadata only. Never send model credentials or download executables.
use super::ControlService;
use crate::Result;
use axum::{extract::State, Json};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const API: &str =
    "https://api.github.com/repos/dude1wudv/cursor-sub2api-byok/releases?per_page=100";
const REPO: &str = "https://github.com/dude1wudv/cursor-sub2api-byok";
const SETTING: &str = "release_update_preferences";
const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    automatic: bool,
    include_prereleases: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            automatic: true,
            include_prereleases: true,
        }
    }
}
#[derive(Default)]
pub(super) struct UpdateState {
    last_attempt: Option<Instant>,
    checked_at_ms: Option<i64>,
    releases: Vec<Release>,
    error: Option<String>,
}
#[derive(Clone, Deserialize)]
struct Asset {
    name: String,
    state: String,
    size: u64,
}
#[derive(Clone, Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Serialize)]
struct AvailableRelease {
    version: String,
    prerelease: bool,
    release_url: String,
    download_url: String,
}
#[derive(Serialize)]
pub struct Status {
    current_version: String,
    preferences: Preferences,
    checked_at_ms: Option<i64>,
    error: Option<String>,
    latest: Option<AvailableRelease>,
    update_available: bool,
}
fn select_release(releases: &[Release], include_prereleases: bool) -> Option<(Version, &Release)> {
    releases
        .iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let version = Version::parse(
                release
                    .tag_name
                    .strip_prefix('v')
                    .unwrap_or(&release.tag_name),
            )
            .ok()?;
            if !include_prereleases && (release.prerelease || !version.pre.is_empty()) {
                return None;
            }
            let complete = [
                "Cursor-Sub2API-BYOK.exe",
                "Cursor-Sub2API-BYOK-windows-x64.zip",
                "LICENSE",
                "THIRD-PARTY-NOTICES.txt",
                "SHA256SUMS.txt",
            ]
            .iter()
            .all(|name| {
                release
                    .assets
                    .iter()
                    .any(|asset| asset.name == *name && asset.state == "uploaded" && asset.size > 0)
            });
            complete.then_some((version, release))
        })
        .max_by(|a, b| a.0.cmp_precedence(&b.0))
}
impl UpdateState {
    fn status(&self, current: &str, preferences: Preferences) -> Status {
        let latest = select_release(&self.releases, preferences.include_prereleases);
        let update_available = latest.as_ref().is_some_and(|(latest, _)| {
            Version::parse(current).is_ok_and(|current| latest.cmp_precedence(&current).is_gt())
        });
        Status {
            current_version: current.into(),
            preferences,
            checked_at_ms: self.checked_at_ms,
            error: self.error.clone(),
            update_available,
            latest: latest.map(|(version, release)| AvailableRelease {
                prerelease: release.prerelease || !version.pre.is_empty(),
                version: version.to_string(),
                release_url: format!("{REPO}/releases/tag/{}", release.tag_name),
                download_url: format!(
                    "{REPO}/releases/download/{}/Cursor-Sub2API-BYOK-windows-x64.zip",
                    release.tag_name
                ),
            }),
        }
    }
}
async fn read_preferences(service: &ControlService) -> Result<Preferences> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value_json FROM service_settings WHERE setting_key = ?")
            .bind(SETTING)
            .fetch_optional(service.store.pool())
            .await?;
    value
        .map(|value| serde_json::from_str(&value).map_err(Into::into))
        .unwrap_or_else(|| Ok(Preferences::default()))
}
pub async fn get(State(service): State<ControlService>) -> Result<Json<Status>> {
    let preferences = read_preferences(&service).await?;
    Ok(Json(
        service
            .updates
            .lock()
            .await
            .status(&service.app_version, preferences),
    ))
}
pub async fn preferences(
    State(service): State<ControlService>,
    Json(preferences): Json<Preferences>,
) -> Result<Json<Status>> {
    sqlx::query("INSERT INTO service_settings(setting_key,value_json,updated_at_ms) VALUES (?,?,?) ON CONFLICT(setting_key) DO UPDATE SET value_json=excluded.value_json,updated_at_ms=excluded.updated_at_ms")
        .bind(SETTING).bind(serde_json::to_string(&preferences)?).bind(chrono::Utc::now().timestamp_millis())
        .execute(service.store.pool()).await?;
    Ok(Json(
        service
            .updates
            .lock()
            .await
            .status(&service.app_version, preferences),
    ))
}
pub async fn check(State(service): State<ControlService>) -> Result<Json<Status>> {
    let preferences = read_preferences(&service).await?;
    let mut state = service.updates.lock().await;
    // Single flight and a one-minute minimum, including failed/rate-limited requests.
    if state
        .last_attempt
        .is_none_or(|at| at.elapsed() >= Duration::from_secs(60))
    {
        state.last_attempt = Some(Instant::now());
        state.checked_at_ms = Some(chrono::Utc::now().timestamp_millis());
        match fetch(&service).await {
            Ok(releases) => {
                state.releases = releases;
                state.error = None;
            }
            Err(error) => {
                state.error = Some(error);
            }
        }
    }
    Ok(Json(state.status(&service.app_version, preferences)))
}
async fn fetch(service: &ControlService) -> std::result::Result<Vec<Release>, String> {
    let client = crate::network::client_builder(&service.store)
        .await
        .map_err(|_| "无法读取出站代理配置")?
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法创建更新检查连接")?;
    let response = client
        .get(API)
        .header("User-Agent", "Cursor-Sub2API-BYOK-update-check")
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2026-03-10")
        .send()
        .await
        .map_err(|_| "GitHub 更新检查失败，请检查出站代理或网络；没有切换为直连")?;
    read_response(response).await
}
async fn read_response(
    mut response: reqwest::Response,
) -> std::result::Result<Vec<Release>, String> {
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            403 | 429 => "GitHub 请求受限，请稍后重试（至少间隔一分钟）".into(),
            status => format!("GitHub 更新检查失败（HTTP {status}）"),
        });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "读取 GitHub 更新信息失败")?
    {
        if body.len() + chunk.len() > MAX_BYTES {
            return Err("GitHub 更新信息超过大小限制".into());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| "GitHub 更新信息格式无效".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(tag: &str) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
            assets: [
                "Cursor-Sub2API-BYOK.exe",
                "Cursor-Sub2API-BYOK-windows-x64.zip",
                "LICENSE",
                "THIRD-PARTY-NOTICES.txt",
                "SHA256SUMS.txt",
            ]
            .into_iter()
            .map(|name| Asset {
                name: name.into(),
                state: "uploaded".into(),
                size: 1,
            })
            .collect(),
        }
    }
    #[test]
    fn selects_semver_with_prerelease_channel_and_complete_assets() {
        let mut draft = release("v9.0.0");
        draft.draft = true;
        let mut incomplete = release("v8.0.0");
        incomplete.assets.pop();
        let releases = vec![
            draft,
            incomplete,
            release("v0.3.0-rc.2"),
            release("v0.3.0-rc.10"),
            release("v0.2.9"),
            release("../bad"),
        ];
        assert_eq!(
            select_release(&releases, true).unwrap().0.to_string(),
            "0.3.0-rc.10"
        );
        assert_eq!(
            select_release(&releases, false).unwrap().0.to_string(),
            "0.2.9"
        );
        let mut state = UpdateState {
            releases,
            ..Default::default()
        };
        assert!(
            state
                .status("0.3.0-rc.2", Preferences::default())
                .update_available
        );
        assert!(
            !state
                .status("0.3.0", Preferences::default())
                .update_available
        );
        state.releases = vec![release("v0.3.0+build.2")];
        assert!(
            !state
                .status("0.3.0+build.1", Preferences::default())
                .update_available
        );
        state.releases.push(release("v0.3.0"));
        assert!(
            state
                .status("0.3.0-rc.10", Preferences::default())
                .update_available
        );
    }
    #[tokio::test]
    async fn rejects_rate_limit_invalid_json_and_large_http_response() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (status, body, expected) in [
            (429, "{}".into(), "受限"),
            (200, "bad json".into(), "格式"),
            (200, "x".repeat(MAX_BYTES + 1), "大小"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let task = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 4096];
                let _ = socket.read(&mut buffer).await;
                let header = format!(
                    "HTTP/1.1 {status} Response\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(body.as_bytes()).await;
            });
            let response = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(format!("http://{address}"))
                .send()
                .await
                .unwrap();
            assert!(read_response(response)
                .await
                .err()
                .unwrap()
                .contains(expected));
            task.await.unwrap();
        }
    }
}
