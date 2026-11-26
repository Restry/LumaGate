use super::*;
use crate::manual::{
    catalog::{Document, Source},
    ManualState,
};
use tauri::State as TauriState;

#[tauri::command]
pub async fn manual_codex_start(state: TauriState<'_, ManualState>) -> Result<Challenge, String> {
    state
        .codex
        .start(Document::load(&state.db)?.revision)
        .await
}

#[tauri::command]
pub async fn manual_codex_cancel(
    state: TauriState<'_, ManualState>,
    flow_id: String,
) -> Result<(), String> {
    state.codex.cancel(&flow_id).await;
    Ok(())
}

#[tauri::command]
pub async fn manual_codex_poll(
    state: TauriState<'_, ManualState>,
    flow_id: String,
) -> Result<Value, String> {
    match state.codex.poll(&flow_id).await? {
        Poll::Pending(interval) => Ok(json!({ "state": "pending", "interval": interval })),
        Poll::Ready(authorized) => {
            let _guard = state.mutation.lock().await;
            let result = link_credential(&state.codex, &state.db, Some(&flow_id), authorized).await?;
            state.maybe_auto_sync();
            Ok(result)
        }
    }
}

#[tauri::command]
pub async fn manual_codex_import_local(
    state: TauriState<'_, ManualState>,
) -> Result<Value, String> {
    let _guard = state.mutation.lock().await;
    let doc = Document::load(&state.db)?;
    let local = read_local_codex_auth(&state.codex.home)
        .ok_or("未检测到本机 Codex 登录（~/.codex/auth.json 无有效 ChatGPT 订阅令牌），请先使用浏览器设备码登录")?;
    let credential = Credential {
        version: 1,
        account_id: local.account_id,
        email: local.email,
        plan: local.plan,
        source: "local_codex".into(),
        access_token: local.access_token,
        refresh_token: local.refresh_token,
        id_token: local.id_token,
        expires_at: local.expires_at,
        grant_id: uuid::Uuid::new_v4().to_string(),
    };
    let result = link_credential(
        &state.codex,
        &state.db,
        None,
        Authorized {
            credential,
            revision: doc.revision,
        },
    )
    .await?;
    state.maybe_auto_sync();
    Ok(result)
}

async fn link_credential(
    manager: &Manager,
    db: &crate::database::Database,
    flow_id: Option<&str>,
    authorized: Authorized,
) -> Result<Value, String> {
    let mut doc = Document::load(db)?;
    if doc.revision != authorized.revision {
        if let Some(id) = flow_id {
            manager.cancel(id).await;
        }
        return Err("授权期间配置已变化，请重试；未覆盖现有配置".into());
    }
    let mut flow = manager.device.lock().await;
    if let Some(id) = flow_id {
        if !flow
            .as_ref()
            .is_some_and(|d| d.id == id && Instant::now() < d.expires)
        {
            return Err("授权已取消或过期".into());
        }
    }
    let old = doc.providers.iter().find(|p| p.id == PROVIDER_ID).cloned();
    if old.as_ref().is_some_and(|p| p.codex.is_none()) {
        *flow = None;
        return Err("现有来源使用了 Codex 订阅保留标识，未覆盖现有来源".into());
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
            .unwrap_or_else(|| "Codex 订阅".into()),
        base_url: API_ROOT.into(),
        key_env: String::new(),
        key_ref: None,
        azure_tenant_id: None,
        azure_client_id: None,
        copilot: None,
        codex: Some(binding),
        protocol: Protocol::OpenaiResponses,
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
        let rollback = if let Some(old_cred) = previous {
            manager.persist(&old_cred)
        } else {
            fs::remove_file(&manager.path).map_err(|_| "无法回滚授权文件".into())
        };
        *flow = None;
        if rollback.is_err() {
            auth.load_error = true;
            auth.credential = None;
            return Err("配置未能保存且授权回滚失败，请断开后重试".into());
        }
        return Err(error);
    }
    let email = authorized.credential.email.clone();
    let plan = authorized.credential.plan.clone();
    auth.credential = Some(authorized.credential);
    auth.load_error = false;
    auth.reauth_required = false;
    *flow = None;
    Ok(json!({
        "state": "connected",
        "email": email,
        "plan": plan,
        "providerId": PROVIDER_ID
    }))
}

#[tauri::command]
pub async fn manual_codex_disconnect(
    state: TauriState<'_, ManualState>,
    revision: u64,
) -> Result<(), String> {
    let _guard = state.mutation.lock().await;
    let mut doc = Document::load(&state.db)?;
    if doc.revision != revision {
        return Err("配置已变化，请刷新后重试".into());
    }
    *state.codex.device.lock().await = None;
    doc.providers.retain(|p| p.codex.is_none());
    doc.validate()?;
    doc.save(&state.db)?;
    let mut auth = state.codex.state.lock().await;
    auth.credential = None;
    match fs::remove_file(&state.codex.path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            auth.load_error = true;
            return Err("Codex 订阅已退出路由，但本机凭据文件未能删除，请检查目录权限".into());
        }
    }
    auth.load_error = false;
    auth.reauth_required = false;
    drop(auth);
    state.maybe_auto_sync();
    Ok(())
}

#[tauri::command]
pub fn manual_codex_open_login() -> Result<(), String> {
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
        .map_err(|_| format!("无法打开浏览器，请手动访问 {LOGIN_URL}"))
}
