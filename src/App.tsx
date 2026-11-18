import { invoke } from "@tauri-apps/api/core";
import { version as APP_VERSION } from "../package.json";
import {
  AlertTriangle,
  ArrowLeft,
  CheckCircle2,
  CircleHelp,
  Copy,
  Eye,
  EyeOff,
  History,
  Loader2,
  Plus,
  Save,
  X
} from "lucide-react";
import { FormEvent, useEffect, useMemo, useState } from "react";

type EnvStatus = { key: string; set: boolean };
type ShellResidue = { file: string; exists: boolean; matches: string[] };
type LaunchctlStatus = { key: string; set: boolean };

type DoctorReport = {
  home: string;
  codexExists: boolean;
  codexStatus: string;
  configExists: boolean;
  modelProvider?: string | null;
  baseUrl?: string | null;
  envKey?: string | null;
  hasCodexProvider: boolean;
  suspiciousDomains: string[];
  env: EnvStatus[];
  shellResidues: ShellResidue[];
  launchctl: LaunchctlStatus[];
  conclusion: string;
  warnings: string[];
  historyUnify: HistoryUnifyStatus;
};

type HistoryUnifyStatus = {
  enabled: boolean;
  configUnified: boolean;
  sessionFiles: number;
  archivedFiles: number;
  openaiThreads?: number | null;
  codexThreads?: number | null;
  otherThreads?: number | null;
  sqliteStatus: string;
  migrationBackups: number;
  restoreBackups: number;
};

type OperationResult = {
  ok: boolean;
  title: string;
  message: string;
  backups: string[];
  actions: string[];
  commands: string[];
  results: string[];
  report: DoctorReport;
};

type ProviderProfile = {
  id: string;
  name: string;
  note: string;
  url: string;
  hasKey?: boolean;
  key?: string;
  persistKey?: boolean;
};

type EditableProvider = ProviderProfile & {
  key: string;
  persistKey: boolean;
};

type Screen = "list" | "edit";
type TaskName = "doctor" | "chatgpt" | "proxy" | "profile" | "history" | "auth" | "historyUnify" | null;

type OperationLogEntry = {
  id: string;
  time: string;
  title: string;
  message: string;
  actions: string[];
  commands: string[];
  results: string[];
  backups: string[];
  resultSummary: string;
};

const STORAGE_KEY = "swidex.providers.v1";
const LEGACY_STORAGE_KEY = "codex-switch.providers.v1";
const OFFICIAL_URL = "https://chatgpt.com/codex";
const unsetSnippet =
  "unset OPENAI_API_KEY CODEX_API_KEY OPENAI_BASE_URL OPENAI_API_BASE BASE_URL API_BASE_URL CODEX_ACCESS_TOKEN";

const emptyDraft: EditableProvider = {
  id: "",
  name: "",
  note: "",
  url: "",
  key: "",
  persistKey: false
};

function App() {
  const [screen, setScreen] = useState<Screen>("list");
  const [report, setReport] = useState<DoctorReport | null>(null);
  const [profiles, setProfiles] = useState<ProviderProfile[]>(() => readProfiles());
  const [draft, setDraft] = useState<EditableProvider>(emptyDraft);
  const [task, setTask] = useState<TaskName>(null);
  const [result, setResult] = useState<OperationResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showDoctor, setShowDoctor] = useState(false);
  const [showLog, setShowLog] = useState(false);
  const [confirmChatgpt, setConfirmChatgpt] = useState(false);
  const [confirmBackup, setConfirmBackup] = useState(false);
  const [confirmAuth, setConfirmAuth] = useState(false);
  const [historyPrompt, setHistoryPrompt] = useState<"enable" | "disable" | null>(null);
  const [migrateExistingHistory, setMigrateExistingHistory] = useState(false);
  const [restoreMigratedHistory, setRestoreMigratedHistory] = useState(true);
  const [proxyPrompt, setProxyPrompt] = useState<ProviderProfile | null>(null);
  const [proxyKey, setProxyKey] = useState("");
  const [proxyPersistKey, setProxyPersistKey] = useState(false);
  const [operationLog, setOperationLog] = useState<OperationLogEntry[]>([]);
  const [showDraftKey, setShowDraftKey] = useState(false);
  const [showProxyKey, setShowProxyKey] = useState(false);

  useEffect(() => {
    refreshDoctor();
  }, []);

  useEffect(() => {
    if (isTauriRuntime() && profiles.some((profile) => profile.key?.trim())) return;
    localStorage.setItem(STORAGE_KEY, JSON.stringify(profiles.map(profileForStorage)));
  }, [profiles]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    const profilesWithPlaintextKeys = profiles.filter((profile) => profile.key?.trim());
    if (profilesWithPlaintextKeys.length === 0) return;

    let cancelled = false;
    async function migrate() {
      for (const profile of profilesWithPlaintextKeys) {
        if (!profile.key?.trim()) continue;
        await saveProviderSecret(profile.id, profile.key);
      }
      if (!cancelled) {
        setProfiles((current) =>
          current.map((profile) =>
            profile.key?.trim()
              ? { ...profile, hasKey: true, key: undefined }
              : profile
          )
        );
      }
    }

    migrate().catch((err) => setError(String(err)));
    return () => {
      cancelled = true;
    };
  }, [profiles]);

  const isProxyMode = report?.modelProvider === "codex" && !report?.historyUnify.configUnified;
  const riskCount = useMemo(() => {
    if (!report) return 0;
    return [
      report.modelProvider === "codex" && !report.historyUnify.configUnified,
      Boolean(report.baseUrl),
      Boolean(report.envKey),
      report.env.some((item) => item.set),
      report.launchctl.some((item) => item.set),
      report.shellResidues.some((item) => item.matches.length > 0)
    ].filter(Boolean).length;
  }, [report]);

  const providers = useMemo(() => {
    const normalizedBase = report?.baseUrl ? normalizeProxyIdentity(report.baseUrl) : "";
    const hasCurrentProxy = normalizedBase
      ? profiles.some((profile) => sameProxyEndpoint(profile.url, report?.baseUrl ?? ""))
      : true;
    const currentProxy =
      isProxyMode && normalizedBase && !hasCurrentProxy
        ? [{ id: "current-proxy", name: "当前中转站", note: "来自 config.toml", url: report?.baseUrl ?? "" }]
        : [];
    return [
      { id: "official", name: "OpenAI Official", note: "ChatGPT 登录", url: OFFICIAL_URL },
      ...currentProxy,
      ...profiles
    ];
  }, [profiles, report?.baseUrl, isProxyMode]);

  async function refreshDoctor() {
    setTask("doctor");
    setError(null);
    try {
      if (!isTauriRuntime()) {
        setReport(mockReport());
        return;
      }
      const next = await invoke<DoctorReport>("doctor");
      setReport(next);
    } catch (err) {
      setError(String(err));
    } finally {
      setTask(null);
    }
  }

  async function runOperation(name: Exclude<TaskName, "doctor" | "proxy" | "profile" | null>, command: string) {
    setTask(name);
    setError(null);
    try {
      if (!isTauriRuntime()) {
        throw new Error("当前是网页预览模式，请在 Swidex App 中执行真实操作。");
      }
      const next = await invoke<OperationResult>(command);
      setResult(next);
      setReport(next.report);
      recordOperation(next);
    } catch (err) {
      setError(String(err));
    } finally {
      setTask(null);
      setConfirmChatgpt(false);
      setConfirmBackup(false);
      setConfirmAuth(false);
      setHistoryPrompt(null);
    }
  }

  async function applyHistoryUnify() {
    if (!historyPrompt) return;
    const enabled = historyPrompt === "enable";
    setTask("historyUnify");
    setError(null);
    try {
      if (!isTauriRuntime()) {
        throw new Error("当前是网页预览模式，请在 Swidex App 中执行真实操作。");
      }
      const next = await invoke<OperationResult>("set_history_unify", {
        enabled,
        migrateExisting: enabled ? migrateExistingHistory : false,
        restoreMigrated: enabled ? false : restoreMigratedHistory
      });
      setResult(next);
      setReport(next.report);
      recordOperation(next);
    } catch (err) {
      setError(String(err));
    } finally {
      setTask(null);
      setHistoryPrompt(null);
      setMigrateExistingHistory(false);
      setRestoreMigratedHistory(true);
    }
  }

  async function enableProxy(profile: ProviderProfile, key: string, persistKey: boolean) {
    setTask("proxy");
    setError(null);
    try {
      if (!isTauriRuntime()) {
        throw new Error("当前是网页预览模式，请在 Swidex App 中执行真实操作。");
      }
      if (profile.id !== "current-proxy" && profile.id !== "official") {
        await saveProviderSecret(profile.id, key);
      }
      const next = await invoke<OperationResult>("use_proxy", {
        url: profile.url,
        key,
        persistKey
      });
      setResult(next);
      setReport(next.report);
      recordOperation(next);
      if (profile.id !== "current-proxy" && profile.id !== "official") {
        setProfiles((current) =>
          current.map((item) =>
            item.id === profile.id
              ? { ...item, hasKey: Boolean(key.trim()), key: undefined, persistKey }
              : item
          )
        );
      }
      setProxyPrompt(null);
      setProxyKey("");
      setShowProxyKey(false);
      setProxyPersistKey(false);
    } catch (err) {
      setError(String(err));
    } finally {
      setTask(null);
    }
  }

  async function submitProvider(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    await saveProfile(draft);
  }

  async function saveProfile(nextDraft: EditableProvider) {
    const id = nextDraft.id || crypto.randomUUID();
    const trimmedKey = nextDraft.key.trim();
    setTask("profile");
    setError(null);
    try {
      if (isTauriRuntime()) {
        if (trimmedKey) {
          await saveProviderSecret(id, trimmedKey);
        } else {
          await deleteProviderSecret(id);
        }
      }

      const nextProfile: ProviderProfile = {
        id,
        name: nextDraft.name.trim() || hostLabel(nextDraft.url) || "未命名中转站",
        note: nextDraft.note.trim(),
        url: nextDraft.url.trim(),
        hasKey: Boolean(trimmedKey),
        key: isTauriRuntime() ? undefined : trimmedKey,
        persistKey: nextDraft.persistKey
      };
      setProfiles((current) => [...current.filter((item) => item.id !== id), nextProfile]);
      setScreen("list");
      setResult(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setTask(null);
    }
  }

  async function openEditor(provider?: Partial<ProviderProfile>) {
    const profileId = provider?.id === "current-proxy" ? "" : provider?.id ?? "";
    const key = profileId ? await loadProviderSecret({ ...provider, id: profileId }) : provider?.key ?? "";
    setDraft({
      id: profileId,
      name: provider?.name ?? "",
      note: provider?.note ?? "",
      url: provider?.url ?? "",
      key,
      persistKey: provider?.persistKey ?? false
    });
    setShowDraftKey(false);
    setResult(null);
    setError(null);
    setScreen("edit");
  }

  async function removeProfile(id: string) {
    if (isTauriRuntime()) {
      await deleteProviderSecret(id).catch((err) => setError(String(err)));
    }
    setProfiles((current) => current.filter((profile) => profile.id !== id));
  }

  async function askEnableProxy(provider: ProviderProfile) {
    const key = await loadProviderSecret(provider);
    if (provider.hasKey && !key.trim()) return;
    setProxyPrompt(provider);
    setProxyKey(key);
    setProxyPersistKey(provider.persistKey ?? false);
    setShowProxyKey(false);
  }

  async function loadProviderSecret(provider: Partial<ProviderProfile>) {
    if (provider.key?.trim()) return provider.key;
    if (!isTauriRuntime() || !provider.id || !provider.hasKey) return "";
    try {
      const secret = await getProviderSecret(provider.id);
      if (secret) return secret;
      setError("未在 macOS 钥匙串找到这个中转站的 Key，请编辑后重新保存。");
      return "";
    } catch (err) {
      setError(String(err));
      return "";
    }
  }

  function recordOperation(next: OperationResult) {
    setOperationLog((current) => [
      {
        id: crypto.randomUUID(),
        time: new Date().toLocaleString(),
        title: next.title,
        message: next.message,
        actions: next.actions,
        commands: next.commands ?? [],
        results: next.results ?? [],
        backups: next.backups,
        resultSummary: next.report.conclusion
      },
      ...current
    ]);
  }

  if (screen === "edit") {
    return (
      <main className="app-frame">
        <header className="page-header">
          <button className="quiet-square" onClick={() => setScreen("list")} aria-label="返回">
            <ArrowLeft />
          </button>
          <div>
            <h1>中转站配置</h1>
            <p>Key 会加密保存到 macOS 钥匙串，启用时自动带入。</p>
          </div>
        </header>

        <form className="settings-card" onSubmit={submitProvider}>
          <div className="form-grid">
            <label>
              <span>名字</span>
              <input
                required
                value={draft.name}
                onChange={(event) => setDraft({ ...draft, name: event.target.value })}
                placeholder="例如：anyrouter"
              />
            </label>
            <label>
              <span>备注</span>
              <input
                value={draft.note}
                onChange={(event) => setDraft({ ...draft, note: event.target.value })}
                placeholder="可选"
              />
            </label>
          </div>

          <label>
            <span>Base URL</span>
            <input
              required
              value={draft.url}
              onChange={(event) => setDraft({ ...draft, url: event.target.value })}
              placeholder="https://example.com/v1"
            />
          </label>

          <label>
            <span>API Key</span>
            <SecretInput
              value={draft.key}
              visible={showDraftKey}
              placeholder="留空表示不保存或删除已保存 Key"
              onChange={(value) => setDraft({ ...draft, key: value })}
              onToggleVisible={() => setShowDraftKey((current) => !current)}
            />
          </label>

          <label className="checkline">
            <input
              type="checkbox"
              checked={draft.persistKey}
              onChange={(event) => setDraft({ ...draft, persistKey: event.target.checked })}
            />
            <span>持久化到 shell/launchctl</span>
            <HelpTip text="把 API 地址和 Key 写进 ~/.zshrc，并同步给 launchctl。这样新打开的终端或从 Dock 启动的 Codex 也能读到；软件内保存的 Key 另存在 macOS 钥匙串里。" />
          </label>
        </form>

        {error && <InlineMessage tone="bad" text={error} />}

        <footer className="bottom-bar">
          <button className="primary-button" type="submit" form="" onClick={(event) => {
            event.preventDefault();
            document.querySelector<HTMLFormElement>(".settings-card")?.requestSubmit();
          }} disabled={task !== null}>
            {task === "proxy" ? <Loader2 className="spin" /> : <Save />}
            保存
          </button>
        </footer>
      </main>
    );
  }

  return (
    <main className="app-frame">
      <header className="main-header">
        <div>
          <div className="brand-line">
            <h1>Swidex</h1>
            <span>v{APP_VERSION}</span>
          </div>
          <p className="subtitle">Codex 账号与中转站切换</p>
        </div>
        <div className={`status-chip ${riskCount > 0 ? "warn" : "clean"}`}>
          {statusLabel(report, riskCount)}
        </div>
        <div className="header-actions">
          <button onClick={() => setShowDoctor(true)}>诊断</button>
          <button onClick={() => setShowLog(true)}>日志</button>
          <button onClick={() => setConfirmBackup(true)} disabled={task !== null}>备份历史</button>
          <button
            className={`sync-menu-button ${report?.historyUnify.enabled ? "enabled" : ""}`}
            onClick={() => {
              const nextMode = report?.historyUnify.enabled ? "disable" : "enable";
              setHistoryPrompt(nextMode);
              setMigrateExistingHistory(nextMode === "enable");
              setRestoreMigratedHistory(true);
            }}
            disabled={task !== null}
          >
            <History />
            同步历史
            <span>{report?.historyUnify.enabled ? "开" : "关"}</span>
          </button>
          <button onClick={() => setConfirmAuth(true)}>重置登录</button>
          <button className="primary-action" onClick={() => openEditor()}>
            <Plus />
            添加中转站
          </button>
        </div>
      </header>

      <section className="provider-list">
        {providers.map((provider) => {
          const active =
            provider.id === "official"
              ? Boolean(report) && !isProxyMode
              : isProxyMode && Boolean(report?.baseUrl) && sameProxyEndpoint(provider.url, report?.baseUrl ?? "");
          return (
            <article className={`provider-row ${active ? "active" : ""}`} key={provider.id}>
              <span className="provider-mark">{provider.id === "official" ? "O" : firstGlyph(provider.name)}</span>
              <div className="provider-main">
                <div className="name-line">
                  <h2>{provider.name}</h2>
                  {active && <span>当前使用</span>}
                  {provider.id !== "official" && provider.hasKey && <b>Key 已加密保存</b>}
                  {provider.note && <b>{provider.note}</b>}
                </div>
                <p>{provider.url}</p>
              </div>
              <div className="row-actions">
                <button
                  className="row-primary"
                  onClick={() => (provider.id === "official" ? setConfirmChatgpt(true) : askEnableProxy(provider))}
                  disabled={task !== null}
                >
                  {task === "proxy" || task === "chatgpt" ? "处理中" : active ? "重启" : "启用"}
                </button>
                {provider.id !== "official" && <button onClick={() => openEditor(provider)}>编辑</button>}
                {provider.id !== "official" && provider.id !== "current-proxy" && (
                  <button className="danger-text" onClick={() => removeProfile(provider.id)}>删除</button>
                )}
              </div>
            </article>
          );
        })}
      </section>

      {(result || error) && (
        <section className={`toast-line ${error ? "bad" : "good"}`}>
          {error ? <AlertTriangle /> : <CheckCircle2 />}
          <div>
            <strong>{error ? "操作失败" : result?.title}</strong>
            <span>{error ?? result?.message}</span>
          </div>
          <button onClick={() => {
            setResult(null);
            setError(null);
          }}><X /></button>
        </section>
      )}

      {showDoctor && (
        <aside className="doctor-panel">
          <div className="doctor-head">
            <div>
              <span>Doctor</span>
              <strong>{report?.conclusion ?? "读取中"}</strong>
            </div>
            <button onClick={() => setShowDoctor(false)}><X /></button>
          </div>
          <DoctorLines report={report} riskCount={riskCount} />
        </aside>
      )}

      {showLog && (
        <aside className="doctor-panel log-panel">
          <div className="doctor-head">
            <div>
              <span>Log</span>
              <strong>操作日志</strong>
            </div>
            <button onClick={() => setShowLog(false)}><X /></button>
          </div>
          {operationLog.length === 0 ? (
            <p className="empty-log">还没有执行过切换、备份或重置操作。</p>
          ) : (
            <div className="log-list">
              {operationLog.map((entry) => (
                <details key={entry.id} open>
                  <summary>
                    <span>{entry.title}</span>
                    <b>{entry.time}</b>
                  </summary>
                  <p>{entry.message}</p>
                  <LogBlock title="结论" items={[entry.resultSummary]} />
                  <LogBlock title="返回结果" items={entry.results} />
                  <LogBlock title="动作" items={entry.actions} />
                  <LogBlock title="命令 / 文件操作" items={entry.commands} />
                  <LogBlock title="备份" items={entry.backups} />
                </details>
              ))}
            </div>
          )}
        </aside>
      )}

      {confirmChatgpt && (
        <ConfirmDialog
          title="切换到 ChatGPT 官方账号"
          body={`会把 Codex 配置切回官方账号模式，并清理中转站 provider、shell 启动文件和 launchctl 环境变量。不会删除 auth.json，也不会让你退出登录。当前终端若仍有残留，可执行：${unsetSnippet}`}
          confirmText="启用官方"
          loading={task === "chatgpt"}
          onCancel={() => setConfirmChatgpt(false)}
          onConfirm={() => runOperation("chatgpt", "use_chatgpt")}
        />
      )}

      {confirmBackup && (
        <ConfirmDialog
          title="备份历史记录"
          body="会复制 ~/.codex/sessions/ 和 ~/.codex/history.jsonl 到 ~/CodexBackups/。不会包含 auth.json，因此不会打包登录凭据。"
          confirmText="开始备份"
          loading={task === "history"}
          onCancel={() => setConfirmBackup(false)}
          onConfirm={() => runOperation("history", "backup_history")}
        />
      )}

      {confirmAuth && (
        <ConfirmDialog
          title="重置登录缓存"
          body="会先备份 auth.json，然后删除当前登录缓存。后果是 Codex 下次需要重新登录；不会删除 sessions 和 history。它不同于“启用官方”：启用官方只清理中转站配置，不退出你的 ChatGPT 账号。"
          confirmText="重置登录"
          danger
          loading={task === "auth"}
          onCancel={() => setConfirmAuth(false)}
          onConfirm={() => runOperation("auth", "reset_auth")}
        />
      )}

      {historyPrompt && (
        <HistoryUnifyDialog
          mode={historyPrompt}
          status={report?.historyUnify}
          migrateExisting={migrateExistingHistory}
          restoreMigrated={restoreMigratedHistory}
          loading={task === "historyUnify"}
          onMigrateExistingChange={setMigrateExistingHistory}
          onRestoreMigratedChange={setRestoreMigratedHistory}
          onCancel={() => setHistoryPrompt(null)}
          onConfirm={applyHistoryUnify}
        />
      )}

      {proxyPrompt && (
        <ProxyConfirmDialog
          provider={proxyPrompt}
          apiKey={proxyKey}
          visible={showProxyKey}
          persistKey={proxyPersistKey}
          loading={task === "proxy"}
          onApiKeyChange={setProxyKey}
          onToggleVisible={() => setShowProxyKey((current) => !current)}
          onPersistKeyChange={setProxyPersistKey}
          onCancel={() => {
            setProxyPrompt(null);
            setProxyKey("");
            setShowProxyKey(false);
            setProxyPersistKey(false);
          }}
          onConfirm={() => enableProxy(proxyPrompt, proxyKey, proxyPersistKey)}
        />
      )}
    </main>
  );
}

function HistoryUnifyDialog({
  mode,
  status,
  migrateExisting,
  restoreMigrated,
  loading,
  onMigrateExistingChange,
  onRestoreMigratedChange,
  onCancel,
  onConfirm
}: {
  mode: "enable" | "disable";
  status?: HistoryUnifyStatus;
  migrateExisting: boolean;
  restoreMigrated: boolean;
  loading: boolean;
  onMigrateExistingChange: (value: boolean) => void;
  onRestoreMigratedChange: (value: boolean) => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const enable = mode === "enable";
  const hasMigrationBackups = (status?.migrationBackups ?? 0) > 0;
  return (
    <div className="overlay" role="dialog" aria-modal="true">
      <section className="confirm-box history-dialog">
        <h2>{enable ? "开启同步对话历史" : "关闭同步对话历史"}</h2>
        <div className="dialog-note">
          <strong>{enable ? "开启说明" : "关闭说明"}</strong>
          <span>
            {enable
              ? "开启后，官方账号与中转或定制供应商会共用同一个历史 / 续聊列表。默认会把现有官方会话也同步进来；认证和当前请求通道都不变。"
              : "关闭后，官方账号与中转或定制供应商会恢复各自的历史列表。能按备份分清来源的官方会话会还原；分不清的新会话会留在原共享历史里。"}
            这个开关不区分“官方切中转”或“中转切官方”；它不会改变当前请求通道。
          </span>
        </div>
        {enable ? (
          <label className="checkline dialog-checkline">
            <input
              type="checkbox"
              checked={migrateExisting}
              onChange={(event) => onMigrateExistingChange(event.target.checked)}
            />
            <span>同步现有官方会话历史</span>
            <HelpTip text="默认勾选。会先备份 sessions、archived_sessions 和 state_5.sqlite，再把可识别的官方老会话迁入共享历史列表；取消勾选则只同步之后新建的会话。" />
          </label>
        ) : (
          <label className={`checkline dialog-checkline ${hasMigrationBackups ? "" : "muted-check"}`}>
            <input
              type="checkbox"
              checked={restoreMigrated && hasMigrationBackups}
              disabled={!hasMigrationBackups}
              onChange={(event) => onRestoreMigratedChange(event.target.checked)}
            />
            <span>按备份还原当初迁入的官方会话</span>
            <HelpTip text="只还原迁移备份里能证明原本属于官方、且当前仍在共享列表里的会话；开启期间新建的会话无法可靠判断来源，会留在中转或定制供应商历史里，避免误伤。" />
          </label>
        )}
        <div className="history-dialog-grid">
          <span>会话文件：{(status?.sessionFiles ?? 0) + (status?.archivedFiles ?? 0)}</span>
          <span>官方历史：{formatThreadCount(status?.openaiThreads)}</span>
          <span>共享历史：{formatThreadCount(status?.codexThreads)}</span>
          <span>迁移备份：{status?.migrationBackups ?? 0}</span>
        </div>
        <div>
          <button className="plain-button" onClick={onCancel}>取消</button>
          <button className="primary-button" onClick={onConfirm} disabled={loading}>
            {loading ? <Loader2 className="spin" /> : null}
            {enable ? "我已了解，开启" : "关闭同步"}
          </button>
        </div>
      </section>
    </div>
  );
}

function InlineMessage({ tone, text }: { tone: "good" | "bad"; text: string }) {
  return (
    <div className={`inline-message ${tone}`}>
      {tone === "good" ? <CheckCircle2 /> : <AlertTriangle />}
      <span>{text}</span>
    </div>
  );
}

function DoctorLines({ report, riskCount }: { report: DoctorReport | null; riskCount: number }) {
  const shellCount = report?.shellResidues.reduce((sum, file) => sum + file.matches.length, 0) ?? 0;
  const envCount = report?.env.filter((item) => item.set).length ?? 0;
  const launchCount = report?.launchctl.filter((item) => item.set).length ?? 0;

  return (
    <div className="doctor-lines">
      <InfoLine label="风险项" value={String(riskCount)} hot={riskCount > 0} />
      <InfoLine label="统一历史" value={report?.historyUnify.enabled ? "开启" : "关闭"} hot={Boolean(report?.historyUnify.enabled && !report?.historyUnify.configUnified)} />
      <InfoLine label="model_provider" value={report?.modelProvider ?? "未设置"} hot={report?.modelProvider === "codex" && !report?.historyUnify.configUnified} />
      <InfoLine label="base_url" value={report?.baseUrl ? compactUrl(report.baseUrl) : "无"} hot={Boolean(report?.baseUrl)} />
      <InfoLine label="env_key" value={report?.envKey ?? "无"} hot={Boolean(report?.envKey)} />
      <InfoLine label="进程环境变量" value={`${envCount} set`} hot={envCount > 0} />
      <InfoLine label="launchctl" value={`${launchCount} set`} hot={launchCount > 0} />
      <InfoLine label="shell 残留" value={`${shellCount} 行`} hot={shellCount > 0} />
      <pre>{report?.codexStatus ?? "codex login status 读取中..."}</pre>
    </div>
  );
}

function InfoLine({ label, value, hot }: { label: string; value: string; hot?: boolean }) {
  return (
    <div className={`info-line ${hot ? "hot" : ""}`}>
      <span>{label}</span>
      <b>{value}</b>
    </div>
  );
}

function LogBlock({ title, items }: { title: string; items: string[] }) {
  if (!items.length) return null;
  return (
    <div className="log-block">
      <span>{title}</span>
      {items.map((item) => <code key={item}>{item}</code>)}
    </div>
  );
}

function HelpTip({ text }: { text: string }) {
  return (
    <span className="help-tip">
      <button type="button" aria-label="说明">
        <CircleHelp />
      </button>
      <span role="tooltip">{text}</span>
    </span>
  );
}

function SecretInput({
  value,
  visible,
  placeholder,
  onChange,
  onToggleVisible
}: {
  value: string;
  visible: boolean;
  placeholder: string;
  onChange: (value: string) => void;
  onToggleVisible: () => void;
}) {
  async function copyValue() {
    if (!value) return;
    await navigator.clipboard?.writeText(value);
  }

  return (
    <div className="secret-field">
      <input
        type={visible ? "text" : "password"}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
      />
      <button type="button" onClick={onToggleVisible} title={visible ? "隐藏 Key" : "查看 Key"}>
        {visible ? <EyeOff /> : <Eye />}
      </button>
      <button type="button" onClick={copyValue} title="复制 Key" disabled={!value}>
        <Copy />
      </button>
    </div>
  );
}

function ConfirmDialog({
  title,
  body,
  confirmText,
  danger,
  loading,
  onCancel,
  onConfirm
}: {
  title: string;
  body: string;
  confirmText: string;
  danger?: boolean;
  loading: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div className="overlay" role="dialog" aria-modal="true">
      <section className={`confirm-box ${danger ? "danger" : ""}`}>
        <h2>{title}</h2>
        <p>{body}</p>
        <div>
          <button className="plain-button" onClick={onCancel}>取消</button>
          <button className="primary-button" onClick={onConfirm} disabled={loading}>
            {loading ? <Loader2 className="spin" /> : null}
            {confirmText}
          </button>
        </div>
      </section>
    </div>
  );
}

function ProxyConfirmDialog({
  provider,
  apiKey,
  visible,
  persistKey,
  loading,
  onApiKeyChange,
  onToggleVisible,
  onPersistKeyChange,
  onCancel,
  onConfirm
}: {
  provider: ProviderProfile;
  apiKey: string;
  visible: boolean;
  persistKey: boolean;
  loading: boolean;
  onApiKeyChange: (value: string) => void;
  onToggleVisible: () => void;
  onPersistKeyChange: (value: boolean) => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div className="overlay" role="dialog" aria-modal="true">
      <section className="confirm-box">
        <h2>启用中转站</h2>
        <p>
          将把 `model_provider` 切到 `codex`，写入 {provider.url}，并使用 `CODEX_API_KEY`。
          这会改变 Codex 后续请求通道，请确认后再继续。
        </p>
        <label className="dialog-field">
          <span>API Key</span>
          <SecretInput
            value={apiKey}
            visible={visible}
            placeholder="sk-..."
            onChange={onApiKeyChange}
            onToggleVisible={onToggleVisible}
          />
        </label>
        <label className="checkline dialog-checkline">
          <input
            type="checkbox"
            checked={persistKey}
            onChange={(event) => onPersistKeyChange(event.target.checked)}
          />
          <span>持久化到 shell/launchctl</span>
          <HelpTip text="把 API 地址和 Key 写进 ~/.zshrc，并同步给 launchctl。这样新打开的终端或从 Dock 启动的 Codex 也能读到；软件内保存的 Key 另存在 macOS 钥匙串里。" />
        </label>
        <div>
          <button className="plain-button" onClick={onCancel}>取消</button>
          <button className="primary-button" onClick={onConfirm} disabled={loading || !apiKey.trim()}>
            {loading ? <Loader2 className="spin" /> : null}
            确认启用
          </button>
        </div>
      </section>
    </div>
  );
}

async function saveProviderSecret(profileId: string, key: string) {
  await invoke("save_provider_secret", { profileId, key });
}

async function getProviderSecret(profileId: string) {
  return await invoke<string | null>("get_provider_secret", { profileId });
}

async function deleteProviderSecret(profileId: string) {
  await invoke("delete_provider_secret", { profileId });
}

function profileForStorage(profile: ProviderProfile) {
  return {
    id: profile.id,
    name: profile.name,
    note: profile.note,
    url: profile.url,
    hasKey: Boolean(profile.hasKey || profile.key?.trim()),
    persistKey: Boolean(profile.persistKey)
  };
}

function readProfiles(): ProviderProfile[] {
  for (const key of [STORAGE_KEY, LEGACY_STORAGE_KEY]) {
    try {
      const raw = localStorage.getItem(key);
      if (!raw) continue;
      const parsed = JSON.parse(raw);
      if (!Array.isArray(parsed)) continue;
      return parsed.map((item) => ({
        key: typeof item.key === "string" ? item.key : "",
        id: item.id || crypto.randomUUID(),
        name: item.name || "未命名中转站",
        note: item.note || "",
        url: item.url || "",
        hasKey: Boolean(item.hasKey || item.key),
        persistKey: Boolean(item.persistKey)
      }));
    } catch {
      continue;
    }
  }
  return [];
}

function isTauriRuntime() {
  return "__TAURI_INTERNALS__" in window;
}

function mockReport(): DoctorReport {
  return {
    home: "/Users/awu",
    codexExists: true,
    codexStatus: "网页预览模式：真实状态会在 Swidex App 中显示。",
    configExists: true,
    modelProvider: "openai",
    baseUrl: null,
    envKey: null,
    hasCodexProvider: false,
    suspiciousDomains: [],
    env: [],
    shellResidues: [],
    launchctl: [],
    conclusion: "当前看起来是 ChatGPT 账号模式",
    warnings: [],
    historyUnify: {
      enabled: false,
      configUnified: false,
      sessionFiles: 0,
      archivedFiles: 0,
      openaiThreads: null,
      codexThreads: null,
      otherThreads: null,
      sqliteStatus: "preview",
      migrationBackups: 0,
      restoreBackups: 0
    }
  };
}

function sameProxyEndpoint(left: string, right: string) {
  return normalizeProxyIdentity(left) === normalizeProxyIdentity(right);
}

function normalizeProxyIdentity(url: string) {
  const trimmed = url.trim();
  if (!trimmed) return "";
  try {
    const parsed = new URL(trimmed);
    return `${parsed.hostname.toLowerCase()}${parsed.port ? `:${parsed.port}` : ""}${normalizeProxyPath(parsed.pathname)}`;
  } catch {
    return normalizeProxyPath(trimmed.replace(/^https?:\/\//i, "").replace(/[?#].*$/, "").toLowerCase());
  }
}

function normalizeProxyPath(value: string) {
  const withoutTrailingSlash = value.replace(/\/+$/, "");
  const withoutVersionSuffix = withoutTrailingSlash.replace(/\/v1$/i, "");
  return withoutVersionSuffix;
}

function statusLabel(report: DoctorReport | null, riskCount: number) {
  if (!report) return "状态读取中";
  if (report.historyUnify.configUnified) return "官方模式 · 统一历史";
  if (report.modelProvider === "codex") return "中转站模式";
  if (riskCount > 0) return "可能走中转站";
  return "官方模式";
}

function formatThreadCount(value?: number | null) {
  return typeof value === "number" ? String(value) : "未知";
}

function compactUrl(url: string) {
  return url.replace(/^https?:\/\//, "");
}

function hostLabel(url: string) {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return url.trim();
  }
}

function firstGlyph(name: string) {
  const trimmed = name.trim();
  if (!trimmed) return "+";
  const first = Array.from(trimmed)[0] ?? "+";
  return /[a-z]/i.test(first) ? first.toUpperCase() : first;
}

export default App;
