import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, ArrowLeft, CheckCircle2, Copy, Eye, EyeOff, Loader2, Plus, Save, X } from "lucide-react";
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
};

type EditableProvider = ProviderProfile & {
  key: string;
  persistKey: boolean;
};

type Screen = "list" | "edit";
type TaskName = "doctor" | "chatgpt" | "proxy" | "history" | "auth" | null;

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
    localStorage.setItem(STORAGE_KEY, JSON.stringify(profiles));
  }, [profiles]);

  const isProxyMode = report?.modelProvider === "codex";
  const riskCount = useMemo(() => {
    if (!report) return 0;
    return [
      report.modelProvider === "codex",
      Boolean(report.baseUrl),
      Boolean(report.envKey),
      report.env.some((item) => item.set),
      report.launchctl.some((item) => item.set),
      report.shellResidues.some((item) => item.matches.length > 0)
    ].filter(Boolean).length;
  }, [report]);

  const providers = useMemo(() => {
    const normalizedBase = report?.baseUrl ? normalizeUrl(report.baseUrl) : "";
    const hasCurrentProxy = normalizedBase
      ? profiles.some((profile) => normalizeUrl(profile.url) === normalizedBase)
      : true;
    const currentProxy =
      normalizedBase && !hasCurrentProxy
        ? [{ id: "current-proxy", name: "当前中转站", note: "来自 config.toml", url: report?.baseUrl ?? "" }]
        : [];
    return [
      { id: "official", name: "OpenAI Official", note: "ChatGPT 登录", url: OFFICIAL_URL },
      ...currentProxy,
      ...profiles
    ];
  }, [profiles, report?.baseUrl]);

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

  async function runOperation(name: Exclude<TaskName, "doctor" | "proxy" | null>, command: string) {
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
    }
  }

  async function enableProxy(profile: ProviderProfile, key: string, persistKey: boolean) {
    setTask("proxy");
    setError(null);
    try {
      if (!isTauriRuntime()) {
        throw new Error("当前是网页预览模式，请在 Swidex App 中执行真实操作。");
      }
      const next = await invoke<OperationResult>("use_proxy", {
        url: profile.url,
        key,
        persistKey
      });
      setResult(next);
      setReport(next.report);
      recordOperation(next);
      setProxyPrompt(null);
      setProxyKey("");
      setProxyPersistKey(false);
    } catch (err) {
      setError(String(err));
    } finally {
      setTask(null);
    }
  }

  async function submitProvider(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    saveProfile(draft);
    setScreen("list");
    setResult(null);
    setError(null);
  }

  function saveProfile(nextDraft: EditableProvider) {
    const id = nextDraft.id || crypto.randomUUID();
    const nextProfile: ProviderProfile = {
      id,
      name: nextDraft.name.trim() || hostLabel(nextDraft.url) || "未命名中转站",
      note: nextDraft.note.trim(),
      url: nextDraft.url.trim()
    };
    setProfiles((current) => [...current.filter((item) => item.id !== id), nextProfile]);
  }

  function openEditor(provider?: Partial<ProviderProfile>) {
    setDraft({
      id: provider?.id === "current-proxy" ? "" : provider?.id ?? "",
      name: provider?.name ?? "",
      note: provider?.note ?? "",
      url: provider?.url ?? "",
      key: "",
      persistKey: false
    });
    setShowDraftKey(false);
    setResult(null);
    setError(null);
    setScreen("edit");
  }

  function removeProfile(id: string) {
    setProfiles((current) => current.filter((profile) => profile.id !== id));
  }

  function askEnableProxy(provider: ProviderProfile) {
    setProxyPrompt(provider);
    setProxyKey("");
    setProxyPersistKey(false);
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
            <p>只保存名称、备注、Base URL。Key 默认只用于本次启用。</p>
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
              placeholder="留空表示只保存配置，不立即启用"
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
            <span>把 CODEX_API_KEY 写入 ~/.zshrc</span>
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
          <h1>Swidex</h1>
          <p className="subtitle">Codex 账号与中转站切换</p>
        </div>
        <div className={`status-chip ${riskCount > 0 ? "warn" : "clean"}`}>
          {statusLabel(report, riskCount)}
        </div>
        <div className="header-actions">
          <button onClick={() => setShowDoctor(true)}>诊断</button>
          <button onClick={() => setShowLog(true)}>日志</button>
          <button onClick={() => setConfirmBackup(true)} disabled={task !== null}>备份历史</button>
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
              ? !isProxyMode
              : isProxyMode && normalizeUrl(provider.url) === normalizeUrl(report?.baseUrl ?? provider.url);
          return (
            <article className={`provider-row ${active ? "active" : ""}`} key={provider.id}>
              <span className="provider-mark">{provider.id === "official" ? "O" : firstGlyph(provider.name)}</span>
              <div className="provider-main">
                <div className="name-line">
                  <h2>{provider.name}</h2>
                  {active && <span>当前使用</span>}
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
                  {task === "proxy" || task === "chatgpt" ? "处理中" : "启用"}
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
      <InfoLine label="model_provider" value={report?.modelProvider ?? "未设置"} hot={report?.modelProvider === "codex"} />
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
          <span>把 CODEX_API_KEY 写入 ~/.zshrc</span>
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

function readProfiles(): ProviderProfile[] {
  for (const key of [STORAGE_KEY, LEGACY_STORAGE_KEY]) {
    try {
      const raw = localStorage.getItem(key);
      if (!raw) continue;
      const parsed = JSON.parse(raw);
      if (!Array.isArray(parsed)) continue;
      return parsed.map((item) => ({
        id: item.id || crypto.randomUUID(),
        name: item.name || "未命名中转站",
        note: item.note || "",
        url: item.url || ""
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
    warnings: []
  };
}

function normalizeUrl(url: string) {
  return url.trim().replace(/\/+$/, "").replace(/^https?:\/\//, "").toLowerCase();
}

function statusLabel(report: DoctorReport | null, riskCount: number) {
  if (!report) return "状态读取中";
  if (riskCount > 0) return "可能走中转站";
  return "官方模式";
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
