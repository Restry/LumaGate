//! Official signed updates. The WebView cannot supply URLs, keys, payloads or restart commands.
use super::{network, ManualState};
use serde::{Deserialize, Serialize};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::{watch, Mutex as AsyncMutex};

const AUTO_KEY: &str = "manual_update_auto_v1";
const DISMISSED_KEY: &str = "manual_update_dismissed_v1";
const RESTART_KEY: &str = "manual_update_restart_v1";
const PERIOD: Duration = Duration::from_secs(6 * 60 * 60);
pub const ENDPOINT: &str =
    "https://github.com/Restry/LumaGate/releases/latest/download/latest.json";
const EVENT: &str = "manual-update";

#[derive(Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Idle,
    Checking,
    Available,
    Downloading,
    Ready,
    Waiting,
    Installing,
    Error,
    Unsupported,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    phase: Phase,
    current_version: String,
    version: Option<String>,
    notes: String,
    auto_check: bool,
    prompt: bool,
    received: u64,
    total: Option<u64>,
    active: usize,
    message: String,
    checked_at: Option<String>,
}
struct Inner {
    view: View,
    update: Option<Update>,
    bytes: Option<Vec<u8>>,
    last_check: Option<Instant>,
}
pub struct Updates {
    inner: Mutex<Inner>,
    operation: AsyncMutex<()>,
    cancel: watch::Sender<bool>,
}
impl Updates {
    pub fn new(db: &crate::database::Database) -> Result<Self, crate::error::AppError> {
        Ok(Self {
            inner: Mutex::new(Inner {
                view: View {
                    phase: Phase::Idle,
                    current_version: env!("CARGO_PKG_VERSION").into(),
                    version: None,
                    notes: String::new(),
                    auto_check: db.get_setting(AUTO_KEY)?.as_deref() != Some("false"),
                    prompt: false,
                    received: 0,
                    total: None,
                    active: 0,
                    message: "尚未检查更新".into(),
                    checked_at: None,
                },
                update: None,
                bytes: None,
                last_check: None,
            }),
            operation: AsyncMutex::new(()),
            cancel: watch::channel(false).0,
        })
    }
    fn view(&self) -> View {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .view
            .clone()
    }
    fn change(&self, app: &tauri::AppHandle, change: impl FnOnce(&mut Inner)) {
        let view = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            change(&mut inner);
            inner.view.clone()
        };
        let _ = app.emit(EVENT, view);
    }
    fn fail(&self, app: &tauri::AppHandle, message: String) {
        self.change(app, |s| {
            s.view.phase = Phase::Error;
            s.view.message = message;
            s.view.active = 0;
        });
    }
}
fn platform() -> Result<(&'static str, &'static str, &'static str), String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok(("darwin-aarch64", "macos-arm64", "app.tar.gz")),
        ("macos", "x86_64") => Ok(("darwin-x86_64", "macos-x64", "app.tar.gz")),
        ("windows", "aarch64") => Ok(("windows-aarch64", "windows-arm64", "exe")),
        ("windows", "x86_64") => Ok(("windows-x86_64", "windows-x64", "exe")),
        ("linux", "x86_64") => Ok(("linux-x86_64", "linux-x64", "AppImage")),
        _ => Err("当前平台或架构尚无官方更新包，请查看 GitHub Release".into()),
    }
}
fn validate_release(
    current: &str,
    version: &str,
    raw: &serde_json::Value,
    url: &url::Url,
    signature: &str,
    target: (&str, &str, &str),
) -> Result<(), String> {
    let valid = || -> Option<()> {
        let version_number = semver::Version::parse(version).ok()?;
        if version_number <= semver::Version::parse(current).ok()?
            || !version_number.pre.is_empty()
            || !version_number.build.is_empty()
        {
            return None;
        }
        let expected = format!("https://github.com/Restry/LumaGate/releases/download/lumagate-v{version}/LumaGate-{version}-{}.{}", target.1, target.2);
        let entry = raw.get("platforms")?.get(target.0)?;
        if url.as_str() != expected
            || entry.get("url")?.as_str()? != expected
            || signature.is_empty()
            || entry.get("signature")?.as_str()? != signature
        {
            return None;
        }
        Some(())
    };
    valid().ok_or_else(|| "更新清单不符合官方版本、平台或资产约束；未下载或安装".into())
}
fn supported(app: &tauri::AppHandle) -> Result<(), String> {
    platform()?;
    #[cfg(target_os = "linux")]
    if app.env().appimage.is_none() {
        return Err("deb / 系统包请使用包管理器或官方安装包升级；应用内安装仅支持 AppImage".into());
    }
    let _ = app;
    Ok(())
}
fn writable_installation() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let app = exe
            .ancestors()
            .nth(3)
            .filter(|p| p.extension().is_some_and(|e| e == "app"))
            .ok_or("请在已安装的 LumaGate.app 中更新")?;
        for path in [app, app.parent().ok_or("应用路径无效")?] {
            let path =
                std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
            if unsafe { libc::access(path.as_ptr(), libc::W_OK | libc::X_OK) } != 0 {
                return Err("当前用户不能替换此应用。请将官方应用安装到当前用户可写的 Applications 目录后再试；不会提权或调用 sudo".into());
            }
        }
    }
    Ok(())
}
async fn check(app: &tauri::AppHandle, manual: bool) -> Result<View, String> {
    let updates = app.state::<Updates>();
    let Ok(_operation) = updates.operation.try_lock() else {
        return Ok(updates.view());
    };
    if let Err(message) = supported(app) {
        updates.change(app, |s| {
            s.view.phase = Phase::Unsupported;
            s.view.message = message;
        });
        return Ok(updates.view());
    }
    {
        let mut inner = updates.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !manual
            && (!inner.view.auto_check
                || inner.last_check.is_some_and(|t| t.elapsed() < PERIOD)
                || inner.bytes.is_some())
        {
            return Ok(inner.view.clone());
        }
        inner.last_check = Some(Instant::now());
    }
    updates.change(app, |s| {
        s.view.phase = Phase::Checking;
        s.view.checked_at = Some(chrono::Utc::now().to_rfc3339());
        s.view.message = "正在检查 GitHub 正式版本…".into();
    });
    let result = async {
        let target = platform()?;
        let exit_app = app.clone();
        let updater = app
            .updater_builder()
            .target(target.0)
            .endpoints(vec![ENDPOINT
                .parse()
                .map_err(|e: url::ParseError| e.to_string())?])
            .map_err(|e| e.to_string())?
            .timeout(Duration::from_secs(25))
            .configure_client(|client| {
                client.redirect(reqwest_updater::redirect::Policy::custom(|attempt| {
                    let url = attempt.url();
                    if attempt.previous().len() < 5
                        && url.scheme() == "https"
                        && matches!(
                            url.host_str(),
                            Some(
                                "github.com"
                                    | "release-assets.githubusercontent.com"
                                    | "objects.githubusercontent.com"
                            )
                        )
                    {
                        attempt.follow()
                    } else {
                        attempt.error("更新重定向不属于受信任的 HTTPS 下载域名")
                    }
                }))
            })
            .on_before_exit(move || {
                // Windows calls process::exit inside install, bypassing RunEvent::Exit.
                let _ = exit_app.state::<ManualState>().current_logs().flush();
                exit_app.cleanup_before_exit();
            })
            .build()
            .map_err(|e| e.to_string())?;
        let mut update = updater.check().await.map_err(|e| e.to_string())?;
        if let Some(update) = update.as_mut() {
            validate_release(
                &update.current_version,
                &update.version,
                &update.raw_json,
                &update.download_url,
                &update.signature,
                target,
            )?;
            update.timeout = Some(Duration::from_secs(15 * 60));
        }
        Ok::<_, String>(update)
    }
    .await;
    match result {
        Ok(update) => {
            let dismissed = app
                .state::<ManualState>()
                .db
                .get_setting(DISMISSED_KEY)
                .map_err(|e| e.to_string())?;
            updates.change(app, |s| {
                let same =
                    s.update.as_ref().map(|u| &u.version) == update.as_ref().map(|u| &u.version);
                if !same {
                    s.bytes = None;
                }
                s.view.version = update.as_ref().map(|u| u.version.clone());
                s.view.notes = update
                    .as_ref()
                    .and_then(|u| u.body.clone())
                    .unwrap_or_default();
                s.view.phase = if s.bytes.is_some() {
                    Phase::Ready
                } else if update.is_some() {
                    Phase::Available
                } else {
                    Phase::Idle
                };
                s.view.prompt = update
                    .as_ref()
                    .is_some_and(|u| manual || dismissed.as_deref() != Some(u.version.as_str()));
                s.view.message = if update.is_some() {
                    "发现新版本；确认后才会下载"
                } else {
                    "当前已是最新版本"
                }
                .into();
                s.update = update;
            });
        }
        Err(error) => updates.fail(app, format!("检查失败，网关不受影响：{error}")),
    }
    Ok(updates.view())
}
#[derive(Serialize, Deserialize)]
struct Restart {
    from: String,
    to: String,
    active: Option<network::Settings>,
}
async fn resume(state: &ManualState, version: &str) -> Result<(), String> {
    let Some(value) = state
        .db
        .get_setting(RESTART_KEY)
        .map_err(|e| e.to_string())?
        .filter(|s| !s.is_empty())
    else {
        return Ok(());
    };
    // Consume once. An ordinary later launch must not become auto-start.
    state
        .db
        .set_setting(RESTART_KEY, "")
        .map_err(|e| e.to_string())?;
    let marker: Restart = serde_json::from_str(&value).map_err(|e| e.to_string())?;
    if version != marker.from && version != marker.to {
        return Err("更新恢复版本不匹配，网关保持停止，请手动检查".into());
    }
    if let Some(settings) = marker.active {
        state.start_gateway_with(settings).await?;
    }
    if version != marker.to {
        return Err("上次安装未完成；原网关状态已恢复。请检查系统安装权限后重试，必要时使用官方安装包覆盖；未自动回滚".into());
    }
    Ok(())
}
pub fn restore_before_show(app: &tauri::AppHandle) -> bool {
    let recovered = tauri::async_runtime::block_on(resume(
        &app.state::<ManualState>(),
        env!("CARGO_PKG_VERSION"),
    ));
    if let Err(error) = recovered {
        app.state::<Updates>().fail(app, error);
        true
    } else {
        false
    }
}
pub async fn startup(app: tauri::AppHandle, recovery_failed: bool) {
    tokio::time::sleep(if recovery_failed {
        PERIOD
    } else {
        Duration::from_secs(15)
    })
    .await;
    loop {
        let _ = check(&app, false).await;
        tokio::time::sleep(PERIOD).await;
    }
}
#[tauri::command]
pub fn manual_update_state(state: State<'_, Updates>) -> View {
    state.view()
}
#[tauri::command]
pub async fn manual_update_check(app: tauri::AppHandle) -> Result<View, String> {
    check(&app, true).await
}
#[tauri::command]
pub fn manual_update_auto(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    app.state::<ManualState>()
        .db
        .set_setting(AUTO_KEY, if enabled { "true" } else { "false" })
        .map_err(|e| e.to_string())?;
    app.state::<Updates>()
        .change(&app, |s| s.view.auto_check = enabled);
    Ok(())
}
#[tauri::command]
pub fn manual_update_later(app: tauri::AppHandle) -> Result<(), String> {
    let updates = app.state::<Updates>();
    let mut inner = updates.inner.lock().unwrap_or_else(|e| e.into_inner());
    if inner.view.phase == Phase::Installing {
        return Err("已开始安装，不能取消；请等待应用重新打开".into());
    }
    if let Some(version) = &inner.view.version {
        app.state::<ManualState>()
            .db
            .set_setting(DISMISSED_KEY, version)
            .map_err(|e| e.to_string())?;
    }
    updates.cancel.send_replace(true);
    inner.view.prompt = false;
    let view = inner.view.clone();
    drop(inner);
    let _ = app.emit(EVENT, view);
    Ok(())
}
#[tauri::command]
pub async fn manual_update_install(app: tauri::AppHandle) -> Result<(), String> {
    let updates = app.state::<Updates>();
    let _operation = updates
        .operation
        .try_lock()
        .map_err(|_| "另一个更新操作正在进行")?;
    supported(&app)?;
    let update = updates
        .inner
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .update
        .clone()
        .ok_or("请先检查更新")?;
    updates.cancel.send_replace(false);
    let mut cancel = updates.cancel.subscribe();
    let result = install(&app, &updates, update, &mut cancel).await;
    if let Err(error) = &result {
        updates.fail(&app, error.clone());
    }
    result
}
async fn install(
    app: &tauri::AppHandle,
    updates: &Updates,
    update: Update,
    cancel: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    let has_bytes = updates
        .inner
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .bytes
        .is_some();
    if !has_bytes {
        updates.change(app, |s| {
            s.view.phase = Phase::Downloading;
            s.view.received = 0;
            s.view.total = None;
            s.view.prompt = true;
            s.view.message = "正在下载并验证签名，网关继续运行…".into();
        });
        let mut received = 0u64;
        let mut last_emit = Instant::now();
        let download = update.download(
            |count, total| {
                received += count as u64;
                if last_emit.elapsed() >= Duration::from_millis(150) {
                    updates.change(app, |s| {
                        s.view.received = received;
                        s.view.total = total;
                    });
                    last_emit = Instant::now();
                }
            },
            || {},
        );
        let bytes = tokio::select! {
            result = download => result.map_err(|e| format!("下载或验签失败，旧应用未被替换，网关继续运行：{e}"))?,
            _ = cancel.changed() => { updates.change(app, |s| { s.view.phase = Phase::Available; s.view.message = "已取消下载，网关继续运行".into(); }); return Ok(()); }
        };
        updates.change(app, |s| {
            s.view.received = bytes.len() as u64;
            s.bytes = Some(bytes);
            s.view.phase = Phase::Ready;
            s.view.message = "签名已验证，准备等待现有请求结束…".into();
        });
    }
    writable_installation()?;
    let state = app.state::<ManualState>();
    let barrier = {
        let _mutation = state.mutation.lock().await;
        state.drain.close()?
    };
    let mut active = state.drain.subscribe();
    loop {
        let count = *active.borrow_and_update();
        if *cancel.borrow() {
            break;
        }
        updates.change(app, |s| {
            s.view.phase = Phase::Waiting;
            s.view.active = count;
            s.view.message = format!(
                "等待 {count} 个请求结束；新请求暂时返回 503，可稍后重试。可取消并恢复接收请求。"
            );
        });
        if count == 0 {
            break;
        }
        tokio::select! { _ = active.changed() => {}, _ = cancel.changed() => {} }
    }
    // Serialize cancel versus the irreversible install transition.
    let cancelled = {
        let mut inner = updates.inner.lock().unwrap_or_else(|e| e.into_inner());
        if *cancel.borrow() {
            true
        } else {
            inner.view.phase = Phase::Installing;
            false
        }
    };
    if cancelled {
        drop(barrier);
        updates.change(app, |s| {
            s.view.phase = Phase::Ready;
            s.view.active = 0;
            s.view.message = "已暂停更新，恢复接收请求；安装包已验签，可稍后重试".into();
        });
        return Ok(());
    }
    let _mutation = state.mutation.lock().await;
    let running = state.server.lock().await.take();
    let marker = Restart {
        from: update.current_version.clone(),
        to: update.version.clone(),
        active: running.as_ref().map(|r| r.settings.clone()),
    };
    let outcome = async {
        // Hyper must flush its last buffered frames too, not merely drop the response body.
        if let Some(running) = &running {
            running
                .server
                .stop_drained()
                .await
                .map_err(|e| e.to_string())?;
        }
        state.current_logs().flush()?;
        state
            .db
            .set_setting(
                RESTART_KEY,
                &serde_json::to_string(&marker).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        updates.change(app, |s| {
            s.view.phase = Phase::Installing;
            s.view.active = 0;
            s.view.message = "正在安装并重启；服务将短暂不可用，请勿关闭应用".into();
        });
        let bytes = updates
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bytes
            .take()
            .ok_or("已验证的安装包不存在，请重新下载")?;
        tauri::async_runtime::spawn_blocking(move || update.install(bytes))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()))
    }
    .await;
    if let Err(error) = outcome {
        let cleared = state
            .db
            .set_setting(RESTART_KEY, "")
            .map_err(|e| e.to_string());
        let recovery = restore_listener(&state, running.map(|r| r.settings)).await;
        drop(barrier);
        return Err(format!("安装未完成，{recovery}。不能保证自动回滚；若应用文件损坏请使用官方安装包覆盖，勿删除 ~/.lumagate。{error}{}", cleared.err().map(|e| format!("；恢复标记清除失败：{e}")).unwrap_or_default()));
    }
    // Keep admission closed until Tauri completes the normal restart lifecycle.
    app.restart();
}

async fn restore_listener(state: &ManualState, settings: Option<network::Settings>) -> String {
    if let Some(settings) = settings {
        match state.start_server(settings).await {
            Ok(server) => {
                *state.server.lock().await = Some(server);
                "原监听已恢复".into()
            }
            Err(error) => format!("监听恢复失败，请手动启动：{error}"),
        }
    } else {
        "原网关保持停止".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_manifest_rejects_wrong_platform_origin_and_non_increasing_version() {
        let target = ("windows-aarch64", "windows-arm64", "exe");
        let url: url::Url = "https://github.com/Restry/LumaGate/releases/download/lumagate-v3.24.8/LumaGate-3.24.8-windows-arm64.exe".parse().unwrap();
        let raw = serde_json::json!({"platforms":{"windows-aarch64":{"url":url.as_str(),"signature":"signed"}}});
        assert!(validate_release("3.24.7", "3.24.8", &raw, &url, "signed", target).is_ok());
        assert!(validate_release("3.24.8", "3.24.8", &raw, &url, "signed", target).is_err());
        assert!(validate_release("3.24.9", "3.24.8", &raw, &url, "signed", target).is_err());
        assert!(validate_release(
            "3.24.7",
            "3.24.8",
            &raw,
            &url,
            "signed",
            ("windows-x86_64", "windows-x64", "exe")
        )
        .is_err());
        let foreign = "https://github.com/other/LumaGate/releases/download/v3.24.8/update.exe"
            .parse()
            .unwrap();
        assert!(validate_release("3.24.7", "3.24.8", &raw, &foreign, "signed", target).is_err());
        assert!(validate_release("3.24.7", "3.24.8", &raw, &url, "", target).is_err());
    }

    #[tokio::test]
    async fn restart_consumes_intent_once_and_preserves_stopped_or_active_listener() {
        use std::sync::Arc;
        let directory = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::database::Database::memory().unwrap());
        super::super::catalog::Document::default()
            .save(&db)
            .unwrap();
        let state = ManualState {
            db: db.clone(),
            copilot: Arc::new(super::super::copilot::Manager::new(
                directory.path().join("copilot"),
            )),
            logs: Mutex::new(Arc::new(super::super::logs::RequestLogs::default())),
            drain: Arc::new(super::super::drain::Gate::default()),
            mutation: AsyncMutex::new(()),
            server: AsyncMutex::new(None),
            plans: AsyncMutex::new(std::collections::HashMap::new()),
        };
        let marker = Restart {
            from: "3.24.7".into(),
            to: "3.24.8".into(),
            active: None,
        };
        db.set_setting(RESTART_KEY, &serde_json::to_string(&marker).unwrap())
            .unwrap();
        resume(&state, "3.24.8").await.unwrap();
        assert!(state.server.lock().await.is_none());
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        let saved = network::Settings::load(&db).unwrap();
        let mut active = saved.clone();
        active.listen_port = port;
        let marker = Restart {
            active: Some(active.clone()),
            ..marker
        };
        db.set_setting(RESTART_KEY, &serde_json::to_string(&marker).unwrap())
            .unwrap();
        resume(&state, "3.24.8").await.unwrap();
        let response = reqwest::get(format!("http://127.0.0.1:{port}/v1/models"))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.bytes().await.unwrap();
        let barrier = state.drain.close().unwrap();
        let running = state.server.lock().await.take().unwrap();
        running.server.stop_drained().await.unwrap();
        assert!(reqwest::get(format!("http://127.0.0.1:{port}/v1/models"))
            .await
            .is_err());
        // The installation error path must restore the exact old listener, not saved settings.
        restore_listener(&state, Some(running.settings)).await;
        drop(barrier);
        assert_eq!(
            reqwest::get(format!("http://127.0.0.1:{port}/v1/models"))
                .await
                .unwrap()
                .status(),
            200
        );
        assert!(network::Settings::load(&db).unwrap() == saved);
        state.set_gateway(false).await.unwrap();
        resume(&state, "3.24.8").await.unwrap();
        assert!(state.server.lock().await.is_none());
        db.set_setting(RESTART_KEY, &serde_json::to_string(&marker).unwrap())
            .unwrap();
        assert!(resume(&state, "3.24.7").await.is_err());
        assert!(state.server.lock().await.is_some());
        state.set_gateway(false).await.unwrap();
    }
}
