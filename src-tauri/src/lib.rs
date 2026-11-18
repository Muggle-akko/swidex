use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use toml_edit::{value, DocumentMut, Item, Table};

const CONFIG_FILE: &str = ".codex/config.toml";
const AUTH_FILE: &str = ".codex/auth.json";
const HISTORY_FILE: &str = ".codex/history.jsonl";
const SESSIONS_DIR: &str = ".codex/sessions";
const ARCHIVED_SESSIONS_DIR: &str = ".codex/archived_sessions";
const STATE_FILE: &str = ".codex/state_5.sqlite";
const BACKUP_ROOT: &str = "CodexBackups";
const SETTINGS_FILE: &str = ".swidex/settings.json";
const UNIFY_BACKUP_DIR: &str = "swidex-history-unify-v1";
const UNIFY_RESTORE_BACKUP_DIR: &str = "swidex-history-unify-restore-v1";
const KEYCHAIN_SERVICE: &str = "Swidex Proxy API Keys";

const ENV_KEYS: [&str; 7] = [
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_API_BASE",
    "BASE_URL",
    "API_BASE_URL",
    "CODEX_ACCESS_TOKEN",
];

const LAUNCHCTL_KEYS: [&str; 6] = [
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_API_BASE",
    "BASE_URL",
    "API_BASE_URL",
];

const PROXY_ENV_KEYS: [&str; 3] = ["OPENAI_BASE_URL", "OPENAI_API_KEY", "CODEX_API_KEY"];

const SHELL_FILES: [&str; 5] = [
    ".zshrc",
    ".zprofile",
    ".bash_profile",
    ".bashrc",
    ".profile",
];

const SUSPICIOUS_DOMAINS: [&str; 2] = ["anyrouter", "muyuan.do"];

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EnvStatus {
    key: String,
    set: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ShellResidue {
    file: String,
    exists: bool,
    matches: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LaunchctlStatus {
    key: String,
    set: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    home: String,
    codex_exists: bool,
    codex_status: String,
    config_exists: bool,
    model_provider: Option<String>,
    base_url: Option<String>,
    env_key: Option<String>,
    has_codex_provider: bool,
    suspicious_domains: Vec<String>,
    env: Vec<EnvStatus>,
    shell_residues: Vec<ShellResidue>,
    launchctl: Vec<LaunchctlStatus>,
    conclusion: String,
    warnings: Vec<String>,
    history_unify: HistoryUnifyStatus,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult {
    ok: bool,
    title: String,
    message: String,
    backups: Vec<String>,
    actions: Vec<String>,
    commands: Vec<String>,
    results: Vec<String>,
    report: DoctorReport,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct SwidexSettings {
    unified_session_history: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HistoryUnifyStatus {
    enabled: bool,
    config_unified: bool,
    session_files: usize,
    archived_files: usize,
    openai_threads: Option<i64>,
    codex_threads: Option<i64>,
    other_threads: Option<i64>,
    sqlite_status: String,
    migration_backups: usize,
    restore_backups: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryBackupMeta {
    kind: String,
    created_at: String,
    codex_home: String,
}

struct HistoryRewriteReport {
    files: usize,
    sqlite_rows: Option<i64>,
}

struct LaunchctlUnsetReport {
    commands: Vec<String>,
    results: Vec<String>,
    unset_count: usize,
    supported: bool,
}

struct LaunchctlSetReport {
    commands: Vec<String>,
    results: Vec<String>,
    set_count: usize,
    supported: bool,
}

#[tauri::command]
fn doctor() -> Result<DoctorReport, String> {
    Ok(build_doctor_report())
}

#[tauri::command]
fn set_history_unify(
    enabled: bool,
    migrate_existing: bool,
    restore_migrated: bool,
) -> Result<OperationResult, String> {
    let home = home_dir()?;
    let stamp = timestamp();
    let mut actions = Vec::new();
    let mut backups = Vec::new();
    let mut commands = Vec::new();
    let mut results = Vec::new();

    ensure_codex_dir(&home)?;
    let config_path = home.join(CONFIG_FILE);
    if config_path.exists() {
        if let Some(backup) = backup_file(&config_path, &stamp)? {
            commands.push(format!(
                "cp {} {}",
                display_path(&config_path),
                display_path(&backup)
            ));
            backups.push(display_path(&backup));
        }
    }

    let mut doc = read_config_or_default(&config_path)?;
    let current_model_provider = doc.get("model_provider").and_then(|v| v.as_str());
    let current_provider = doc.get("model_providers").and_then(|v| v.get("codex"));
    let current_is_proxy =
        current_model_provider == Some("codex") && is_proxy_history_provider(current_provider);
    let current_is_official_unified =
        is_official_history_provider(current_provider) && current_model_provider == Some("codex");
    let mut config_changed = false;
    if enabled {
        if current_is_proxy {
            actions.push(
                "已开启同步对话历史；当前仍保持中转或定制供应商，切回官方时会自动加入同一历史列表。"
                    .to_string(),
            );
            results.push("config.toml: 当前为中转/定制供应商，未切换当前请求通道。".to_string());
        } else {
            set_model_provider(&mut doc, "codex");
            upsert_official_history_provider(&mut doc);
            config_changed = true;
            actions.push(
                "已开启同步对话历史：官方账号会和中转/定制供应商显示在同一历史列表。".to_string(),
            );
            results.push("config.toml: model_provider=codex（官方登录形态）。".to_string());
        }
    } else {
        if current_is_proxy {
            actions.push(
                "已关闭同步对话历史；当前仍保持中转或定制供应商，切回官方时会恢复官方独立历史列表。"
                    .to_string(),
            );
            results.push("config.toml: 当前为中转/定制供应商，未切换当前请求通道。".to_string());
        } else if current_is_official_unified || current_model_provider != Some("codex") {
            set_model_provider(&mut doc, "openai");
            remove_proxy_state(&mut doc);
            config_changed = true;
            actions.push("已关闭同步对话历史：官方账号恢复独立历史列表。".to_string());
            results.push("config.toml: model_provider=openai，codex provider 已移除。".to_string());
        }
    }
    if config_changed {
        write_config(&config_path, &doc)?;
        commands.push(format!("write {}", display_path(&config_path)));
    }

    save_settings(
        &home,
        &SwidexSettings {
            unified_session_history: enabled,
        },
    )?;
    commands.push(format!("write {}", display_path(&home.join(SETTINGS_FILE))));
    results.push(format!("settings: unified_session_history={}", enabled));

    if enabled && migrate_existing {
        let report = migrate_official_history(&home, &stamp)?;
        backups.push(report.0);
        actions.push(format!(
            "已迁入现有官方会话：{} 个会话文件，{} 条索引记录。",
            report.1.files,
            format_optional_count(report.1.sqlite_rows)
        ));
        commands.extend(report.2);
        results.push(format!(
            "history migrate: files={}, sqlite_rows={}",
            report.1.files,
            format_optional_count(report.1.sqlite_rows)
        ));
    } else if enabled {
        actions.push("未迁入旧会话；开启后新建的官方会话会进入共享列表。".to_string());
        results.push("history migrate: skipped by user。".to_string());
    }

    if !enabled && restore_migrated {
        let report = restore_official_history(&home, &stamp)?;
        if let Some(backup) = report.0 {
            backups.push(backup);
        }
        actions.push(format!(
            "已按备份还原可识别的官方会话：{} 个会话文件，{} 条索引记录。",
            report.1.files,
            format_optional_count(report.1.sqlite_rows)
        ));
        commands.extend(report.2);
        results.push(format!(
            "history restore: files={}, sqlite_rows={}",
            report.1.files,
            format_optional_count(report.1.sqlite_rows)
        ));
    } else if !enabled {
        actions.push("未按备份还原；已迁入或开启期间产生的会话会留在 codex 历史桶。".to_string());
        results.push("history restore: skipped by user。".to_string());
    }

    let report = build_doctor_report();
    results.extend(report_results(&report));
    Ok(OperationResult {
        ok: true,
        title: if enabled {
            "同步对话历史已开启".to_string()
        } else {
            "同步对话历史已关闭".to_string()
        },
        message: if enabled {
            "官方账号与中转/定制供应商会进入同一个历史列表；认证和当前请求通道不会被这个开关混用。"
                .to_string()
        } else {
            "官方账号与中转/定制供应商会恢复各自的历史列表；开启期间的新会话会留在中转/定制供应商历史里。".to_string()
        },
        backups,
        actions,
        commands,
        results,
        report,
    })
}

#[tauri::command]
fn use_chatgpt() -> Result<OperationResult, String> {
    let mut actions = Vec::new();
    let mut backups = Vec::new();
    let mut commands = Vec::new();
    let mut results = Vec::new();
    let home = home_dir()?;
    let stamp = timestamp();

    let config_path = home.join(CONFIG_FILE);
    ensure_codex_dir(&home)?;
    if config_path.exists() {
        if let Some(backup) = backup_file(&config_path, &stamp)? {
            commands.push(format!(
                "cp {} {}",
                display_path(&config_path),
                display_path(&backup)
            ));
            backups.push(display_path(&backup));
        }
    }

    let settings = read_settings(&home);
    let mut doc = read_config_or_default(&config_path)?;
    restore_model_catalog(&home, &mut doc)?;
    if settings.unified_session_history {
        set_model_provider(&mut doc, "codex");
        remove_proxy_state(&mut doc);
        upsert_official_history_provider(&mut doc);
    } else {
        set_model_provider(&mut doc, "openai");
        remove_proxy_state(&mut doc);
    }
    write_config(&config_path, &doc)?;
    commands.push(format!("write {}", display_path(&config_path)));
    if settings.unified_session_history {
        actions.push(
            "已切回 ChatGPT 官方账号；同步对话历史开启时，新会话会继续进入共享历史列表。"
                .to_string(),
        );
        results.push(
            "config.toml: model_provider=codex（官方登录形态），中转站 base_url/env_key 已移除。"
                .to_string(),
        );
    } else {
        actions.push(
            "已将 Codex provider 切回 openai，并移除 codex 中转站 provider 配置。".to_string(),
        );
        results.push("config.toml: model_provider=openai，codex provider 已移除。".to_string());
    }

    for shell_file in SHELL_FILES {
        let path = home.join(shell_file);
        if !path.exists() {
            continue;
        }
        if let Some(backup) = backup_file(&path, &stamp)? {
            commands.push(format!(
                "cp {} {}",
                display_path(&path),
                display_path(&backup)
            ));
            backups.push(display_path(&backup));
        }
        let removed = clean_shell_file(&path)?;
        if removed > 0 {
            commands.push(format!(
                "edit {} # remove {} matching lines",
                display_path(&path),
                removed
            ));
            actions.push(format!(
                "已从 {} 移除 {} 行可疑环境配置。",
                shell_file, removed
            ));
            results.push(format!("{}: 移除 {} 行。", shell_file, removed));
        } else {
            results.push(format!("{}: 未发现需要移除的行。", shell_file));
        }
    }

    for key in ENV_KEYS {
        let was_set = env::var_os(key).is_some();
        env::remove_var(key);
        commands.push(format!("unset {}", key));
        results.push(format!(
            "{}: {}",
            key,
            if was_set { "unset" } else { "not set" }
        ));
    }
    actions.push(
        "已清理本应用进程内的相关环境变量。当前已打开终端的父进程环境需要重新打开或手动 unset。"
            .to_string(),
    );

    let launchctl_report = unset_launchctl_envs();
    commands.extend(launchctl_report.commands);
    results.extend(launchctl_report.results);
    if launchctl_report.unset_count > 0 {
        actions.push(format!(
            "已清理 {} 个 launchctl 环境变量。",
            launchctl_report.unset_count
        ));
    } else if !launchctl_report.supported {
        actions.push("当前系统不支持 launchctl，已跳过 GUI 环境变量清理。".to_string());
    } else {
        actions.push("未发现需要清理的 launchctl 环境变量。".to_string());
    }

    let report = build_doctor_report();
    results.extend(report_results(&report));
    Ok(OperationResult {
        ok: true,
        title: "已切换到 ChatGPT 账号模式".to_string(),
        message: "如果 login status 仍不是 ChatGPT/OAuth/account，请运行 codex logout && codex login，并重新打开需要使用 Codex 的应用。".to_string(),
        backups,
        actions,
        commands,
        results,
        report,
    })
}

#[tauri::command]
async fn use_proxy(url: String, key: String, persist_key: bool) -> Result<OperationResult, String> {
    tauri::async_runtime::spawn_blocking(move || use_proxy_impl(url, key, persist_key))
        .await
        .map_err(|err| err.to_string())?
}

fn use_proxy_impl(url: String, key: String, persist_key: bool) -> Result<OperationResult, String> {
    let trimmed_key = key.trim();
    if trimmed_key.is_empty() {
        return Err("API Key 不能为空。".to_string());
    }

    let normalized_url = normalize_proxy_url(&url)?;
    let mut actions = Vec::new();
    let mut backups = Vec::new();
    let mut commands = Vec::new();
    let mut results = Vec::new();
    let home = home_dir()?;
    let stamp = timestamp();
    let config_path = home.join(CONFIG_FILE);
    ensure_codex_dir(&home)?;

    if config_path.exists() {
        if let Some(backup) = backup_file(&config_path, &stamp)? {
            commands.push(format!(
                "cp {} {}",
                display_path(&config_path),
                display_path(&backup)
            ));
            backups.push(display_path(&backup));
        }
    }

    let mut doc = read_config_or_default(&config_path)?;
    set_model_provider(&mut doc, "codex");
    // Fetch before changing the active configuration. A failed refresh must not
    // silently reuse another relay's catalog or partially switch providers.
    let model_ids = fetch_proxy_models(&normalized_url, trimmed_key)?;
    let catalog_count = install_proxy_catalog(&home, &mut doc, &model_ids)?;
    results.push(format!(
        "已同步 {} 个具有完整元数据的模型；其余 {} 个未加入 Codex 目录。",
        catalog_count,
        model_ids.len().saturating_sub(catalog_count)
    ));
    upsert_proxy_provider(&mut doc, &normalized_url);
    write_config(&config_path, &doc)?;
    commands.push(format!("write {}", display_path(&config_path)));
    actions.push(format!("已写入中转站 base_url：{}", normalized_url));
    results.push("config.toml: model_provider=codex。".to_string());
    results.push(format!("base_url: {}", normalized_url));
    results.push("env_key: CODEX_API_KEY".to_string());

    set_runtime_proxy_envs(&normalized_url, trimmed_key);
    commands.push(format!("setenv OPENAI_BASE_URL {}", normalized_url));
    commands.push("setenv OPENAI_API_KEY ***".to_string());
    commands.push("setenv CODEX_API_KEY ***".to_string());
    actions.push(
        "已在 Swidex 当前进程设置 OPENAI_BASE_URL、OPENAI_API_KEY 和 CODEX_API_KEY。".to_string(),
    );
    results.push(format!("OPENAI_BASE_URL: {}", normalized_url));
    results.push("OPENAI_API_KEY: 已设置（日志已隐藏明文）。".to_string());
    results.push("CODEX_API_KEY: 已设置（日志已隐藏明文）。".to_string());

    if persist_key {
        let zshrc = home.join(".zshrc");
        if zshrc.exists() {
            if let Some(backup) = backup_file(&zshrc, &stamp)? {
                commands.push(format!(
                    "cp {} {}",
                    display_path(&zshrc),
                    display_path(&backup)
                ));
                backups.push(display_path(&backup));
            }
        }
        persist_proxy_env(&zshrc, &normalized_url, trimmed_key)?;
        commands.push(format!(
            "write {} # export OPENAI_BASE_URL / OPENAI_API_KEY / CODEX_API_KEY",
            display_path(&zshrc)
        ));
        actions.push(
            "已按你的选择把 OPENAI_BASE_URL、OPENAI_API_KEY 和 CODEX_API_KEY 持久化到 ~/.zshrc。"
                .to_string(),
        );
        results.push("OPENAI_BASE_URL: 已持久化到 ~/.zshrc。".to_string());
        results.push("OPENAI_API_KEY: 已持久化到 ~/.zshrc（日志已隐藏明文）。".to_string());
        results.push("CODEX_API_KEY: 已持久化到 ~/.zshrc（日志已隐藏明文）。".to_string());

        let launchctl_report = set_launchctl_proxy_envs(&normalized_url, trimmed_key);
        commands.extend(launchctl_report.commands);
        results.extend(launchctl_report.results);
        if launchctl_report.set_count > 0 {
            actions.push(format!(
                "已同步 {} 个 launchctl 环境变量，重新打开 GUI 应用后会继承。",
                launchctl_report.set_count
            ));
        } else if !launchctl_report.supported {
            actions.push("当前系统不支持 launchctl，已跳过 GUI 环境变量同步。".to_string());
        } else {
            actions.push("launchctl 环境变量同步未成功，请查看结果日志。".to_string());
        }
    } else {
        actions.push("未持久化 API key。可用下方临时命令从终端启动 Codex。".to_string());
        results.push("CODEX_API_KEY: 未持久化。".to_string());
    }

    let report = build_doctor_report();
    results.extend(report_results(&report));
    Ok(OperationResult {
        ok: true,
        title: "已切换到 API / 中转站模式".to_string(),
        message: if persist_key {
            "关闭并重新打开终端或相关应用后，持久化的 API 环境变量才会被新进程继承。".to_string()
        } else {
            format!(
                "临时启动命令：OPENAI_BASE_URL=\"{}\" OPENAI_API_KEY=\"{}\" CODEX_API_KEY=\"{}\" codex",
                normalized_url,
                mask_key(trimmed_key),
                mask_key(trimmed_key)
            )
        },
        backups,
        actions,
        commands,
        results,
        report,
    })
}

#[tauri::command]
fn save_provider_secret(profile_id: String, key: String) -> Result<(), String> {
    let trimmed_key = key.trim();
    if trimmed_key.is_empty() {
        return delete_provider_secret(profile_id);
    }
    write_keychain_secret(&profile_id, trimmed_key)
}

#[tauri::command]
fn get_provider_secret(profile_id: String) -> Result<Option<String>, String> {
    read_keychain_secret(&profile_id)
}

#[tauri::command]
fn delete_provider_secret(profile_id: String) -> Result<(), String> {
    delete_keychain_secret(&profile_id)
}

#[tauri::command]
fn backup_history() -> Result<OperationResult, String> {
    let home = home_dir()?;
    let stamp = timestamp();
    let backup_dir = home
        .join(BACKUP_ROOT)
        .join(format!("codex-history-{}", stamp));
    fs::create_dir_all(&backup_dir).map_err(to_string_err)?;

    let mut actions = Vec::new();
    let mut commands = vec![format!("mkdir -p {}", display_path(&backup_dir))];
    let mut results = vec![format!("backup_dir: {}", display_path(&backup_dir))];
    let sessions = home.join(SESSIONS_DIR);
    if sessions.exists() {
        copy_dir_recursive(&sessions, &backup_dir.join("sessions")).map_err(to_string_err)?;
        commands.push(format!(
            "cp -R {} {}",
            display_path(&sessions),
            display_path(&backup_dir.join("sessions"))
        ));
        actions.push("已备份 sessions/。".to_string());
        results.push("sessions/: copied。".to_string());
    } else {
        actions.push("未发现 ~/.codex/sessions/，已跳过。".to_string());
        results.push("sessions/: not found，skipped。".to_string());
    }

    let history = home.join(HISTORY_FILE);
    if history.exists() {
        let bytes = fs::copy(&history, backup_dir.join("history.jsonl")).map_err(to_string_err)?;
        commands.push(format!(
            "cp {} {}",
            display_path(&history),
            display_path(&backup_dir.join("history.jsonl"))
        ));
        actions.push("已备份 history.jsonl。".to_string());
        results.push(format!("history.jsonl: copied {} bytes。", bytes));
    } else {
        actions.push("未发现 ~/.codex/history.jsonl，已跳过。".to_string());
        results.push("history.jsonl: not found，skipped。".to_string());
    }

    actions.push("未备份 auth.json。".to_string());
    results.push("auth.json: excluded。".to_string());

    let report = build_doctor_report();
    results.extend(report_results(&report));
    Ok(OperationResult {
        ok: true,
        title: "历史记录备份完成".to_string(),
        message: display_path(&backup_dir),
        backups: vec![display_path(&backup_dir)],
        actions,
        commands,
        results,
        report,
    })
}

#[tauri::command]
fn reset_auth() -> Result<OperationResult, String> {
    let home = home_dir()?;
    let stamp = timestamp();
    let auth = home.join(AUTH_FILE);
    let mut backups = Vec::new();
    let mut actions = Vec::new();
    let mut commands = Vec::new();
    let mut results = Vec::new();

    if auth.exists() {
        if let Some(backup) = backup_file(&auth, &stamp)? {
            commands.push(format!(
                "cp {} {}",
                display_path(&auth),
                display_path(&backup)
            ));
            backups.push(display_path(&backup));
        }
        fs::remove_file(&auth).map_err(to_string_err)?;
        commands.push(format!("rm {}", display_path(&auth)));
        actions.push("已删除 auth.json。下一次使用 Codex 需要重新登录。".to_string());
        results.push("auth.json: backed up and removed。".to_string());
    } else {
        actions.push("未发现 auth.json，无需重置。".to_string());
        results.push("auth.json: not found。".to_string());
    }

    let report = build_doctor_report();
    results.extend(report_results(&report));
    Ok(OperationResult {
        ok: true,
        title: "认证缓存已重置".to_string(),
        message: "请运行 codex login，并选择 Sign in with ChatGPT。".to_string(),
        backups,
        actions,
        commands,
        results,
        report,
    })
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            doctor,
            set_history_unify,
            use_chatgpt,
            use_proxy,
            save_provider_secret,
            get_provider_secret,
            delete_provider_secret,
            backup_history,
            reset_auth
        ])
        .run(tauri::generate_context!())
        .expect("error while running Swidex");
}

fn build_doctor_report() -> DoctorReport {
    let home = match home_dir() {
        Ok(path) => path,
        Err(err) => {
            return DoctorReport {
                home: "unknown".to_string(),
                codex_exists: false,
                codex_status: format!("无法定位 HOME：{}", err),
                config_exists: false,
                model_provider: None,
                base_url: None,
                env_key: None,
                has_codex_provider: false,
                suspicious_domains: Vec::new(),
                env: env_statuses(),
                shell_residues: Vec::new(),
                launchctl: launchctl_statuses(),
                conclusion: "当前状态不确定".to_string(),
                warnings: vec![err],
                history_unify: HistoryUnifyStatus::fallback(),
            };
        }
    };

    let config_path = home.join(CONFIG_FILE);
    let config_text = fs::read_to_string(&config_path).unwrap_or_default();
    let doc = config_text.parse::<DocumentMut>().ok();
    let model_provider = doc
        .as_ref()
        .and_then(|d| d.get("model_provider"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    let codex_provider = doc
        .as_ref()
        .and_then(|d| d.get("model_providers"))
        .and_then(|v| v.get("codex"));
    let base_url = codex_provider
        .and_then(|v| v.get("base_url"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            doc.as_ref()
                .and_then(|d| d.get("openai_base_url"))
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned)
        });
    let env_key = codex_provider
        .and_then(|v| v.get("env_key"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    let history_unify = build_history_unify_status(&home, doc.as_ref());
    let official_unified = history_unify.config_unified;

    let suspicious_domains = suspicious_hits(&config_text);
    let env = env_statuses();
    let shell_residues = shell_residues(&home);
    let launchctl = launchctl_statuses();
    let codex_status = run_codex_status();
    let codex_exists = command_exists("codex");

    let env_any = env.iter().any(|item| item.set);
    let shell_any = shell_residues.iter().any(|item| !item.matches.is_empty());
    let launch_any = launchctl.iter().any(|item| item.set);
    let proxyish = (model_provider.as_deref() == Some("codex") && !official_unified)
        || base_url.is_some()
        || env_key.is_some()
        || !suspicious_domains.is_empty()
        || env_any
        || shell_any
        || launch_any;

    let conclusion = if official_unified && !proxyish {
        "当前看起来是 ChatGPT 账号模式（统一历史已开启）".to_string()
    } else if proxyish {
        "当前看起来仍可能走 API / 中转站".to_string()
    } else if model_provider.as_deref() == Some("openai") || model_provider.is_none() {
        "当前看起来是 ChatGPT 账号模式".to_string()
    } else {
        "当前状态不确定".to_string()
    };

    let mut warnings = Vec::new();
    if !codex_exists {
        warnings.push("未在 PATH 中找到 codex 命令。".to_string());
    }
    if !config_path.exists() {
        warnings.push("未发现 ~/.codex/config.toml，切换时会创建最小配置。".to_string());
    }
    if env_any {
        warnings.push("当前应用进程里存在 API/Base URL 环境变量。".to_string());
    }
    if shell_any {
        warnings.push("shell 启动文件中发现疑似中转站或 API key 配置残留。".to_string());
    }
    if launch_any {
        warnings.push("launchctl 中发现环境变量残留，从 Dock 启动的应用可能会继承。".to_string());
    }

    DoctorReport {
        home: display_path(&home),
        codex_exists,
        codex_status,
        config_exists: config_path.exists(),
        model_provider,
        base_url,
        env_key,
        has_codex_provider: codex_provider.is_some(),
        suspicious_domains,
        env,
        shell_residues,
        launchctl,
        conclusion,
        warnings,
        history_unify,
    }
}

fn home_dir() -> Result<PathBuf, String> {
    if let Ok(path) = env::var("CODEX_SWITCH_HOME") {
        if !path.trim().is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "HOME 环境变量不存在。".to_string())
}

fn timestamp() -> String {
    Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn ensure_codex_dir(home: &Path) -> Result<(), String> {
    fs::create_dir_all(home.join(".codex")).map_err(to_string_err)
}

fn backup_file(path: &Path, stamp: &str) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| format!("无法识别文件名：{}", display_path(path)))?;
    let backup_name = format!("{}.backup.swidex.{}", file_name, stamp);
    let backup_path = path.with_file_name(backup_name);
    fs::copy(path, &backup_path).map_err(to_string_err)?;
    Ok(Some(backup_path))
}

fn read_config_or_default(path: &Path) -> Result<DocumentMut, String> {
    if path.exists() {
        let text = fs::read_to_string(path).map_err(to_string_err)?;
        text.parse::<DocumentMut>()
            .map_err(|err| format!("config.toml 解析失败：{}", err))
    } else {
        Ok(DocumentMut::new())
    }
}

fn write_config(path: &Path, doc: &DocumentMut) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(to_string_err)?;
    }
    fs::write(path, doc.to_string()).map_err(to_string_err)
}

fn read_settings(home: &Path) -> SwidexSettings {
    let path = home.join(SETTINGS_FILE);
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<SwidexSettings>(&text).ok())
        .unwrap_or_default()
}

fn save_settings(home: &Path, settings: &SwidexSettings) -> Result<(), String> {
    let path = home.join(SETTINGS_FILE);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(to_string_err)?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(to_string_err)?;
    fs::write(path, format!("{}\n", text)).map_err(to_string_err)
}

// Keep a separate catalog rather than changing Codex's account-bound cache.
const MODEL_CATALOG: &str = ".swidex/codex-models.json";
const MODEL_CATALOG_STATE: &str = ".swidex/model-catalog-state.json";

fn fetch_proxy_models(url: &str, key: &str) -> Result<Vec<String>, String> {
    if key.chars().any(char::is_control) {
        return Err("API Key 包含无效控制字符。".into());
    }
    // Send credentials through stdin, never command-line arguments or logs.
    let header = format!("Authorization: Bearer {}", key)
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let mut child = Command::new("/usr/bin/curl")
        .args([
            "--silent",
            "--fail",
            "--connect-timeout",
            "10",
            "--max-time",
            "30",
            "--max-filesize",
            "8388608",
            "--config",
            "-",
            "--url",
            &format!("{}/models", url),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "无法启动模型列表请求。".to_string())?;
    let input = format!("header = \"{}\"\n", header);
    let write_result = child.stdin.take().unwrap().write_all(input.as_bytes());
    let output = child
        .wait_with_output()
        .map_err(|_| "读取模型列表失败。".to_string())?;
    if write_result.is_err() || !output.status.success() {
        return Err("模型列表同步失败，请检查中转站地址、密钥或网络；当前配置未切换。".into());
    }
    let response: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "中转站模型列表不是有效 JSON。".to_string())?;
    parse_proxy_model_ids(&response)
}

fn parse_proxy_model_ids(response: &serde_json::Value) -> Result<Vec<String>, String> {
    let data = response
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "中转站模型列表缺少 data 数组。".to_string())?;
    let mut ids = Vec::new();
    for model in data {
        if let Some(id) = model.get("id").and_then(|v| v.as_str()) {
            if !id.is_empty() && !ids.iter().any(|x| x == id) {
                ids.push(id.to_string());
            }
        }
    }
    if ids.is_empty() {
        return Err("中转站返回了空模型列表。".into());
    }
    Ok(ids)
}

fn build_model_catalog(
    ids: &[String],
    sources: &[serde_json::Value],
) -> Result<serde_json::Value, String> {
    let mut metadata = BTreeMap::new();
    for source in sources {
        if let Some(models) = source.get("models").and_then(|v| v.as_array()) {
            for model in models {
                if let Some(slug) = model.get("slug").and_then(|v| v.as_str()) {
                    metadata.insert(slug.to_string(), model.clone());
                }
            }
        }
    }
    let mut models = Vec::new();
    for id in ids {
        if let Some(model) = metadata.get(id) {
            // Retain the exact reasoning, context, tool and instruction metadata.
            // Hidden/internal models and image generators must stay hidden.
            models.push(model.clone());
        }
    }
    if models.is_empty() {
        return Err("未找到可匹配的 Codex 模型元数据。请先使用官方模式刷新模型列表，再重试切换；现有配置已保留。".into());
    }
    Ok(serde_json::json!({"models": models}))
}

fn install_proxy_catalog(
    home: &Path,
    doc: &mut DocumentMut,
    ids: &[String],
) -> Result<usize, String> {
    let catalog_path = home.join(MODEL_CATALOG);
    let mut sources = Vec::new();
    // The account cache is the latest authority; an earlier Swidex catalog is
    // only a fallback for metadata absent from the newer account cache.
    for path in [catalog_path.clone(), home.join(".codex/models_cache.json")] {
        if let Ok(bytes) = fs::read(path) {
            if let Ok(source) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                sources.push(source);
            }
        }
    }
    let catalog = build_model_catalog(ids, &sources)?;
    fs::create_dir_all(home.join(".swidex")).map_err(to_string_err)?;
    let current = doc.get("model_catalog_json").and_then(|v| v.as_str());
    if current != catalog_path.to_str() {
        let state = serde_json::json!({"previous": current});
        write_atomic(
            &home.join(MODEL_CATALOG_STATE),
            &serde_json::to_vec_pretty(&state).map_err(to_string_err)?,
        )?;
    }
    write_atomic(
        &catalog_path,
        &serde_json::to_vec_pretty(&catalog).map_err(to_string_err)?,
    )?;
    doc["model_catalog_json"] = value(catalog_path.to_string_lossy().as_ref());
    Ok(catalog["models"].as_array().unwrap().len())
}

fn restore_model_catalog(home: &Path, doc: &mut DocumentMut) -> Result<(), String> {
    let path = home.join(MODEL_CATALOG);
    if doc.get("model_catalog_json").and_then(|v| v.as_str()) != path.to_str() {
        return Ok(());
    }
    let bytes = fs::read(home.join(MODEL_CATALOG_STATE)).map_err(to_string_err)?;
    let state: serde_json::Value = serde_json::from_slice(&bytes).map_err(to_string_err)?;
    if let Some(previous) = state.get("previous").and_then(|v| v.as_str()) {
        doc["model_catalog_json"] = value(previous);
    } else {
        doc.as_table_mut().remove("model_catalog_json");
    }
    Ok(())
}

fn set_model_provider(doc: &mut DocumentMut, provider: &str) {
    doc["model_provider"] = value(provider);
}

fn remove_proxy_state(doc: &mut DocumentMut) {
    doc.as_table_mut().remove("openai_base_url");
    if let Some(providers) = doc.get_mut("model_providers") {
        if let Some(table) = providers.as_table_mut() {
            table.remove("codex");
            let should_remove = table.is_empty();
            if should_remove {
                doc.as_table_mut().remove("model_providers");
            }
        }
    }
}

fn upsert_proxy_provider(doc: &mut DocumentMut, url: &str) {
    if !doc.as_table().contains_key("model_providers") {
        doc["model_providers"] = Item::Table(Table::new());
    }
    doc["model_providers"]["codex"]["name"] = value("codex");
    doc["model_providers"]["codex"]["wire_api"] = value("responses");
    doc["model_providers"]["codex"]["requires_openai_auth"] = value(false);
    doc["model_providers"]["codex"]["base_url"] = value(url);
    doc["model_providers"]["codex"]["supports_websockets"] = value(false);
    doc["model_providers"]["codex"]["env_key"] = value("CODEX_API_KEY");
}

fn upsert_official_history_provider(doc: &mut DocumentMut) {
    doc.as_table_mut().remove("openai_base_url");
    if !doc.as_table().contains_key("model_providers") {
        doc["model_providers"] = Item::Table(Table::new());
    }
    doc["model_providers"]["codex"]["name"] = value("OpenAI");
    doc["model_providers"]["codex"]["wire_api"] = value("responses");
    doc["model_providers"]["codex"]["requires_openai_auth"] = value(true);
    doc["model_providers"]["codex"]["supports_websockets"] = value(true);
    if let Some(provider) = doc
        .get_mut("model_providers")
        .and_then(|item| item.get_mut("codex"))
        .and_then(|item| item.as_table_mut())
    {
        provider.remove("base_url");
        provider.remove("env_key");
    }
}

fn is_official_history_provider(provider: Option<&Item>) -> bool {
    let Some(provider) = provider else {
        return false;
    };
    let requires_openai_auth = provider
        .get("requires_openai_auth")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let has_base_url = provider.get("base_url").and_then(|v| v.as_str()).is_some();
    let has_env_key = provider.get("env_key").and_then(|v| v.as_str()).is_some();
    requires_openai_auth && !has_base_url && !has_env_key
}

fn is_proxy_history_provider(provider: Option<&Item>) -> bool {
    let Some(provider) = provider else {
        return false;
    };
    let has_base_url = provider.get("base_url").and_then(|v| v.as_str()).is_some();
    let has_env_key = provider.get("env_key").and_then(|v| v.as_str()).is_some();
    let requires_openai_auth = provider
        .get("requires_openai_auth")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    has_base_url || has_env_key || !requires_openai_auth
}

impl HistoryUnifyStatus {
    fn fallback() -> Self {
        HistoryUnifyStatus {
            enabled: false,
            config_unified: false,
            session_files: 0,
            archived_files: 0,
            openai_threads: None,
            codex_threads: None,
            other_threads: None,
            sqlite_status: "unavailable".to_string(),
            migration_backups: 0,
            restore_backups: 0,
        }
    }
}

fn build_history_unify_status(home: &Path, doc: Option<&DocumentMut>) -> HistoryUnifyStatus {
    let settings = read_settings(home);
    let config_unified = doc
        .map(|doc| {
            doc.get("model_provider").and_then(|v| v.as_str()) == Some("codex")
                && is_official_history_provider(
                    doc.get("model_providers").and_then(|v| v.get("codex")),
                )
        })
        .unwrap_or(false);
    let session_files = list_jsonl_files(&home.join(SESSIONS_DIR)).len();
    let archived_files = list_jsonl_files(&home.join(ARCHIVED_SESSIONS_DIR)).len();
    let (openai_threads, codex_threads, other_threads, sqlite_status) =
        sqlite_provider_counts(&home.join(STATE_FILE));

    HistoryUnifyStatus {
        enabled: settings.unified_session_history,
        config_unified,
        session_files,
        archived_files,
        openai_threads,
        codex_threads,
        other_threads,
        sqlite_status,
        migration_backups: backup_generation_count(home, UNIFY_BACKUP_DIR),
        restore_backups: backup_generation_count(home, UNIFY_RESTORE_BACKUP_DIR),
    }
}

fn backup_generation_count(home: &Path, name: &str) -> usize {
    fs::read_dir(home.join(BACKUP_ROOT).join(name))
        .ok()
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_dir())
                .count()
        })
        .unwrap_or(0)
}

fn clean_shell_file(path: &Path) -> Result<usize, String> {
    let text = fs::read_to_string(path).map_err(to_string_err)?;
    let mut removed = 0usize;
    let mut kept = Vec::new();
    for line in text.lines() {
        if is_shell_residue_line(line) {
            removed += 1;
        } else {
            kept.push(line);
        }
    }
    let trailing_newline = if text.ends_with('\n') { "\n" } else { "" };
    fs::write(path, format!("{}{}", kept.join("\n"), trailing_newline)).map_err(to_string_err)?;
    Ok(removed)
}

fn is_shell_residue_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return false;
    }
    let lower = trimmed.to_lowercase();
    let looks_like_assignment = trimmed.starts_with("export ")
        || trimmed.starts_with("setenv ")
        || trimmed.starts_with("unset ")
        || trimmed.contains("=");
    looks_like_assignment
        && (line_sets_any_env_key(trimmed, &ENV_KEYS)
            || SUSPICIOUS_DOMAINS
                .iter()
                .any(|domain| lower.contains(domain)))
}

fn unset_launchctl_envs() -> LaunchctlUnsetReport {
    let mut commands = Vec::new();
    let mut results = Vec::new();
    if !cfg!(target_os = "macos") {
        return LaunchctlUnsetReport {
            commands,
            results: LAUNCHCTL_KEYS
                .iter()
                .map(|key| format!("launchctl {}: skipped (not macOS)", key))
                .collect(),
            unset_count: 0,
            supported: false,
        };
    }
    let mut count = 0usize;
    for key in LAUNCHCTL_KEYS {
        commands.push(format!("launchctl getenv {}", key));
        match Command::new("launchctl").args(["getenv", key]).output() {
            Ok(output) => {
                let is_set = output.status.success()
                    && !String::from_utf8_lossy(&output.stdout).trim().is_empty();
                if is_set {
                    commands.push(format!("launchctl unsetenv {}", key));
                    match Command::new("launchctl").args(["unsetenv", key]).status() {
                        Ok(status) if status.success() => {
                            count += 1;
                            results.push(format!("launchctl {}: unset", key));
                        }
                        Ok(status) => {
                            results.push(format!("launchctl {}: unset failed ({})", key, status))
                        }
                        Err(err) => {
                            results.push(format!("launchctl {}: unset failed ({})", key, err))
                        }
                    }
                } else {
                    results.push(format!("launchctl {}: not set", key));
                }
            }
            Err(err) => results.push(format!("launchctl {}: check failed ({})", key, err)),
        }
    }
    LaunchctlUnsetReport {
        commands,
        results,
        unset_count: count,
        supported: true,
    }
}

fn set_launchctl_proxy_envs(base_url: &str, key: &str) -> LaunchctlSetReport {
    let mut commands = Vec::new();
    let mut results = Vec::new();
    if !cfg!(target_os = "macos") {
        return LaunchctlSetReport {
            commands,
            results: PROXY_ENV_KEYS
                .iter()
                .map(|env_key| format!("launchctl {}: skipped (not macOS)", env_key))
                .collect(),
            set_count: 0,
            supported: false,
        };
    }

    let mut count = 0usize;
    for (env_key, value) in proxy_env_pairs(base_url, key) {
        commands.push(format!(
            "launchctl setenv {} {}",
            env_key,
            redacted_env_value(env_key, value)
        ));
        match Command::new("launchctl")
            .args(["setenv", env_key, value])
            .status()
        {
            Ok(status) if status.success() => {
                count += 1;
                results.push(format!("launchctl {}: set", env_key));
            }
            Ok(status) => results.push(format!("launchctl {}: set failed ({})", env_key, status)),
            Err(err) => results.push(format!("launchctl {}: set failed ({})", env_key, err)),
        }
    }

    LaunchctlSetReport {
        commands,
        results,
        set_count: count,
        supported: true,
    }
}

fn normalize_proxy_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/').to_string();
    if trimmed.is_empty() {
        return Err("Base URL 不能为空。".to_string());
    }
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return Err("Base URL 需要以 http:// 或 https:// 开头。".to_string());
    }
    if trimmed.ends_with("/v1") {
        Ok(trimmed)
    } else {
        Ok(format!("{}/v1", trimmed))
    }
}

fn set_runtime_proxy_envs(base_url: &str, key: &str) {
    for (env_key, value) in proxy_env_pairs(base_url, key) {
        env::set_var(env_key, value);
    }
}

fn write_keychain_secret(profile_id: &str, key: &str) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("API Key 加密保存目前仅支持 macOS Keychain。".to_string());
    }
    let account = keychain_account(profile_id)?;
    let status = Command::new("/usr/bin/security")
        .args([
            "add-generic-password",
            "-a",
            account.as_str(),
            "-s",
            KEYCHAIN_SERVICE,
            "-w",
            key,
            "-U",
        ])
        .status()
        .map_err(|err| format!("无法写入 macOS Keychain：{}", err))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("macOS Keychain 写入失败：{}", status))
    }
}

fn read_keychain_secret(profile_id: &str) -> Result<Option<String>, String> {
    if !cfg!(target_os = "macos") {
        return Err("API Key 加密读取目前仅支持 macOS Keychain。".to_string());
    }
    let account = keychain_account(profile_id)?;
    let output = Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-a",
            account.as_str(),
            "-s",
            KEYCHAIN_SERVICE,
            "-w",
        ])
        .output()
        .map_err(|err| format!("无法读取 macOS Keychain：{}", err))?;
    if output.status.success() {
        let secret = String::from_utf8_lossy(&output.stdout)
            .trim_end_matches(['\r', '\n'])
            .to_string();
        Ok(Some(secret))
    } else if keychain_item_not_found(&output) {
        Ok(None)
    } else {
        Err(format_security_error("读取", &output))
    }
}

fn delete_keychain_secret(profile_id: &str) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("API Key 加密删除目前仅支持 macOS Keychain。".to_string());
    }
    let account = keychain_account(profile_id)?;
    let output = Command::new("/usr/bin/security")
        .args([
            "delete-generic-password",
            "-a",
            account.as_str(),
            "-s",
            KEYCHAIN_SERVICE,
        ])
        .output()
        .map_err(|err| format!("无法删除 macOS Keychain 项：{}", err))?;
    if output.status.success() || keychain_item_not_found(&output) {
        Ok(())
    } else {
        Err(format_security_error("删除", &output))
    }
}

fn keychain_account(profile_id: &str) -> Result<String, String> {
    let trimmed = profile_id.trim();
    if trimmed.is_empty() {
        return Err("中转站 ID 不能为空。".to_string());
    }
    Ok(format!("swidex-provider-{}", trimmed))
}

fn keychain_item_not_found(output: &std::process::Output) -> bool {
    output.status.code() == Some(44)
        || String::from_utf8_lossy(&output.stderr)
            .to_lowercase()
            .contains("could not be found")
}

fn format_security_error(action: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        "无详细信息"
    };
    format!("macOS Keychain {}失败：{}", action, detail)
}

fn persist_proxy_env(path: &Path, base_url: &str, key: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(to_string_err)?;
    }
    let existing = fs::read_to_string(path).unwrap_or_default();
    let cleaned = existing
        .lines()
        .filter(|line| !is_proxy_env_line(line))
        .collect::<Vec<_>>()
        .join("\n");
    let proxy_exports = proxy_env_pairs(base_url, key)
        .iter()
        .map(|(env_key, value)| format!("export {}={}", env_key, shell_double_quote(value)))
        .collect::<Vec<_>>()
        .join("\n");
    let next = if cleaned.trim().is_empty() {
        format!("{}\n", proxy_exports)
    } else {
        format!("{}\n{}\n", cleaned, proxy_exports)
    };
    fs::write(path, next).map_err(to_string_err)
}

fn proxy_env_pairs<'a>(base_url: &'a str, key: &'a str) -> [(&'static str, &'a str); 3] {
    [
        ("OPENAI_BASE_URL", base_url),
        ("OPENAI_API_KEY", key),
        ("CODEX_API_KEY", key),
    ]
}

fn is_proxy_env_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return false;
    }
    line_sets_any_env_key(trimmed, &PROXY_ENV_KEYS)
}

fn line_sets_any_env_key(line: &str, keys: &[&str]) -> bool {
    shell_env_keys(line)
        .iter()
        .any(|candidate| keys.iter().any(|key| candidate.eq_ignore_ascii_case(key)))
}

fn shell_env_keys(line: &str) -> Vec<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return Vec::new();
    }

    let payload = trimmed
        .strip_prefix("export ")
        .or_else(|| trimmed.strip_prefix("setenv "))
        .or_else(|| trimmed.strip_prefix("unset "))
        .unwrap_or(trimmed);

    payload
        .split_whitespace()
        .filter_map(|part| {
            let key = part
                .split_once('=')
                .map(|(left, _)| left)
                .unwrap_or(part)
                .trim_matches(|ch: char| ch == ';' || ch == '"' || ch == '\'')
                .trim();
            if key.is_empty() || !key.chars().all(is_shell_key_char) {
                None
            } else {
                Some(key.to_string())
            }
        })
        .collect()
}

fn is_shell_key_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn shell_double_quote(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`");
    format!("\"{}\"", escaped)
}

fn redacted_env_value(env_key: &str, value: &str) -> String {
    if env_key.contains("KEY") {
        "***".to_string()
    } else {
        value.to_string()
    }
}

fn copy_dir_recursive(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let entry_path = entry.path();
        let target_path = to.join(entry.file_name());
        if entry_path.is_dir() {
            copy_dir_recursive(&entry_path, &target_path)?;
        } else {
            fs::copy(&entry_path, &target_path)?;
        }
    }
    Ok(())
}

fn list_jsonl_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_jsonl_files(root, &mut files);
    files
}

fn collect_jsonl_files(root: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files);
        } else if path.extension().and_then(OsStr::to_str) == Some("jsonl") {
            files.push(path);
        }
    }
}

fn migrate_official_history(
    home: &Path,
    stamp: &str,
) -> Result<(String, HistoryRewriteReport, Vec<String>), String> {
    let backup_dir = create_history_backup(home, stamp, UNIFY_BACKUP_DIR, "migrate")?;
    let mut commands = vec![format!("mkdir -p {}", display_path(&backup_dir))];
    let files = rewrite_history_files(home, "openai", "codex")?;
    commands.push("rewrite session_meta model_provider: openai -> codex".to_string());
    let sqlite_rows = update_state_provider(&home.join(STATE_FILE), "openai", "codex")?;
    if sqlite_rows.is_some() {
        commands.push(format!(
            "sqlite update {}",
            display_path(&home.join(STATE_FILE))
        ));
    }
    Ok((
        display_path(&backup_dir),
        HistoryRewriteReport { files, sqlite_rows },
        commands,
    ))
}

fn restore_official_history(
    home: &Path,
    stamp: &str,
) -> Result<(Option<String>, HistoryRewriteReport, Vec<String>), String> {
    let backups = migration_backup_dirs(home);
    if backups.is_empty() {
        return Ok((
            None,
            HistoryRewriteReport {
                files: 0,
                sqlite_rows: Some(0),
            },
            vec!["restore skipped: no migration backup".to_string()],
        ));
    }

    let restore_backup = create_history_backup(home, stamp, UNIFY_RESTORE_BACKUP_DIR, "restore")?;
    let mut commands = vec![format!("mkdir -p {}", display_path(&restore_backup))];
    let mut files = 0usize;
    for backup in &backups {
        files += restore_history_files_from_backup(home, backup)?;
    }
    commands.push("rewrite session_meta model_provider by migration backups".to_string());

    let mut total_rows = 0i64;
    let mut sqlite_seen = false;
    for backup in &backups {
        let backup_state = backup.join("state_5.sqlite");
        if !backup_state.exists() {
            continue;
        }
        if let Some(rows) =
            restore_state_provider_from_backup(&home.join(STATE_FILE), &backup_state)?
        {
            sqlite_seen = true;
            total_rows += rows;
        }
    }
    if sqlite_seen {
        commands.push(format!(
            "sqlite restore {}",
            display_path(&home.join(STATE_FILE))
        ));
    }

    Ok((
        Some(display_path(&restore_backup)),
        HistoryRewriteReport {
            files,
            sqlite_rows: if sqlite_seen { Some(total_rows) } else { None },
        },
        commands,
    ))
}

fn create_history_backup(
    home: &Path,
    stamp: &str,
    family: &str,
    kind: &str,
) -> Result<PathBuf, String> {
    let backup_dir = home.join(BACKUP_ROOT).join(family).join(stamp);
    fs::create_dir_all(&backup_dir).map_err(to_string_err)?;
    copy_history_source_if_exists(home, SESSIONS_DIR, &backup_dir.join("sessions"))?;
    copy_history_source_if_exists(
        home,
        ARCHIVED_SESSIONS_DIR,
        &backup_dir.join("archived_sessions"),
    )?;
    let state = home.join(STATE_FILE);
    if state.exists() {
        fs::copy(&state, backup_dir.join("state_5.sqlite")).map_err(to_string_err)?;
    }
    let meta = HistoryBackupMeta {
        kind: kind.to_string(),
        created_at: stamp.to_string(),
        codex_home: display_path(&home.join(".codex")),
    };
    let meta_text = serde_json::to_string_pretty(&meta).map_err(to_string_err)?;
    fs::write(backup_dir.join("meta.json"), format!("{}\n", meta_text)).map_err(to_string_err)?;
    Ok(backup_dir)
}

fn copy_history_source_if_exists(home: &Path, source: &str, target: &Path) -> Result<(), String> {
    let path = home.join(source);
    if path.exists() {
        copy_dir_recursive(&path, target).map_err(to_string_err)?;
    }
    Ok(())
}

fn migration_backup_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = fs::read_dir(home.join(BACKUP_ROOT).join(UNIFY_BACKUP_DIR))
        .ok()
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

fn rewrite_history_files(home: &Path, from: &str, to: &str) -> Result<usize, String> {
    let mut changed = 0usize;
    for root in [home.join(SESSIONS_DIR), home.join(ARCHIVED_SESSIONS_DIR)] {
        for file in list_jsonl_files(&root) {
            if rewrite_session_meta_provider(&file, from, to)? {
                changed += 1;
            }
        }
    }
    Ok(changed)
}

fn restore_history_files_from_backup(home: &Path, backup_dir: &Path) -> Result<usize, String> {
    let mut changed = 0usize;
    for (backup_name, active_root) in [
        ("sessions", home.join(SESSIONS_DIR)),
        ("archived_sessions", home.join(ARCHIVED_SESSIONS_DIR)),
    ] {
        let backup_root = backup_dir.join(backup_name);
        for backup_file in list_jsonl_files(&backup_root) {
            if session_meta_provider(&backup_file)? != Some("openai".to_string()) {
                continue;
            }
            let Ok(relative) = backup_file.strip_prefix(&backup_root) else {
                continue;
            };
            let active_file = active_root.join(relative);
            if active_file.exists()
                && rewrite_session_meta_provider(&active_file, "codex", "openai")?
            {
                changed += 1;
            }
        }
    }
    Ok(changed)
}

fn rewrite_session_meta_provider(path: &Path, from: &str, to: &str) -> Result<bool, String> {
    let text = fs::read_to_string(path).map_err(to_string_err)?;
    let mut lines = text.lines();
    let Some(first) = lines.next() else {
        return Ok(false);
    };
    let mut value: serde_json::Value = match serde_json::from_str(first) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    if value.get("type").and_then(|item| item.as_str()) != Some("session_meta") {
        return Ok(false);
    }
    let provider = value
        .get_mut("payload")
        .and_then(|payload| payload.get_mut("model_provider"));
    let Some(provider) = provider else {
        return Ok(false);
    };
    if provider.as_str() != Some(from) {
        return Ok(false);
    }
    *provider = serde_json::Value::String(to.to_string());
    let mut next = serde_json::to_string(&value).map_err(to_string_err)?;
    for line in lines {
        next.push('\n');
        next.push_str(line);
    }
    if text.ends_with('\n') {
        next.push('\n');
    }
    write_atomic(path, next.as_bytes())
}

fn session_meta_provider(path: &Path) -> Result<Option<String>, String> {
    let text = fs::read_to_string(path).map_err(to_string_err)?;
    let Some(first) = text.lines().next() else {
        return Ok(None);
    };
    let value: serde_json::Value = match serde_json::from_str(first) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if value.get("type").and_then(|item| item.as_str()) != Some("session_meta") {
        return Ok(None);
    }
    Ok(value
        .get("payload")
        .and_then(|payload| payload.get("model_provider"))
        .and_then(|provider| provider.as_str())
        .map(ToOwned::to_owned))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<bool, String> {
    let tmp = path.with_extension("jsonl.swidex-tmp");
    fs::write(&tmp, bytes).map_err(to_string_err)?;
    fs::rename(&tmp, path).map_err(to_string_err)?;
    Ok(true)
}

fn sqlite_provider_counts(path: &Path) -> (Option<i64>, Option<i64>, Option<i64>, String) {
    if !path.exists() {
        return (None, None, None, "state_5.sqlite not found".to_string());
    }
    let sql = "SELECT provider || char(9) || COUNT(*) FROM (SELECT COALESCE(model_provider,'<empty>') AS provider FROM threads) GROUP BY provider;";
    match run_sqlite(path, sql) {
        Ok(output) => {
            let mut openai = 0i64;
            let mut codex = 0i64;
            let mut other = 0i64;
            for line in output.lines() {
                let mut parts = line.split('\t');
                let provider = parts.next().unwrap_or_default();
                let count = parts
                    .next()
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or(0);
                match provider {
                    "openai" => openai += count,
                    "codex" => codex += count,
                    _ => other += count,
                }
            }
            (Some(openai), Some(codex), Some(other), "ok".to_string())
        }
        Err(err) => (None, None, None, err),
    }
}

fn update_state_provider(path: &Path, from: &str, to: &str) -> Result<Option<i64>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let sql = format!(
        "BEGIN; UPDATE threads SET model_provider={} WHERE model_provider={}; SELECT changes(); COMMIT;",
        sql_quote(to),
        sql_quote(from)
    );
    run_sqlite(path, &sql)
        .map(|output| last_sqlite_number(&output))
        .map(Some)
}

fn restore_state_provider_from_backup(active: &Path, backup: &Path) -> Result<Option<i64>, String> {
    if !active.exists() || !backup.exists() {
        return Ok(None);
    }
    let sql = format!(
        "ATTACH DATABASE {} AS backup; BEGIN; UPDATE threads SET model_provider='openai' WHERE model_provider='codex' AND id IN (SELECT id FROM backup.threads WHERE model_provider='openai'); SELECT changes(); COMMIT; DETACH DATABASE backup;",
        sql_quote(&display_path(backup))
    );
    run_sqlite(active, &sql)
        .map(|output| last_sqlite_number(&output))
        .map(Some)
}

fn run_sqlite(path: &Path, sql: &str) -> Result<String, String> {
    let output = Command::new("sqlite3")
        .arg(path)
        .arg(sql)
        .output()
        .map_err(|err| format!("无法运行 sqlite3：{}", err))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("sqlite3 失败：{}", stderr.trim()))
    }
}

fn sql_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn last_sqlite_number(output: &str) -> i64 {
    output
        .lines()
        .rev()
        .find_map(|line| line.trim().parse::<i64>().ok())
        .unwrap_or(0)
}

fn format_optional_count(count: Option<i64>) -> String {
    count
        .map(|count| count.to_string())
        .unwrap_or_else(|| "未找到索引库".to_string())
}

fn env_statuses() -> Vec<EnvStatus> {
    ENV_KEYS
        .iter()
        .map(|key| EnvStatus {
            key: (*key).to_string(),
            set: env::var_os(key).is_some(),
        })
        .collect()
}

fn shell_residues(home: &Path) -> Vec<ShellResidue> {
    SHELL_FILES
        .iter()
        .map(|file| {
            let path = home.join(file);
            let matches = if path.exists() {
                fs::read_to_string(&path)
                    .unwrap_or_default()
                    .lines()
                    .enumerate()
                    .filter_map(|(idx, line)| {
                        if is_shell_residue_line(line) {
                            Some(format!("{}: {}", idx + 1, redact_line(line)))
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };
            ShellResidue {
                file: display_path(&path),
                exists: path.exists(),
                matches,
            }
        })
        .collect()
}

fn launchctl_statuses() -> Vec<LaunchctlStatus> {
    LAUNCHCTL_KEYS
        .iter()
        .map(|key| LaunchctlStatus {
            key: (*key).to_string(),
            set: launchctl_getenv_is_set(key),
        })
        .collect()
}

fn launchctl_getenv_is_set(key: &str) -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    Command::new("launchctl")
        .args(["getenv", key])
        .output()
        .map(|output| {
            output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty()
        })
        .unwrap_or(false)
}

fn run_codex_status() -> String {
    if !command_exists("codex") {
        return "未找到 codex 命令".to_string();
    }
    match Command::new("codex").args(["login", "status"]).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let text = format!("{}{}", stdout, stderr);
            if text.trim().is_empty() {
                format!("codex login status 退出码：{}", output.status)
            } else {
                redact_line(text.trim())
            }
        }
        Err(err) => format!("无法运行 codex login status：{}", err),
    }
}

fn command_exists(command: &str) -> bool {
    env::var_os("PATH")
        .and_then(|paths| {
            env::split_paths(&paths).find_map(|path| {
                let candidate = path.join(command);
                if candidate.exists() {
                    Some(())
                } else {
                    None
                }
            })
        })
        .is_some()
}

fn suspicious_hits(text: &str) -> Vec<String> {
    let mut hits: BTreeMap<String, ()> = BTreeMap::new();
    let lower = text.to_lowercase();
    for token in ["anyrouter", "muyuan.do", "base_url", "openai_base_url"] {
        if lower.contains(token) {
            hits.insert(token.to_string(), ());
        }
    }
    hits.keys().cloned().collect()
}

fn redact_line(line: &str) -> String {
    let mut out = line.to_string();
    for marker in ["sk-", "sess-", "eyJ"] {
        if let Some(idx) = out.find(marker) {
            let visible_end = (idx + marker.len() + 4).min(out.len());
            out.replace_range(visible_end..out.len(), "***");
        }
    }
    out
}

fn mask_key(key: &str) -> String {
    if key.len() <= 10 {
        "***".to_string()
    } else {
        format!("{}...{}", &key[..6], &key[key.len() - 4..])
    }
}

fn report_results(report: &DoctorReport) -> Vec<String> {
    vec![
        format!("doctor: {}", report.conclusion),
        format!(
            "config.toml: model_provider={}, base_url={}, env_key={}",
            report.model_provider.as_deref().unwrap_or("未设置"),
            report.base_url.as_deref().unwrap_or("无"),
            report.env_key.as_deref().unwrap_or("无")
        ),
        format!(
            "history_unify: enabled={}, config_unified={}, openai_threads={}, codex_threads={}",
            report.history_unify.enabled,
            report.history_unify.config_unified,
            report
                .history_unify
                .openai_threads
                .map(|count| count.to_string())
                .unwrap_or_else(|| "未知".to_string()),
            report
                .history_unify
                .codex_threads
                .map(|count| count.to_string())
                .unwrap_or_else(|| "未知".to_string())
        ),
        format!("codex login status: {}", report.codex_status),
    ]
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn to_string_err(err: impl std::fmt::Display) -> String {
    err.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn fetches_models_with_bearer_auth_and_rejects_http_errors() {
        use std::io::Read;
        use std::net::TcpListener;
        for status in ["200 OK", "401 Unauthorized"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let worker = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with("GET /v1/models "));
                assert!(request.contains("Authorization: Bearer test-secret"));
                let body = r#"{"data":[{"id":"gpt-6.2-sol"},{"id":"gpt-6.2-astra"}]}"#;
                write!(
                    stream,
                    "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    body.len(),
                    body
                )
                .unwrap();
            });
            let result = fetch_proxy_models(&format!("http://{}/v1", address), "test-secret");
            worker.join().unwrap();
            if status == "200 OK" {
                assert_eq!(result.unwrap().len(), 2);
            } else {
                assert!(result.is_err());
            }
        }
        assert!(fetch_proxy_models("http://localhost", "bad\nkey").is_err());
    }

    #[test]
    fn catalog_filters_relay_models_and_preserves_metadata() {
        let source = serde_json::json!({"models": [
            {"slug":"gpt-6.2-sol", "visibility":"list", "context_window":123456,
             "supported_reasoning_levels":[{"effort":"high","description":"High"}]},
            {"slug":"internal", "visibility":"hide"}, {"slug":"other-relay"}
        ]});
        let ids = parse_proxy_model_ids(&serde_json::json!({"data":[
            {"id":"gpt-6.2-sol"},{"id":"gpt-6.2-sol"},{"id":"gpt-image-2"},{"id":"internal"}
        ]}))
        .unwrap();
        assert_eq!(ids.len(), 3);
        let catalog = build_model_catalog(&ids, &[source.clone()]).unwrap();
        assert_eq!(catalog["models"].as_array().unwrap().len(), 2);
        assert_eq!(catalog["models"][0], source["models"][0]);
        assert_eq!(catalog["models"][1]["visibility"], "hide");
        assert!(parse_proxy_model_ids(&serde_json::json!({"data":[]})).is_err());
        assert!(parse_proxy_model_ids(&serde_json::json!({"error":"unauthorized"})).is_err());
    }

    #[test]
    fn catalog_round_trip_preserves_user_config_and_repeated_switches() {
        let dir = tempdir().unwrap();
        let home = dir.path();
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::write(
            home.join(".codex/models_cache.json"),
            r#"{"models":[{"slug":"new-model","context_window":42}]}"#,
        )
        .unwrap();
        let mut doc = "model_catalog_json = '/custom/catalog.json'\nmodel = 'my-choice'\n"
            .parse::<DocumentMut>()
            .unwrap();
        let ids = vec!["new-model".to_string()];
        install_proxy_catalog(home, &mut doc, &ids).unwrap();
        install_proxy_catalog(home, &mut doc, &ids).unwrap();
        let saved = doc.to_string();
        let bytes = fs::read(home.join(MODEL_CATALOG)).unwrap();
        assert!(install_proxy_catalog(home, &mut doc, &["unknown".into()]).is_err());
        assert_eq!(doc.to_string(), saved);
        assert_eq!(fs::read(home.join(MODEL_CATALOG)).unwrap(), bytes);
        restore_model_catalog(home, &mut doc).unwrap();
        assert_eq!(
            doc["model_catalog_json"].as_str(),
            Some("/custom/catalog.json")
        );
        assert_eq!(doc["model"].as_str(), Some("my-choice"));
        doc.as_table_mut().remove("model_catalog_json");
        install_proxy_catalog(home, &mut doc, &ids).unwrap();
        restore_model_catalog(home, &mut doc).unwrap();
        assert!(doc.get("model_catalog_json").is_none());
    }

    #[test]
    fn normalizes_proxy_url() {
        assert_eq!(
            normalize_proxy_url("https://example.com").unwrap(),
            "https://example.com/v1"
        );
        assert_eq!(
            normalize_proxy_url("https://example.com/v1/").unwrap(),
            "https://example.com/v1"
        );
    }

    #[test]
    fn removes_proxy_state_from_config() {
        let mut doc = r#"
model = "gpt-5.5"
model_provider = "codex"
openai_base_url = "https://anyrouter.top/v1"

[model_providers.codex]
name = "codex"
base_url = "https://anyrouter.top/v1"
env_key = "CODEX_API_KEY"
"#
        .parse::<DocumentMut>()
        .unwrap();
        set_model_provider(&mut doc, "openai");
        remove_proxy_state(&mut doc);
        let text = doc.to_string();
        assert!(text.contains("model_provider = \"openai\""));
        assert!(!text.contains("anyrouter"));
        assert!(!text.contains("CODEX_API_KEY"));
    }

    #[test]
    fn shell_cleaning_keeps_comments_and_unrelated_lines() {
        assert!(!is_shell_residue_line("# export CODEX_API_KEY=\"sk-test\""));
        assert!(!is_shell_residue_line("export PATH=\"$HOME/bin:$PATH\""));
        assert!(!is_shell_residue_line(
            "export DATABASE_URL=\"postgres://localhost/app\""
        ));
        assert!(!is_shell_residue_line(
            "export MY_OPENAI_BASE_URL=\"https://example.com\""
        ));
        assert!(is_shell_residue_line("export CODEX_API_KEY=\"sk-test\""));
        assert!(is_shell_residue_line("setenv OPENAI_API_KEY sk-test"));
        assert!(is_shell_residue_line("unset OPENAI_BASE_URL CODEX_API_KEY"));
        assert!(is_shell_residue_line(
            "export BASE_URL=\"https://example.com\""
        ));
        assert!(is_shell_residue_line(
            "OPENAI_BASE_URL=https://example.com/v1"
        ));
        assert!(is_shell_residue_line("PROXY_URL=https://anyrouter.top"));
    }

    #[test]
    fn proxy_env_detection_uses_exact_env_names() {
        assert!(is_proxy_env_line("export OPENAI_API_KEY=\"sk-test\""));
        assert!(is_proxy_env_line("CODEX_API_KEY=sk-test"));
        assert!(is_proxy_env_line("unset OPENAI_BASE_URL CODEX_API_KEY"));
        assert!(!is_proxy_env_line("export MY_OPENAI_API_KEY=\"sk-test\""));
        assert!(!is_proxy_env_line(
            "export BASE_URL=\"https://example.com\""
        ));
    }

    #[test]
    fn persisting_proxy_env_writes_all_required_keys() {
        let dir = tempdir().unwrap();
        let zshrc = dir.path().join(".zshrc");
        fs::write(
            &zshrc,
            r#"export PATH="$HOME/bin:$PATH"
export OPENAI_BASE_URL="https://old.example/v1"
export OPENAI_API_KEY="old-openai"
export CODEX_API_KEY="old-codex"
# export CODEX_API_KEY="commented"
"#,
        )
        .unwrap();

        persist_proxy_env(&zshrc, "https://anyrouter.top/v1", "sk-test").unwrap();

        let text = fs::read_to_string(&zshrc).unwrap();
        assert!(text.contains(r#"export PATH="$HOME/bin:$PATH""#));
        assert!(text.contains(r#"# export CODEX_API_KEY="commented""#));
        assert!(text.contains(r#"export OPENAI_BASE_URL="https://anyrouter.top/v1""#));
        assert!(text.contains(r#"export OPENAI_API_KEY="sk-test""#));
        assert!(text.contains(r#"export CODEX_API_KEY="sk-test""#));
        assert!(!text.contains("old-openai"));
        assert!(!text.contains("old-codex"));
        assert!(!text.contains("old.example"));
    }

    #[test]
    fn keychain_account_uses_profile_id_namespace() {
        assert_eq!(
            keychain_account("profile-123").unwrap(),
            "swidex-provider-profile-123"
        );
        assert!(keychain_account("   ").is_err());
    }

    #[test]
    fn backup_history_excludes_auth_json() {
        let dir = tempdir().unwrap();
        let home = dir.path();
        fs::create_dir_all(home.join(".codex/sessions")).unwrap();
        fs::write(home.join(".codex/history.jsonl"), "{}\n").unwrap();
        fs::write(home.join(".codex/auth.json"), "{\"token\":\"secret\"}").unwrap();
        copy_dir_recursive(
            &home.join(".codex/sessions"),
            &home.join("CodexBackups/test/sessions"),
        )
        .unwrap();
        fs::copy(
            home.join(".codex/history.jsonl"),
            home.join("CodexBackups/test/history.jsonl"),
        )
        .unwrap();
        assert!(!home.join("CodexBackups/test/auth.json").exists());
    }
}
