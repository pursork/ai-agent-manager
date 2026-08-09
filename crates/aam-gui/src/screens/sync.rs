//! Device/sync management panel (`docs/04-webdav-sync-security.md`):
//! vault init/join, device list/revoke, Provider re-encrypt/push/pull,
//! account push/list/pull, and session-index sync (deferred here from
//! Round 2 on purpose -- it needs the same WebDAV connection state every
//! other action on this screen does) -- graphical equivalent of
//! `crates/aam-cli/src/commands.rs::run_device`/`run_sync`/
//! `SessionAction::Sync`.
//!
//! **The one screen that holds real secrets across multiple actions.**
//! Confirmed with the user: the WebDAV password and vault passphrase are
//! remembered in this screen's `State` for the lifetime of this
//! `aam-gui` run (not written anywhere), rather than re-prompted per
//! action like the CLI does -- traded deliberately for convenience given
//! how many actions here share the same connection, and disclosed
//! plainly in the UI (a persistent note + an explicit "清除已记住的密码"
//! button), not hidden.

use std::collections::HashMap;

use aam_memory::ProjectIndex;
use aam_sync::{local_identity, DeviceEntry, WebDavBackend};
use aam_switcher::{AccountCatalogEntry, Profile, ProviderRecord, Tool};
use iced::widget::{button, column, container, pick_list, row, text, text_input};
use iced::{Element, Length, Task};

use crate::style::{primary_button, secondary_button, SPACING_LG, SPACING_MD, SPACING_SM};
use crate::task::perform;

#[derive(Debug, Clone)]
pub enum RowStatus {
    Working,
    Failed(String),
}

pub struct State {
    // -- Connection: URL/user/label aren't secret and are worth keeping
    // around; the two password fields are the one place on this screen
    // (this whole app, in fact) secrets outlive a single action. --
    pub webdav_url: String,
    pub webdav_user: String,
    pub label: String,
    pub webdav_password: String,
    pub vault_passphrase: String,

    pub new_passphrase: String,
    pub new_passphrase_confirm: String,
    pub vault_busy: bool,

    pub devices: Vec<DeviceEntry>,
    pub loading_devices: bool,
    pub reencrypting: bool,
    pub row_status: HashMap<String, RowStatus>,

    pub provider_choice: Option<String>,
    pub provider_sync_busy: bool,

    pub account_push_tool: Tool,
    pub account_push_label: String,
    pub account_pushing: bool,
    pub accounts: Vec<AccountCatalogEntry>,
    pub loading_accounts: bool,
    pub pull_tool: Tool,
    pub pull_key: String,
    pub pull_as: String,
    pub pulling_account: bool,

    pub syncing_sessions: bool,

    pub status_message: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            webdav_url: String::new(),
            webdav_user: String::new(),
            label: String::new(),
            webdav_password: String::new(),
            vault_passphrase: String::new(),
            new_passphrase: String::new(),
            new_passphrase_confirm: String::new(),
            vault_busy: false,
            devices: Vec::new(),
            loading_devices: false,
            reencrypting: false,
            row_status: HashMap::new(),
            provider_choice: None,
            provider_sync_busy: false,
            // `Tool` is defined in `aam_switcher`, so this crate can't add
            // a `Default` impl for it (orphan rule, same reasoning as
            // `screens/profiles.rs`) -- pick a starting value here.
            account_push_tool: Tool::Claude,
            account_push_label: String::new(),
            account_pushing: false,
            accounts: Vec::new(),
            loading_accounts: false,
            pull_tool: Tool::Claude,
            pull_key: String::new(),
            pull_as: String::new(),
            pulling_account: false,
            syncing_sessions: false,
            status_message: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    UrlChanged(String),
    UserChanged(String),
    LabelChanged(String),
    WebdavPasswordChanged(String),
    PassphraseChanged(String),
    ClearRemembered,

    NewPassphraseChanged(String),
    NewPassphraseConfirmChanged(String),
    SubmitInit,
    SubmitJoin,
    VaultOpDone(Result<String, String>),

    RefreshDevices,
    DevicesLoaded(Result<Vec<DeviceEntry>, String>),
    RevokeDevice(String),
    Revoked(String, Result<(), String>),
    Reencrypt,
    ReencryptDone(Result<Vec<(String, String)>, String>),

    ProviderPicked(String),
    PushProvider,
    PullProvider,
    ProviderSyncDone(Result<String, String>),

    AccountPushToolChanged(Tool),
    AccountPushLabelChanged(String),
    PushAccount,
    AccountPushed(Result<String, String>),
    RefreshAccounts,
    AccountsLoaded(Result<Vec<AccountCatalogEntry>, String>),
    UseAccountEntry(AccountCatalogEntry),
    PullToolChanged(Tool),
    PullKeyChanged(String),
    PullAsChanged(String),
    PullAccount,
    AccountPulled(Result<String, String>),

    SyncSessions,
    SessionsSynced(Result<String, String>),
}

fn validate_connection(url: &str, user: &str, password: &str) -> Result<(), String> {
    if url.trim().is_empty() {
        return Err("WebDAV URL 不能为空".to_string());
    }
    if user.trim().is_empty() {
        return Err("WebDAV 用户名不能为空".to_string());
    }
    if password.is_empty() {
        return Err("WebDAV 密码不能为空（上面「连接信息」区域填一下）".to_string());
    }
    Ok(())
}

fn validate_passphrase(passphrase: &str) -> Result<(), String> {
    if passphrase.is_empty() {
        return Err("vault 主密码不能为空（上面「连接信息」区域填一下）".to_string());
    }
    Ok(())
}

fn should_show_revoke(device: &DeviceEntry) -> bool {
    !device.revoked
}

fn sync_state_dir() -> std::path::PathBuf {
    aam_core::aam_home().join("sync")
}

/// Doesn't take `providers` (unlike [`view`]) -- Push/Pull only need the
/// id already sitting in `state.provider_choice`, `aam_switcher`'s own
/// functions look the record up from the registry by id themselves.
pub fn update(state: &mut State, message: Message, profiles: &[Profile]) -> Task<Message> {
    match message {
        Message::UrlChanged(v) => {
            state.webdav_url = v;
            Task::none()
        }
        Message::UserChanged(v) => {
            state.webdav_user = v;
            Task::none()
        }
        Message::LabelChanged(v) => {
            state.label = v;
            Task::none()
        }
        Message::WebdavPasswordChanged(v) => {
            state.webdav_password = v;
            Task::none()
        }
        Message::PassphraseChanged(v) => {
            state.vault_passphrase = v;
            Task::none()
        }
        Message::ClearRemembered => {
            state.webdav_password.clear();
            state.vault_passphrase.clear();
            state.status_message = Some("已清除记住的密码".to_string());
            Task::none()
        }
        Message::NewPassphraseChanged(v) => {
            state.new_passphrase = v;
            Task::none()
        }
        Message::NewPassphraseConfirmChanged(v) => {
            state.new_passphrase_confirm = v;
            Task::none()
        }
        Message::SubmitInit => {
            if let Err(e) = validate_connection(&state.webdav_url, &state.webdav_user, "x") {
                state.status_message = Some(e);
                return Task::none();
            }
            if state.new_passphrase.is_empty() {
                state.status_message = Some("新主密码不能为空".to_string());
                return Task::none();
            }
            if state.new_passphrase != state.new_passphrase_confirm {
                state.status_message = Some("两次输入的新主密码不一致".to_string());
                return Task::none();
            }
            state.vault_busy = true;
            let (url, user, label, passphrase) =
                (state.webdav_url.clone(), state.webdav_user.clone(), state.label.clone(), state.new_passphrase.clone());
            let password = state.webdav_password.clone();
            perform(
                move || {
                    let backend = WebDavBackend::new(url, user, password);
                    let entry =
                        aam_sync::init_vault(&backend, &sync_state_dir(), &passphrase, &label).map_err(|e| e.to_string())?;
                    Ok(format!(
                        "vault 已初始化；本机注册为设备 '{}' ({})",
                        entry.label, entry.device_id
                    ))
                },
                Message::VaultOpDone,
            )
        }
        Message::SubmitJoin => {
            if let Err(e) = validate_connection(&state.webdav_url, &state.webdav_user, &state.webdav_password) {
                state.status_message = Some(e);
                return Task::none();
            }
            if let Err(e) = validate_passphrase(&state.vault_passphrase) {
                state.status_message = Some(e);
                return Task::none();
            }
            state.vault_busy = true;
            let (url, user, password, label, passphrase) = (
                state.webdav_url.clone(),
                state.webdav_user.clone(),
                state.webdav_password.clone(),
                state.label.clone(),
                state.vault_passphrase.clone(),
            );
            perform(
                move || {
                    let backend = WebDavBackend::new(url, user, password);
                    let entry = aam_sync::join_device_to_vault(&backend, &sync_state_dir(), &passphrase, &label)
                        .map_err(|e| e.to_string())?;
                    Ok(format!(
                        "已加入 vault，注册为设备 '{}' ({})；还不能解密现有数据，需要一台已授权设备点「重新加密全部 Provider」",
                        entry.label, entry.device_id
                    ))
                },
                Message::VaultOpDone,
            )
        }
        Message::VaultOpDone(Ok(msg)) => {
            state.vault_busy = false;
            state.new_passphrase.clear();
            state.new_passphrase_confirm.clear();
            state.status_message = Some(msg);
            Task::none()
        }
        Message::VaultOpDone(Err(e)) => {
            state.vault_busy = false;
            state.status_message = Some(format!("操作失败: {e}"));
            Task::none()
        }
        Message::RefreshDevices => {
            if let Err(e) = validate_connection(&state.webdav_url, &state.webdav_user, &state.webdav_password) {
                state.status_message = Some(e);
                return Task::none();
            }
            if let Err(e) = validate_passphrase(&state.vault_passphrase) {
                state.status_message = Some(e);
                return Task::none();
            }
            state.loading_devices = true;
            let (url, user, password, passphrase) = (
                state.webdav_url.clone(),
                state.webdav_user.clone(),
                state.webdav_password.clone(),
                state.vault_passphrase.clone(),
            );
            perform(
                move || {
                    let backend = WebDavBackend::new(url, user, password);
                    aam_sync::list_devices(&backend, &passphrase)
                        .map(|m| m.devices)
                        .map_err(|e| e.to_string())
                },
                Message::DevicesLoaded,
            )
        }
        Message::DevicesLoaded(Ok(devices)) => {
            state.loading_devices = false;
            state.devices = devices;
            Task::none()
        }
        Message::DevicesLoaded(Err(e)) => {
            state.loading_devices = false;
            state.status_message = Some(format!("加载设备列表失败: {e}"));
            Task::none()
        }
        Message::RevokeDevice(device_id) => {
            state.row_status.insert(device_id.clone(), RowStatus::Working);
            let (url, user, password, passphrase) = (
                state.webdav_url.clone(),
                state.webdav_user.clone(),
                state.webdav_password.clone(),
                state.vault_passphrase.clone(),
            );
            let id_for_perform = device_id.clone();
            perform(
                move || {
                    let backend = WebDavBackend::new(url, user, password);
                    aam_sync::revoke_device_in_vault(&backend, &passphrase, &id_for_perform)
                        .map(|_manifest| ())
                        .map_err(|e| e.to_string())
                },
                move |result| Message::Revoked(device_id.clone(), result),
            )
        }
        Message::Revoked(device_id, Ok(())) => {
            state.row_status.remove(&device_id);
            state.status_message = Some(format!(
                "设备 '{device_id}' 已吊销 -- 点「重新加密全部 Provider」让后续推送排除它；已经同步过的历史明文数据不会被远程擦除，这是设计上的已知限制（`08` #13）"
            ));
            if let Some(d) = state.devices.iter_mut().find(|d| d.device_id == device_id) {
                d.revoked = true;
            }
            Task::none()
        }
        Message::Revoked(device_id, Err(e)) => {
            state.row_status.insert(device_id, RowStatus::Failed(e));
            Task::none()
        }
        Message::Reencrypt => {
            if let Err(e) = validate_connection(&state.webdav_url, &state.webdav_user, &state.webdav_password) {
                state.status_message = Some(e);
                return Task::none();
            }
            if let Err(e) = validate_passphrase(&state.vault_passphrase) {
                state.status_message = Some(e);
                return Task::none();
            }
            state.reencrypting = true;
            let (url, user, password, passphrase) = (
                state.webdav_url.clone(),
                state.webdav_user.clone(),
                state.webdav_password.clone(),
                state.vault_passphrase.clone(),
            );
            perform(
                move || {
                    let backend = WebDavBackend::new(url, user, password);
                    let identity = local_identity(&sync_state_dir())
                        .map_err(|e| e.to_string())?
                        .ok_or_else(no_local_identity_error)?;
                    let manifest = aam_sync::list_devices(&backend, &passphrase).map_err(|e| e.to_string())?;
                    let recipients = manifest.active_recipients();
                    let registry = aam_switcher::ProviderRegistry::open_default();
                    let results = aam_switcher::reencrypt_all_known_providers(
                        &backend,
                        &registry,
                        &identity.private_key,
                        &recipients,
                        &identity.device_id,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(results
                        .into_iter()
                        .map(|(id, meta)| {
                            let note = match meta {
                                Some(m) => format!("已重新加密（version {}）", m.version),
                                None => "还没推送过，跳过".to_string(),
                            };
                            (id, note)
                        })
                        .collect())
                },
                Message::ReencryptDone,
            )
        }
        Message::ReencryptDone(Ok(results)) => {
            state.reencrypting = false;
            state.status_message = Some(if results.is_empty() {
                "本机没有已注册的 provider".to_string()
            } else {
                results.into_iter().map(|(id, note)| format!("{id}: {note}")).collect::<Vec<_>>().join("; ")
            });
            Task::none()
        }
        Message::ReencryptDone(Err(e)) => {
            state.reencrypting = false;
            state.status_message = Some(format!("重新加密失败: {e}"));
            Task::none()
        }
        Message::ProviderPicked(id) => {
            state.provider_choice = Some(id);
            Task::none()
        }
        Message::PushProvider => {
            let Some(provider_id) = state.provider_choice.clone() else {
                state.status_message = Some("先选一个 provider".to_string());
                return Task::none();
            };
            if let Err(e) = validate_connection(&state.webdav_url, &state.webdav_user, &state.webdav_password) {
                state.status_message = Some(e);
                return Task::none();
            }
            if let Err(e) = validate_passphrase(&state.vault_passphrase) {
                state.status_message = Some(e);
                return Task::none();
            }
            state.provider_sync_busy = true;
            let (url, user, password, passphrase) = (
                state.webdav_url.clone(),
                state.webdav_user.clone(),
                state.webdav_password.clone(),
                state.vault_passphrase.clone(),
            );
            perform(
                move || {
                    let backend = WebDavBackend::new(url, user, password);
                    let identity = local_identity(&sync_state_dir())
                        .map_err(|e| e.to_string())?
                        .ok_or_else(no_local_identity_error)?;
                    let manifest = aam_sync::list_devices(&backend, &passphrase).map_err(|e| e.to_string())?;
                    let recipients = manifest.active_recipients();
                    let registry = aam_switcher::ProviderRegistry::open_default();
                    let meta = aam_switcher::push_provider(&backend, &registry, &provider_id, &recipients, &identity.device_id)
                        .map_err(|e| e.to_string())?;
                    Ok(format!("已推送 provider '{provider_id}'（version {}）", meta.version))
                },
                Message::ProviderSyncDone,
            )
        }
        Message::PullProvider => {
            let Some(provider_id) = state.provider_choice.clone() else {
                state.status_message = Some("先选一个 provider".to_string());
                return Task::none();
            };
            if let Err(e) = validate_connection(&state.webdav_url, &state.webdav_user, &state.webdav_password) {
                state.status_message = Some(e);
                return Task::none();
            }
            state.provider_sync_busy = true;
            let (url, user, password) = (state.webdav_url.clone(), state.webdav_user.clone(), state.webdav_password.clone());
            perform(
                move || {
                    let backend = WebDavBackend::new(url, user, password);
                    let identity = local_identity(&sync_state_dir())
                        .map_err(|e| e.to_string())?
                        .ok_or_else(no_local_identity_error)?;
                    let registry = aam_switcher::ProviderRegistry::open_default();
                    match aam_switcher::pull_provider(&backend, &registry, &provider_id, &identity.private_key)
                        .map_err(|e| e.to_string())?
                    {
                        Some(meta) => Ok(format!("已拉取 provider '{provider_id}'（version {}）", meta.version)),
                        None => Ok(format!("vault 里没有 provider '{provider_id}' 的记录")),
                    }
                },
                Message::ProviderSyncDone,
            )
        }
        Message::ProviderSyncDone(Ok(msg)) => {
            state.provider_sync_busy = false;
            state.status_message = Some(msg);
            Task::none()
        }
        Message::ProviderSyncDone(Err(e)) => {
            state.provider_sync_busy = false;
            state.status_message = Some(format!("Provider 同步失败: {e}"));
            Task::none()
        }
        Message::AccountPushToolChanged(t) => {
            state.account_push_tool = t;
            Task::none()
        }
        Message::AccountPushLabelChanged(v) => {
            state.account_push_label = v;
            Task::none()
        }
        Message::PushAccount => {
            if state.account_push_label.trim().is_empty() {
                state.status_message = Some("先填要推送的本机 Profile label".to_string());
                return Task::none();
            }
            let profile = profiles
                .iter()
                .find(|p| p.tool == state.account_push_tool && p.label == state.account_push_label)
                .cloned();
            let Some(profile) = profile else {
                state.status_message = Some(format!(
                    "本机没有 {} Profile '{}'",
                    state.account_push_tool, state.account_push_label
                ));
                return Task::none();
            };
            if let Err(e) = validate_connection(&state.webdav_url, &state.webdav_user, &state.webdav_password) {
                state.status_message = Some(e);
                return Task::none();
            }
            if let Err(e) = validate_passphrase(&state.vault_passphrase) {
                state.status_message = Some(e);
                return Task::none();
            }
            state.account_pushing = true;
            let (url, user, password, passphrase) = (
                state.webdav_url.clone(),
                state.webdav_user.clone(),
                state.webdav_password.clone(),
                state.vault_passphrase.clone(),
            );
            perform(
                move || {
                    let backend = WebDavBackend::new(url, user, password);
                    let identity = local_identity(&sync_state_dir())
                        .map_err(|e| e.to_string())?
                        .ok_or_else(no_local_identity_error)?;
                    let manifest = aam_sync::list_devices(&backend, &passphrase).map_err(|e| e.to_string())?;
                    let recipients = manifest.active_recipients();
                    let meta = aam_switcher::push_account(&backend, &profile, &recipients, &identity.device_id, &passphrase)
                        .map_err(|e| e.to_string())?;
                    Ok(format!(
                        "已推送 {} 账号 '{}'（version {}）",
                        profile.tool, profile.label, meta.version
                    ))
                },
                Message::AccountPushed,
            )
        }
        Message::AccountPushed(Ok(msg)) => {
            state.account_pushing = false;
            state.status_message = Some(msg);
            Task::none()
        }
        Message::AccountPushed(Err(e)) => {
            state.account_pushing = false;
            state.status_message = Some(format!("推送账号失败: {e}"));
            Task::none()
        }
        Message::RefreshAccounts => {
            if let Err(e) = validate_connection(&state.webdav_url, &state.webdav_user, &state.webdav_password) {
                state.status_message = Some(e);
                return Task::none();
            }
            if let Err(e) = validate_passphrase(&state.vault_passphrase) {
                state.status_message = Some(e);
                return Task::none();
            }
            state.loading_accounts = true;
            let (url, user, password, passphrase) = (
                state.webdav_url.clone(),
                state.webdav_user.clone(),
                state.webdav_password.clone(),
                state.vault_passphrase.clone(),
            );
            perform(
                move || {
                    let backend = WebDavBackend::new(url, user, password);
                    aam_switcher::list_accounts(&backend, &passphrase).map_err(|e| e.to_string())
                },
                Message::AccountsLoaded,
            )
        }
        Message::AccountsLoaded(Ok(accounts)) => {
            state.loading_accounts = false;
            state.accounts = accounts;
            Task::none()
        }
        Message::AccountsLoaded(Err(e)) => {
            state.loading_accounts = false;
            state.status_message = Some(format!("加载账号列表失败: {e}"));
            Task::none()
        }
        Message::UseAccountEntry(entry) => {
            state.pull_tool = if entry.tool == "codex" { Tool::Codex } else { Tool::Claude };
            state.pull_key = entry.key;
            Task::none()
        }
        Message::PullToolChanged(t) => {
            state.pull_tool = t;
            Task::none()
        }
        Message::PullKeyChanged(v) => {
            state.pull_key = v;
            Task::none()
        }
        Message::PullAsChanged(v) => {
            state.pull_as = v;
            Task::none()
        }
        Message::PullAccount => {
            if state.pull_key.trim().is_empty() || state.pull_as.trim().is_empty() {
                state.status_message = Some("key 和本机 Profile label 都要填".to_string());
                return Task::none();
            }
            if let Err(e) = validate_connection(&state.webdav_url, &state.webdav_user, &state.webdav_password) {
                state.status_message = Some(e);
                return Task::none();
            }
            state.pulling_account = true;
            let tool = state.pull_tool;
            let key = state.pull_key.clone();
            let as_label = state.pull_as.clone();
            let (url, user, password) = (state.webdav_url.clone(), state.webdav_user.clone(), state.webdav_password.clone());
            perform(
                move || {
                    let backend = WebDavBackend::new(url, user, password);
                    let identity = local_identity(&sync_state_dir())
                        .map_err(|e| e.to_string())?
                        .ok_or_else(no_local_identity_error)?;
                    let registry = aam_switcher::ProfileRegistry::open_default();
                    let profile = aam_switcher::pull_account(&backend, &registry, tool, &key, &as_label, &identity.private_key)
                        .map_err(|e| e.to_string())?;
                    Ok(format!(
                        "已拉取账号 '{key}' 到本机 Profile '{}' -- 去 Profiles 页打开终端使用",
                        profile.label
                    ))
                },
                Message::AccountPulled,
            )
        }
        Message::AccountPulled(Ok(msg)) => {
            state.pulling_account = false;
            state.status_message = Some(msg);
            Task::none()
        }
        Message::AccountPulled(Err(e)) => {
            state.pulling_account = false;
            state.status_message = Some(format!("拉取账号失败: {e}"));
            Task::none()
        }
        Message::SyncSessions => {
            if let Err(e) = validate_connection(&state.webdav_url, &state.webdav_user, &state.webdav_password) {
                state.status_message = Some(e);
                return Task::none();
            }
            if let Err(e) = validate_passphrase(&state.vault_passphrase) {
                state.status_message = Some(e);
                return Task::none();
            }
            state.syncing_sessions = true;
            let (url, user, password, passphrase) = (
                state.webdav_url.clone(),
                state.webdav_user.clone(),
                state.webdav_password.clone(),
                state.vault_passphrase.clone(),
            );
            perform(
                move || {
                    let backend = WebDavBackend::new(url, user, password);
                    let identity = local_identity(&sync_state_dir())
                        .map_err(|e| e.to_string())?
                        .ok_or_else(no_local_identity_error)?;
                    let manifest = aam_sync::list_devices(&backend, &passphrase).map_err(|e| e.to_string())?;
                    let recipients = manifest.active_recipients();
                    let local = ProjectIndex::open_default();
                    let mirror = aam_memory::remote_mirror_index();
                    let meta = aam_memory::sync_index(
                        &backend,
                        &local,
                        &mirror,
                        &recipients,
                        &identity.device_id,
                        &identity.private_key,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(format!(
                        "已同步会话索引（version {}）-- 其他设备的记录现在能在 Projects 页看到了",
                        meta.version
                    ))
                },
                Message::SessionsSynced,
            )
        }
        Message::SessionsSynced(Ok(msg)) => {
            state.syncing_sessions = false;
            state.status_message = Some(msg);
            Task::none()
        }
        Message::SessionsSynced(Err(e)) => {
            state.syncing_sessions = false;
            state.status_message = Some(format!("同步会话索引失败: {e}"));
            Task::none()
        }
    }
}

fn no_local_identity_error() -> String {
    "本机还没有 sync 身份 -- 先在上面「初始化新 Vault」或「加入已有 Vault」".to_string()
}

pub fn view<'a>(state: &'a State, profiles: &'a [Profile], providers: &'a [ProviderRecord]) -> Element<'a, Message> {
    let connection = column![
        text("连接信息").size(16),
        text_input("WebDAV URL", &state.webdav_url).on_input(Message::UrlChanged),
        text_input("WebDAV 用户名", &state.webdav_user).on_input(Message::UserChanged),
        text_input("label（这台设备的名字）", &state.label).on_input(Message::LabelChanged),
        text_input("WebDAV 密码", &state.webdav_password).on_input(Message::WebdavPasswordChanged).secure(true),
        text_input("Vault 主密码", &state.vault_passphrase).on_input(Message::PassphraseChanged).secure(true),
        text("密码在本次 aam-gui 运行期间保留在内存里，不写入任何文件；关闭程序或点右边「清除」会清空。").size(12),
        secondary_button("清除已记住的密码", Some(Message::ClearRemembered)),
    ]
    .spacing(SPACING_SM);

    let vault_setup = row![
        column![
            text("初始化新 Vault").size(14),
            text_input("新主密码", &state.new_passphrase).on_input(Message::NewPassphraseChanged).secure(true),
            text_input("确认新主密码", &state.new_passphrase_confirm)
                .on_input(Message::NewPassphraseConfirmChanged)
                .secure(true),
            primary_button(
                if state.vault_busy { "处理中..." } else { "初始化" },
                if state.vault_busy { None } else { Some(Message::SubmitInit) }
            ),
        ]
        .spacing(SPACING_SM)
        .width(Length::FillPortion(1)),
        column![
            text("加入已有 Vault").size(14),
            text("用上面「连接信息」里已经填好的 label + 主密码").size(12),
            secondary_button(
                if state.vault_busy { "处理中..." } else { "加入" },
                if state.vault_busy { None } else { Some(Message::SubmitJoin) }
            ),
        ]
        .spacing(SPACING_SM)
        .width(Length::FillPortion(1)),
    ]
    .spacing(SPACING_LG);

    let mut device_list = column![].spacing(SPACING_SM);
    if state.devices.is_empty() {
        device_list = device_list.push(text("(点「刷新设备列表」查看)"));
    }
    for d in &state.devices {
        let working = matches!(state.row_status.get(&d.device_id), Some(RowStatus::Working));
        let mut r = row![
            text(d.label.clone()).width(Length::Fixed(140.0)),
            text(d.device_id.clone()).width(Length::Fixed(220.0)),
            text(if d.revoked { "已吊销" } else { "有效" }).width(Length::Fixed(70.0)),
            text(d.added_at.clone()).width(Length::Fixed(160.0)),
        ]
        .spacing(SPACING_SM);
        if should_show_revoke(d) {
            r = r.push(
                button(text(if working { "吊销中..." } else { "吊销" }))
                    .on_press_maybe(if working { None } else { Some(Message::RevokeDevice(d.device_id.clone())) })
                    .style(button::danger),
            );
        }
        if let Some(RowStatus::Failed(e)) = state.row_status.get(&d.device_id) {
            r = r.push(text(e.clone()).size(12));
        }
        device_list = device_list.push(r);
    }
    let devices_section = column![
        text("设备管理").size(16),
        row![
            secondary_button(
                if state.loading_devices { "刷新中..." } else { "刷新设备列表" },
                if state.loading_devices { None } else { Some(Message::RefreshDevices) }
            ),
            secondary_button(
                if state.reencrypting { "重新加密中..." } else { "重新加密全部 Provider" },
                if state.reencrypting { None } else { Some(Message::Reencrypt) }
            ),
        ]
        .spacing(SPACING_MD),
        device_list,
    ]
    .spacing(SPACING_SM);

    let provider_ids: Vec<String> = providers.iter().map(|p| p.id.clone()).collect();
    let provider_section = column![
        text("Provider 同步").size(16),
        row![
            pick_list(provider_ids, state.provider_choice.clone(), Message::ProviderPicked).placeholder("选一个 provider..."),
            secondary_button(
                if state.provider_sync_busy { "推送中..." } else { "推送" },
                if state.provider_sync_busy { None } else { Some(Message::PushProvider) }
            ),
            secondary_button(
                if state.provider_sync_busy { "拉取中..." } else { "拉取" },
                if state.provider_sync_busy { None } else { Some(Message::PullProvider) }
            ),
        ]
        .spacing(SPACING_MD),
    ]
    .spacing(SPACING_SM);

    let profile_labels: Vec<String> = profiles
        .iter()
        .filter(|p| p.tool == state.account_push_tool)
        .map(|p| p.label.clone())
        .collect();
    let push_account_section = column![
        text("推送账号").size(14),
        row![
            secondary_button("Claude", Some(Message::AccountPushToolChanged(Tool::Claude))),
            secondary_button("Codex", Some(Message::AccountPushToolChanged(Tool::Codex))),
            text(format!("当前: {}", state.account_push_tool)),
        ]
        .spacing(SPACING_MD),
        pick_list(profile_labels, Some(state.account_push_label.clone()).filter(|l| !l.is_empty()), Message::AccountPushLabelChanged)
            .placeholder("选一个本机 Profile..."),
        primary_button(
            if state.account_pushing { "推送中..." } else { "推送账号" },
            if state.account_pushing { None } else { Some(Message::PushAccount) }
        ),
    ]
    .spacing(SPACING_SM);

    let mut accounts_list = column![].spacing(SPACING_SM);
    if state.accounts.is_empty() {
        accounts_list = accounts_list.push(text("(点「刷新账号列表」查看)"));
    }
    for a in &state.accounts {
        accounts_list = accounts_list.push(
            row![
                text(a.tool.clone()).width(Length::Fixed(70.0)),
                text(a.key.clone()).width(Length::Fixed(220.0)),
                text(a.label_hint.clone()).width(Length::Fixed(140.0)),
                text(a.email_hint.clone().unwrap_or_default()).width(Length::Fixed(160.0)),
                secondary_button("选用", Some(Message::UseAccountEntry(a.clone()))),
            ]
            .spacing(SPACING_SM),
        );
    }
    let pull_account_section = column![
        text("拉取账号").size(14),
        secondary_button(
            if state.loading_accounts { "刷新中..." } else { "刷新账号列表" },
            if state.loading_accounts { None } else { Some(Message::RefreshAccounts) }
        ),
        accounts_list,
        row![
            secondary_button("Claude", Some(Message::PullToolChanged(Tool::Claude))),
            secondary_button("Codex", Some(Message::PullToolChanged(Tool::Codex))),
            text(format!("当前: {}", state.pull_tool)),
        ]
        .spacing(SPACING_MD),
        text_input("key（从上面列表点「选用」，或手填）", &state.pull_key).on_input(Message::PullKeyChanged),
        text_input("拉到本机哪个 Profile label", &state.pull_as).on_input(Message::PullAsChanged),
        primary_button(
            if state.pulling_account { "拉取中..." } else { "拉取账号" },
            if state.pulling_account { None } else { Some(Message::PullAccount) }
        ),
    ]
    .spacing(SPACING_SM);

    let account_section = column![text("账号同步").size(16), push_account_section, pull_account_section].spacing(SPACING_MD);

    let session_section = column![
        text("会话索引同步").size(16),
        primary_button(
            if state.syncing_sessions { "同步中..." } else { "同步会话索引" },
            if state.syncing_sessions { None } else { Some(Message::SyncSessions) }
        ),
    ]
    .spacing(SPACING_SM);

    let status = state
        .status_message
        .as_ref()
        .map(|m| text(m.clone()))
        .unwrap_or_else(|| text(""));

    container(
        iced::widget::scrollable(
            column![
                text("Sync").size(24),
                connection,
                vault_setup,
                devices_section,
                provider_section,
                account_section,
                session_section,
                status,
            ]
            .spacing(SPACING_LG),
        )
        .height(Length::Fill),
    )
    .padding(SPACING_LG)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, revoked: bool) -> DeviceEntry {
        DeviceEntry {
            device_id: id.to_string(),
            label: "x".to_string(),
            age_public_key: "age1xxxxx".to_string(),
            added_at: "2026-08-09T00:00:00Z".to_string(),
            revoked,
        }
    }

    #[test]
    fn validate_connection_rejects_empty_url() {
        assert!(validate_connection("", "user", "pw").is_err());
    }

    #[test]
    fn validate_connection_rejects_empty_user() {
        assert!(validate_connection("https://x", "", "pw").is_err());
    }

    #[test]
    fn validate_connection_rejects_empty_password() {
        assert!(validate_connection("https://x", "user", "").is_err());
    }

    #[test]
    fn validate_connection_accepts_everything_filled() {
        assert!(validate_connection("https://x", "user", "pw").is_ok());
    }

    #[test]
    fn validate_passphrase_rejects_empty() {
        assert!(validate_passphrase("").is_err());
    }

    #[test]
    fn validate_passphrase_accepts_nonempty() {
        assert!(validate_passphrase("hunter2").is_ok());
    }

    #[test]
    fn should_show_revoke_only_for_active_devices() {
        assert!(should_show_revoke(&device("a", false)));
        assert!(!should_show_revoke(&device("b", true)));
    }
}
