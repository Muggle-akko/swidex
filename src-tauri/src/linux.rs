//! Linux integration: Secret Service keyring and systemd user environment.
use super::*;
use std::io::Write;
use std::process::Stdio;

fn secret_command(action: &str, account: &str) -> Command {
    let mut command = Command::new("secret-tool");
    command.arg(action);
    if action == "store" {
        command.arg("--label=Swidex API Key");
    }
    command.args(["service", KEYCHAIN_SERVICE, "account", account]);
    command
}

pub(super) fn store_secret(account: &str, secret: &str) -> Result<(), String> {
    let mut child = secret_command("store", account)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "无法启动 secret-tool，请安装 libsecret 并启用系统密钥环。".to_string())?;
    let write_result = child
        .stdin
        .take()
        .ok_or("无法连接密钥环输入")?
        .write_all(secret.as_bytes());
    let output = child.wait_with_output().map_err(to_string_err)?;
    write_result.map_err(|_| "无法向系统密钥环发送密钥".to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err("系统密钥环保存失败，请解锁登录密钥环后重试。".into())
    }
}

pub(super) fn read_secret(account: &str) -> Result<Option<String>, String> {
    let output = secret_command("lookup", account)
        .output()
        .map_err(|_| "无法启动 secret-tool，请安装 libsecret。".to_string())?;
    if output.status.success() {
        Ok(Some(
            String::from_utf8_lossy(&output.stdout)
                .trim_end_matches(['\r', '\n'])
                .to_string(),
        ))
    } else if output.status.code() == Some(1) && output.stderr.is_empty() {
        Ok(None)
    } else {
        Err("系统密钥环读取失败，请检查密钥环服务是否运行并已解锁。".into())
    }
}

pub(super) fn delete_secret(account: &str) -> Result<(), String> {
    let output = secret_command("clear", account)
        .output()
        .map_err(|_| "无法启动 secret-tool，请安装 libsecret。".to_string())?;
    if output.status.success() || (output.status.code() == Some(1) && output.stderr.is_empty()) {
        Ok(())
    } else {
        Err("系统密钥环删除失败，请解锁密钥环后重试。".into())
    }
}

pub(super) fn env_is_set(key: &str) -> bool {
    Command::new("systemctl")
        .args(["--user", "show-environment"])
        .output()
        .map(|out| {
            out.status.success()
                && String::from_utf8_lossy(&out.stdout).lines().any(|line| {
                    line.split_once('=')
                        .map(|(k, v)| k == key && !v.is_empty())
                        .unwrap_or(false)
                })
        })
        .unwrap_or(false)
}

pub(super) fn unset_envs() -> LaunchctlUnsetReport {
    let output = Command::new("systemctl")
        .args(["--user", "unset-environment"])
        .args(LAUNCHCTL_KEYS)
        .output();
    let ok = output.map(|out| out.status.success()).unwrap_or(false);
    LaunchctlUnsetReport {
        commands: vec![format!(
            "systemctl --user unset-environment {}",
            LAUNCHCTL_KEYS.join(" ")
        )],
        results: vec![if ok {
            "桌面用户服务的 API 环境变量已清理；请重新打开 Codex。"
        } else {
            "无法清理 systemd 用户环境，请检查用户服务。"
        }
        .into()],
        unset_count: if ok { LAUNCHCTL_KEYS.len() } else { 0 },
        supported: true,
    }
}

pub(super) fn set_envs(base_url: &str, key: &str) -> LaunchctlSetReport {
    // Import through the child environment, never expose secrets in argv/logs.
    let output = Command::new("systemctl")
        .args(["--user", "import-environment"])
        .args(PROXY_ENV_KEYS)
        .envs(proxy_env_pairs(base_url, key))
        .output();
    let ok = output.map(|out| out.status.success()).unwrap_or(false);
    LaunchctlSetReport {
        commands: vec![
            "systemctl --user import-environment OPENAI_BASE_URL OPENAI_API_KEY CODEX_API_KEY"
                .into(),
        ],
        results: vec![if ok {
            "API 环境变量已同步到桌面用户服务；请重新打开 Codex。"
        } else {
            "systemd 用户环境同步失败，请从新终端启动 Codex。"
        }
        .into()],
        set_count: if ok { PROXY_ENV_KEYS.len() } else { 0 },
        supported: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires an unlocked desktop keyring; uses only a unique synthetic entry"]
    fn keyring_round_trip() {
        let account = format!("swidex-port-test-{}", std::process::id());
        store_secret(&account, "synthetic-linux-test").unwrap();
        let value = read_secret(&account);
        let removal = delete_secret(&account);
        assert_eq!(value.unwrap().as_deref(), Some("synthetic-linux-test"));
        removal.unwrap();
        assert!(read_secret(&account).unwrap().is_none());
    }
    #[test]
    fn keyring_arguments_are_namespaced_and_do_not_include_secret() {
        let command = secret_command("store", "swidex-provider-test");
        let args: Vec<_> = command
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            [
                "store",
                "--label=Swidex API Key",
                "service",
                KEYCHAIN_SERVICE,
                "account",
                "swidex-provider-test"
            ]
        );
    }
}
