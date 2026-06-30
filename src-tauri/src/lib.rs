use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use toml_edit::{value, DocumentMut, Item, Table};

const CONFIG_FILE: &str = ".codex/config.toml";
const AUTH_FILE: &str = ".codex/auth.json";
const HISTORY_FILE: &str = ".codex/history.jsonl";
const SESSIONS_DIR: &str = ".codex/sessions";
const BACKUP_ROOT: &str = "CodexBackups";

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

const SHELL_FILES: [&str; 5] = [
    ".zshrc",
    ".zprofile",
    ".bash_profile",
    ".bashrc",
    ".profile",
];

const SUSPICIOUS_TOKENS: [&str; 13] = [
    "anyrouter",
    "muyuan.do",
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_API_BASE",
    "BASE_URL",
    "API_BASE_URL",
    "CODEX_ACCESS_TOKEN",
    "api-key",
    "apikey",
    "base_url",
    "model_providers.codex",
];

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

struct LaunchctlUnsetReport {
    commands: Vec<String>,
    results: Vec<String>,
    unset_count: usize,
    supported: bool,
}

#[tauri::command]
fn doctor() -> Result<DoctorReport, String> {
    Ok(build_doctor_report())
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
            commands.push(format!("cp {} {}", display_path(&config_path), display_path(&backup)));
            backups.push(display_path(&backup));
        }
    }

    let mut doc = read_config_or_default(&config_path)?;
    set_model_provider(&mut doc, "openai");
    remove_proxy_state(&mut doc);
    write_config(&config_path, &doc)?;
    commands.push(format!("write {}", display_path(&config_path)));
    actions.push("已将 Codex provider 切回 openai，并移除 codex 中转站 provider 配置。".to_string());
    results.push("config.toml: model_provider=openai，codex provider 已移除。".to_string());

    for shell_file in SHELL_FILES {
        let path = home.join(shell_file);
        if !path.exists() {
            continue;
        }
        if let Some(backup) = backup_file(&path, &stamp)? {
            commands.push(format!("cp {} {}", display_path(&path), display_path(&backup)));
            backups.push(display_path(&backup));
        }
        let removed = clean_shell_file(&path)?;
        if removed > 0 {
            commands.push(format!("edit {} # remove {} matching lines", display_path(&path), removed));
            actions.push(format!("已从 {} 移除 {} 行可疑环境配置。", shell_file, removed));
            results.push(format!("{}: 移除 {} 行。", shell_file, removed));
        } else {
            results.push(format!("{}: 未发现需要移除的行。", shell_file));
        }
    }

    for key in ENV_KEYS {
        let was_set = env::var_os(key).is_some();
        env::remove_var(key);
        commands.push(format!("unset {}", key));
        results.push(format!("{}: {}", key, if was_set { "unset" } else { "not set" }));
    }
    actions.push("已清理本应用进程内的相关环境变量。当前已打开终端的父进程环境需要重新打开或手动 unset。".to_string());

    let launchctl_report = unset_launchctl_envs();
    commands.extend(launchctl_report.commands);
    results.extend(launchctl_report.results);
    if launchctl_report.unset_count > 0 {
        actions.push(format!("已清理 {} 个 launchctl 环境变量。", launchctl_report.unset_count));
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
fn use_proxy(url: String, key: String, persist_key: bool) -> Result<OperationResult, String> {
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
            commands.push(format!("cp {} {}", display_path(&config_path), display_path(&backup)));
            backups.push(display_path(&backup));
        }
    }

    let mut doc = read_config_or_default(&config_path)?;
    set_model_provider(&mut doc, "codex");
    upsert_proxy_provider(&mut doc, &normalized_url);
    write_config(&config_path, &doc)?;
    commands.push(format!("write {}", display_path(&config_path)));
    actions.push(format!("已写入中转站 base_url：{}", normalized_url));
    results.push("config.toml: model_provider=codex。".to_string());
    results.push(format!("base_url: {}", normalized_url));
    results.push("env_key: CODEX_API_KEY".to_string());

    if persist_key {
        let zshrc = home.join(".zshrc");
        if zshrc.exists() {
            if let Some(backup) = backup_file(&zshrc, &stamp)? {
                commands.push(format!("cp {} {}", display_path(&zshrc), display_path(&backup)));
                backups.push(display_path(&backup));
            }
        }
        persist_codex_key(&zshrc, trimmed_key)?;
        commands.push(format!("write {} # export CODEX_API_KEY=***", display_path(&zshrc)));
        actions.push("已按你的选择把 CODEX_API_KEY 持久化到 ~/.zshrc。".to_string());
        results.push("CODEX_API_KEY: 已持久化到 ~/.zshrc（日志已隐藏明文）。".to_string());
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
            "关闭并重新打开终端或相关应用后，持久化的 CODEX_API_KEY 才会被新进程继承。".to_string()
        } else {
            format!("临时启动命令：CODEX_API_KEY=\"{}\" codex", mask_key(trimmed_key))
        },
        backups,
        actions,
        commands,
        results,
        report,
    })
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
            commands.push(format!("cp {} {}", display_path(&auth), display_path(&backup)));
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
            use_chatgpt,
            use_proxy,
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

    let suspicious_domains = suspicious_hits(&config_text);
    let env = env_statuses();
    let shell_residues = shell_residues(&home);
    let launchctl = launchctl_statuses();
    let codex_status = run_codex_status();
    let codex_exists = command_exists("codex");

    let env_any = env.iter().any(|item| item.set);
    let shell_any = shell_residues.iter().any(|item| !item.matches.is_empty());
    let launch_any = launchctl.iter().any(|item| item.set);
    let proxyish = model_provider.as_deref() == Some("codex")
        || base_url.is_some()
        || env_key.is_some()
        || !suspicious_domains.is_empty()
        || env_any
        || shell_any
        || launch_any;

    let conclusion = if proxyish {
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
        && SUSPICIOUS_TOKENS
            .iter()
            .any(|token| lower.contains(&token.to_lowercase()))
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
                let is_set =
                    output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty();
                if is_set {
                    commands.push(format!("launchctl unsetenv {}", key));
                    match Command::new("launchctl").args(["unsetenv", key]).status() {
                        Ok(status) if status.success() => {
                            count += 1;
                            results.push(format!("launchctl {}: unset", key));
                        }
                        Ok(status) => results.push(format!("launchctl {}: unset failed ({})", key, status)),
                        Err(err) => results.push(format!("launchctl {}: unset failed ({})", key, err)),
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

fn persist_codex_key(path: &Path, key: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(to_string_err)?;
    }
    let existing = fs::read_to_string(path).unwrap_or_default();
    let cleaned = existing
        .lines()
        .filter(|line| !is_shell_residue_line(line) || !line.contains("CODEX_API_KEY"))
        .collect::<Vec<_>>()
        .join("\n");
    let next = if cleaned.trim().is_empty() {
        format!("export CODEX_API_KEY=\"{}\"\n", key)
    } else {
        format!("{}\nexport CODEX_API_KEY=\"{}\"\n", cleaned, key)
    };
    fs::write(path, next).map_err(to_string_err)
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
        .map(|output| output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty())
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
        assert!(is_shell_residue_line("export CODEX_API_KEY=\"sk-test\""));
        assert!(is_shell_residue_line("OPENAI_BASE_URL=https://example.com/v1"));
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
