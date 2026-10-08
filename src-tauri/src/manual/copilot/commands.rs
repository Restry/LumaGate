use super::*;
use crate::manual::{
    catalog::{Document, Source},
    ManualState,
};
use tauri::State as TauriState;
#[tauri::command]
pub async fn manual_copilot_start(state: TauriState<'_, ManualState>) -> Result<Challenge, String> {
    state
        .copilot
        .start(Document::load(&state.db)?.revision)
        .await
}
#[tauri::command]
pub async fn manual_copilot_cancel(
    state: TauriState<'_, ManualState>,
    flow_id: String,
) -> Result<(), String> {
    state.copilot.cancel(&flow_id).await;
    Ok(())
}
#[tauri::command]
pub async fn manual_copilot_poll(
    state: TauriState<'_, ManualState>,
    flow_id: String,
) -> Result<Value, String> {
    match state.copilot.poll(&flow_id).await? {
        Poll::Pending(interval) => Ok(json!({"state":"pending","interval":interval})),
        Poll::Ready(authorized) => {
            let _guard = state.mutation.lock().await;
            link(&state.copilot, &state.db, &flow_id, authorized).await
        }
    }
}
// The caller owns ManualState's mutation lock. Tests cross this same commit seam.
pub(super) async fn link(
    manager: &Manager,
    db: &crate::database::Database,
    flow_id: &str,
    authorized: Authorized,
) -> Result<Value, String> {
    let mut doc = Document::load(db)?;
    if doc.revision != authorized.revision {
        manager.cancel(flow_id).await;
        return Err("授权期间配置已变化，请重新登录；未覆盖现有配置".into());
    }
    let mut flow = manager.device.lock().await;
    if !flow
        .as_ref()
        .is_some_and(|d| d.id == flow_id && Instant::now() < d.expires)
    {
        return Err("授权已取消或过期".into());
    }
    if doc
        .providers
        .iter()
        .filter_map(|p| p.copilot.as_ref())
        .any(|b| b.account_id != authorized.credential.account_id)
    {
        *flow = None;
        return Err("已有另一个 GitHub 账号，请先断开本机登录后再切换账号".into());
    }
    let old = doc.providers.iter().find(|p| p.id == PROVIDER_ID).cloned();
    if old.as_ref().is_some_and(|p| p.copilot.is_none()) {
        *flow = None;
        return Err("现有来源使用了 Copilot 保留标识，未覆盖现有来源".into());
    }
    let binding = Binding {
        account_id: authorized.credential.account_id.clone(),
        grant_id: authorized.credential.grant_id.clone(),
    };
    let source = Source {
        id: PROVIDER_ID.into(),
        name: old
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or("GitHub Copilot".into()),
        base_url: API_ROOT.into(),
        key_env: String::new(),
        key_ref: None,
        copilot: Some(binding),
        protocol: Protocol::OpenaiChat,
        enabled: old.as_ref().map(|p| p.enabled).unwrap_or(true),
        cost_estimation_enabled: old.as_ref().is_none_or(|p| p.cost_estimation_enabled),
        models: old.map(|p| p.models).unwrap_or_default(),
    };
    doc.providers.retain(|p| p.id != PROVIDER_ID);
    doc.providers.push(source);
    doc.validate()?;
    let mut auth = manager.state.lock().await;
    let previous = auth.credential.clone();
    manager.persist(&authorized.credential)?;
    if let Err(error) = crate::manual::mirror_sources(db, &doc).and_then(|_| doc.save(db)) {
        let rollback = if let Some(old) = previous {
            manager.persist(&old)
        } else {
            fs::remove_file(&manager.path).map_err(|_| "无法回滚授权文件".into())
        };
        *flow = None;
        if rollback.is_err() {
            auth.load_error = true;
            auth.credential = None;
            auth.access = None;
            return Err(
                "配置未能保存，授权文件回滚也失败；已停止使用凭据，请清理本机登录后重试".into(),
            );
        }
        return Err(error);
    }
    let login = authorized.credential.login.clone();
    auth.credential = Some(authorized.credential);
    auth.access = Some(authorized.access);
    auth.load_error = false;
    auth.reauth_required = false;
    *flow = None;
    Ok(json!({"state":"connected","login":login,"providerId":PROVIDER_ID}))
}
#[tauri::command]
pub async fn manual_copilot_disconnect(
    state: TauriState<'_, ManualState>,
    revision: u64,
) -> Result<(), String> {
    let _guard = state.mutation.lock().await;
    let mut doc = Document::load(&state.db)?;
    if doc.revision != revision {
        return Err("配置已变化，请刷新后重试".into());
    }
    *state.copilot.device.lock().await = None;
    doc.providers.retain(|p| p.copilot.is_none());
    doc.validate()?;
    doc.save(&state.db)?;
    let mut auth = state.copilot.state.lock().await;
    auth.credential = None;
    auth.access = None;
    match fs::remove_file(&state.copilot.path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            auth.load_error = true;
            return Err(
                "Copilot 已退出路由，但本机授权文件未能删除；请检查私有目录权限后再次断开".into(),
            );
        }
    }
    auth.load_error = false;
    auth.reauth_required = false;
    Ok(())
}
#[tauri::command]
pub fn manual_copilot_open_login() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(LOGIN_URL).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", LOGIN_URL])
        .spawn();
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open")
        .arg(LOGIN_URL)
        .spawn();
    result
        .map(|_| ())
        .map_err(|_| "无法打开浏览器，请手动打开 https://github.com/login/device".into())
}
