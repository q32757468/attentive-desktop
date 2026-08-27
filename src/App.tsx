import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ChangeEvent, FormEvent, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";

type AppConfig = {
  host: string;
  port: number;
  maxBodyBytes: number;
  autostart: boolean;
};

type ServerStatus = {
  running: boolean;
  host: string;
  port: number;
  endpoint: string;
  lanEndpoint?: string | null;
  lastError?: string | null;
};

type SettingsSnapshot = {
  config: AppConfig;
  status: ServerStatus;
  autostart: boolean;
};

type ConfigDraft = {
  host: string;
  port: string;
  maxBodyBytes: string;
};

type ToastTone = "success" | "error" | "neutral";
type BusyAction = "save" | "test" | "autostart" | null;

type ToastMessage = {
  id: number;
  message: string;
  tone: ToastTone;
};

const DEFAULT_CONFIG: AppConfig = {
  host: "0.0.0.0",
  port: 8765,
  maxBodyBytes: 1024 * 1024,
  autostart: false,
};

const DEFAULT_STATUS: ServerStatus = {
  running: false,
  host: DEFAULT_CONFIG.host,
  port: DEFAULT_CONFIG.port,
  endpoint: "http://127.0.0.1:8765",
  lanEndpoint: null,
  lastError: null,
};

const DEFAULT_SNAPSHOT: SettingsSnapshot = {
  config: DEFAULT_CONFIG,
  status: DEFAULT_STATUS,
  autostart: false,
};

function configToDraft(config: AppConfig): ConfigDraft {
  return {
    host: config.host,
    port: String(config.port),
    maxBodyBytes: String(config.maxBodyBytes),
  };
}

function draftToConfig(draft: ConfigDraft, autostart: boolean): AppConfig {
  return {
    host: draft.host.trim(),
    port: draft.port.trim() === "" ? Number.NaN : Number(draft.port),
    maxBodyBytes: Number(draft.maxBodyBytes),
    autostart,
  };
}

function getErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isDraftDirty(draft: ConfigDraft, savedConfig: AppConfig): boolean {
  const config = draftToConfig(draft, savedConfig.autostart);
  return config.host !== savedConfig.host
    || config.port !== savedConfig.port
    || config.maxBodyBytes !== savedConfig.maxBodyBytes;
}

function validateDraft(draft: ConfigDraft): string | undefined {
  if (!draft.host.trim()) {
    return "监听地址不能为空";
  }
  if (!draft.port.trim()) {
    return "端口不能为空（如需随机端口请输入 0）";
  }

  const port = Number(draft.port);
  if (!Number.isInteger(port) || port < 0 || port > 65535) {
    return "端口必须是 0 到 65535 之间的整数";
  }

  const maxBodyBytes = Number(draft.maxBodyBytes);
  if (!Number.isInteger(maxBodyBytes) || maxBodyBytes < 1) {
    return "请求体上限必须是大于 0 的整数";
  }

  return undefined;
}

function App(): ReactNode {
  const [snapshot, setSnapshot] = useState<SettingsSnapshot>(DEFAULT_SNAPSHOT);
  const [savedConfig, setSavedConfig] = useState<AppConfig>(DEFAULT_CONFIG);
  const [draft, setDraft] = useState<ConfigDraft>(configToDraft(DEFAULT_CONFIG));
  const [isLoading, setIsLoading] = useState(true);
  const [busyAction, setBusyAction] = useState<BusyAction>(null);
  const [toast, setToast] = useState<ToastMessage | null>(null);
  const toastTimer = useRef<number | undefined>(undefined);
  const toastSequence = useRef(0);
  const formRef = useRef<HTMLFormElement>(null);
  const hasLoadedSettings = useRef(false);
  const draftRef = useRef(draft);
  const savedConfigRef = useRef(savedConfig);

  const isDirty = useMemo(
    () => isDraftDirty(draft, savedConfig),
    [draft, savedConfig],
  );

  useEffect(() => {
    draftRef.current = draft;
  }, [draft]);

  useEffect(() => {
    savedConfigRef.current = savedConfig;
  }, [savedConfig]);

  const showToast = useCallback((message: string, tone: ToastTone = "neutral") => {
    window.clearTimeout(toastTimer.current);
    toastSequence.current += 1;
    setToast({ id: toastSequence.current, message, tone });
    toastTimer.current = window.setTimeout(() => setToast(null), 3600);
  }, []);

  useEffect(() => () => window.clearTimeout(toastTimer.current), []);

  const loadSettings = useCallback(async () => {
    setIsLoading(true);
    try {
      const nextSnapshot = await invoke<SettingsSnapshot>("get_settings");
      const keepDraft = hasLoadedSettings.current
        && isDraftDirty(draftRef.current, savedConfigRef.current);
      setSnapshot(nextSnapshot);
      setSavedConfig(nextSnapshot.config);
      if (!keepDraft) {
        setDraft(configToDraft(nextSnapshot.config));
      }
      hasLoadedSettings.current = true;
    } catch (error) {
      if (!hasLoadedSettings.current) {
        setSnapshot(DEFAULT_SNAPSHOT);
        setSavedConfig(DEFAULT_CONFIG);
        setDraft(configToDraft(DEFAULT_CONFIG));
        hasLoadedSettings.current = true;
      }
      showToast(`预览模式：${getErrorMessage(error)}`, "neutral");
    } finally {
      setIsLoading(false);
    }
  }, [showToast]);

  useEffect(() => {
    void loadSettings();
  }, [loadSettings]);

  const updateDraft = (key: keyof ConfigDraft, event: ChangeEvent<HTMLInputElement>) => {
    const value = event.target.value;
    setDraft((current) => ({ ...current, [key]: value }));
  };

  const saveSettings = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const validationError = validateDraft(draft);
    if (validationError) {
      showToast(validationError, "error");
      return;
    }

    setBusyAction("save");
    try {
      const config = draftToConfig(draft, snapshot.autostart);
      const nextSnapshot = await invoke<SettingsSnapshot>("save_settings", { settings: config });
      setSnapshot(nextSnapshot);
      setSavedConfig(nextSnapshot.config);
      setDraft(configToDraft(nextSnapshot.config));
      showToast("配置已保存，通知服务已重启", "success");
    } catch (error) {
      showToast(getErrorMessage(error), "error");
      await loadSettings();
    } finally {
      setBusyAction(null);
    }
  };

  const resetSettings = () => {
    setDraft(configToDraft(DEFAULT_CONFIG));
    showToast("已恢复默认值，点击保存后生效", "neutral");
  };

  const toggleAutostart = async () => {
    const nextValue = !snapshot.autostart;
    setBusyAction("autostart");
    try {
      const nextSnapshot = await invoke<SettingsSnapshot>("set_autostart", { enabled: nextValue });
      setSnapshot(nextSnapshot);
      setSavedConfig(nextSnapshot.config);
      showToast(nextValue ? "已设置为开机自动启动" : "已关闭开机自动启动", "success");
    } catch (error) {
      showToast(getErrorMessage(error), "error");
    } finally {
      setBusyAction(null);
    }
  };

  const sendTestNotification = async () => {
    setBusyAction("test");
    try {
      await invoke("test_notification");
      showToast("Toast 测试已提交到 Windows", "success");
    } catch (error) {
      showToast(getErrorMessage(error), "error");
    } finally {
      setBusyAction(null);
    }
  };

  const copyEndpoint = async (endpoint: string, label: string) => {
    try {
      await navigator.clipboard.writeText(endpoint);
      showToast(`${label}已复制`, "success");
    } catch {
      showToast(endpoint, "neutral");
    }
  };

  useEffect(() => {
    const handleSaveShortcut = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        formRef.current?.requestSubmit();
      }
    };
    window.addEventListener("keydown", handleSaveShortcut);
    return () => window.removeEventListener("keydown", handleSaveShortcut);
  }, []);

  const status = snapshot.status;
  const localEndpoint = status.endpoint || `http://127.0.0.1:${status.port}`;
  const statusOrbClass = [
    "status-orb",
    isLoading ? "is-loading" : status.running ? "is-online" : "is-offline",
  ].join(" ");
  const statusDetail = status.running
    ? status.lastError
      ? `${status.host}:${status.port} · ${status.lastError}`
      : `${status.host}:${status.port} · 可接收外部通知请求`
    : status.lastError || "保存配置后，服务会自动尝试启动";
  const saveState = isLoading
    ? "等待初始化"
    : status.running
      ? status.lastError ? "服务已运行，但配置有警告" : "服务已就绪"
      : "服务未运行";

  return (
    <>
      <div className="app-shell">
        <aside className="sidebar">
          <div className="brand-lockup">
            <div className="brand-mark" aria-hidden="true">
              <span></span><span></span><span></span>
            </div>
            <div>
              <p className="eyebrow">ATTENTIVE KIT</p>
              <p className="brand-name">Desktop</p>
            </div>
          </div>

          <div className="sidebar-rule"></div>

          <nav className="side-nav" aria-label="主导航">
            <p className="nav-label">工作区</p>
            <a className="nav-item is-active" href="#service-settings" aria-current="page">
              <span className="nav-icon nav-icon-sliders" aria-hidden="true">
                <i></i><i></i><i></i>
              </span>
              <span>服务设置</span>
              <span className="nav-active-dot" aria-hidden="true"></span>
            </a>
          </nav>

          <div className="sidebar-footer">
            <div className="os-badge">
              <span className="windows-glyph" aria-hidden="true">⊞</span>
              <span>Windows 原生通知</span>
            </div>
            <p className="version-label">ATTENTIVE DESKTOP <span>v0.1</span></p>
          </div>
        </aside>

        <main className="main-content" id="service-settings">
          <header className="page-header">
            <div>
              <p className="eyebrow accent-eyebrow">LOCAL SERVICE / CONFIGURATION</p>
              <h1>服务设置</h1>
              <p className="page-intro">管理本机通知服务的监听方式，让其他工具可以稳定地把消息送到 Windows 通知中心。</p>
            </div>
            <div className="header-meta">
              <span className="local-chip">
                <span className="chip-dot"></span>
                {status.lanEndpoint ? "本机服务 · 支持局域网" : "仅运行在本机"}
              </span>
              <button
                className="icon-button"
                type="button"
                title="刷新状态"
                aria-label="刷新状态"
                onClick={() => void loadSettings()}
                disabled={isLoading}
              >
                <RefreshIcon />
              </button>
            </div>
          </header>

          <section className="status-banner" aria-live="polite">
            <div className={statusOrbClass}><span></span></div>
            <div className="status-copy">
              <p className="status-kicker">
                {isLoading ? "正在读取服务状态" : status.running ? "NOTIFIER SERVICE / ONLINE" : "NOTIFIER SERVICE / OFFLINE"}
              </p>
              <p className="status-title">
                {isLoading ? "准备中…" : status.running ? "通知服务正在运行" : "通知服务未运行"}
              </p>
              <p className="status-detail">{isLoading ? "正在连接本地运行时" : statusDetail}</p>
            </div>
            <div className="status-endpoints">
              <EndpointRow
                label="本机地址"
                eyebrow="LOCAL ENDPOINT"
                endpoint={localEndpoint}
                onCopy={() => void copyEndpoint(localEndpoint, "本机地址")}
              />
              {status.lanEndpoint ? (
                <EndpointRow
                  label="局域网地址"
                  eyebrow="LAN ENDPOINT"
                  endpoint={status.lanEndpoint}
                  onCopy={() => void copyEndpoint(status.lanEndpoint ?? "", "局域网地址")}
                />
              ) : (
                <div className="status-endpoint is-unavailable">
                  <span className="endpoint-label">LAN ENDPOINT</span>
                  <span className="endpoint-unavailable">监听地址未开放局域网访问</span>
                </div>
              )}
            </div>
          </section>

          <div className="content-grid">
            <div className="settings-column">
              <section className="settings-card" aria-labelledby="listener-title">
                <div className="card-heading">
                  <div>
                    <p className="section-number">01 / 02</p>
                    <h2 id="listener-title">监听配置</h2>
                  </div>
                  <span className="heading-icon" aria-hidden="true"><SettingsIcon /></span>
                </div>
                <p className="card-description">这些参数会作用于 notifier HTTP API。修改后需要重启服务才能生效。</p>

                <form ref={formRef} onSubmit={(event) => void saveSettings(event)} noValidate>
                  <div className="form-row two-up">
                    <label className="field-group">
                      <span className="field-label">监听地址 <span className="field-key">HOST</span></span>
                      <span className="field-control with-suffix">
                        <input
                          name="host"
                          type="text"
                          value={draft.host}
                          onChange={(event) => updateDraft("host", event)}
                          autoComplete="off"
                          spellCheck={false}
                          placeholder="0.0.0.0"
                        />
                        <span className="input-suffix">bind</span>
                      </span>
                      <span className="field-help">使用 0.0.0.0 接受局域网请求，127.0.0.1 仅允许本机访问。</span>
                    </label>

                    <label className="field-group">
                      <span className="field-label">端口 <span className="field-key">PORT</span></span>
                      <span className="field-control with-suffix">
                        <input
                          name="port"
                          type="number"
                          min="0"
                          max="65535"
                          step="1"
                          value={draft.port}
                          onChange={(event) => updateDraft("port", event)}
                          placeholder="8765"
                        />
                        <span className="input-suffix">tcp</span>
                      </span>
                      <span className="field-help">默认端口 8765；设为 0 可使用随机空闲端口。</span>
                    </label>
                  </div>

                  <label className="field-group full-field">
                    <span className="field-label">请求体上限 <span className="field-key">MAX BODY BYTES</span></span>
                    <span className="field-control with-suffix">
                      <input
                        name="maxBodyBytes"
                        type="number"
                        min="1"
                        step="1024"
                        value={draft.maxBodyBytes}
                        onChange={(event) => updateDraft("maxBodyBytes", event)}
                        placeholder="1048576"
                      />
                      <span className="input-suffix">bytes</span>
                    </span>
                    <span className="field-help">超过上限的请求会返回 413。默认 1 MB，适合标题、正文和少量 metadata。</span>
                  </label>

                  <div className="form-actions">
                    <button className="secondary-button" type="button" onClick={resetSettings}>恢复默认值</button>
                    <div className="action-cluster">
                      <span className={`dirty-indicator${isDirty ? " is-dirty" : ""}`}>
                        {isDirty ? "有未保存修改" : "已保存"}
                      </span>
                      <button className="primary-button" type="submit" disabled={busyAction !== null}>
                        <span>{busyAction === "save" ? "正在重启服务…" : "保存并重启服务"}</span>
                        <ArrowIcon />
                      </button>
                    </div>
                  </div>
                </form>
              </section>

              <section className="settings-card compact-card" aria-labelledby="startup-title">
                <div className="card-heading">
                  <div>
                    <p className="section-number">02 / 02</p>
                    <h2 id="startup-title">启动行为</h2>
                  </div>
                  <span className="heading-icon heading-icon-warm" aria-hidden="true"><StartupIcon /></span>
                </div>
                <div className="setting-toggle-row">
                  <div>
                    <p className="toggle-title">开机自动启动</p>
                    <p className="toggle-description">登录 Windows 后自动运行服务，确保编辑器和脚本无需手动唤醒。</p>
                  </div>
                  <button
                    className={`toggle${snapshot.autostart ? " is-on" : ""}`}
                    type="button"
                    role="switch"
                    aria-checked={snapshot.autostart}
                    aria-label="开机自动启动"
                    onClick={() => void toggleAutostart()}
                    disabled={busyAction !== null}
                  >
                    <span></span>
                  </button>
                </div>
              </section>
            </div>

            <aside className="utility-column">
              <section className="utility-card test-card">
                <div className="utility-topline"><span>快速验证</span><span className="live-mark">LIVE</span></div>
                <h2>发一条 Toast 测试</h2>
                <p>确认 Windows Toast 能正常显示，并检查系统通知权限。</p>
                <button className="test-button" type="button" onClick={() => void sendTestNotification()} disabled={busyAction !== null}>
                  <span className="test-button-icon"><SendIcon /></span>
                  <span>{busyAction === "test" ? "正在提交 Toast…" : "发送 Toast 测试"}</span>
                  <ArrowIcon className="button-arrow" />
                </button>
              </section>

              <section className="utility-card protocol-card">
                <div className="utility-topline"><span>协议摘要</span><span className="protocol-version">API v1</span></div>
                <div className="protocol-line"><span className="method-badge get">GET</span><code>/health</code></div>
                <div className="protocol-line"><span className="method-badge post">POST</span><code>/api/v1/notifications</code></div>
                <p className="protocol-note">通知支持 <code>http</code>、<code>https</code> 和 <code>vscode</code> action URI。</p>
              </section>

              <div className="quote-note">
                <span className="quote-mark">“</span>
                <p>让通知保持安静、可靠，并且恰到好处。</p>
              </div>
            </aside>
          </div>

          <footer className="page-footer">
            <span>所有配置保存在当前用户的应用配置目录</span>
            <span className="footer-separator">•</span>
            <span id="save-state">{saveState}</span>
          </footer>
        </main>
      </div>

      <div className="toast-region" aria-live="polite" aria-atomic="true">
        {toast ? (
          <div key={toast.id} className={`toast-message ${toast.tone} is-visible`}>
            <span className="toast-message-dot"></span>
            <span>{toast.message}</span>
          </div>
        ) : null}
      </div>
    </>
  );
}

type EndpointRowProps = {
  label: string;
  eyebrow: string;
  endpoint: string;
  onCopy: () => void;
};

function EndpointRow({ label, eyebrow, endpoint, onCopy }: EndpointRowProps): ReactNode {
  return (
    <div className="status-endpoint">
      <span className="endpoint-label">{eyebrow} · {label}</span>
      <code title={endpoint}>{endpoint}</code>
      <button className="copy-button" type="button" onClick={onCopy}>复制</button>
    </div>
  );
}

type IconProps = {
  className?: string;
};

function RefreshIcon(): ReactNode {
  return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 11a8.1 8.1 0 0 0-14.8-3L3 11m0 0V5m0 6h6M4 13a8.1 8.1 0 0 0 14.8 3L21 13m0 0v6m0-6h-6" /></svg>;
}

function SettingsIcon(): ReactNode {
  return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7h10M18 7h2M4 12h3M11 12h9M4 17h10M18 17h2" /><circle cx="16" cy="7" r="2" /><circle cx="9" cy="12" r="2" /><circle cx="16" cy="17" r="2" /></svg>;
}

function StartupIcon(): ReactNode {
  return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3v10m0-10 4 4m-4-4-4 4" /><path d="M5 11v7a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-7" /></svg>;
}

function SendIcon(): ReactNode {
  return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 4 9 15" /><path d="m20 4-7 16-4-5-5-4 16-7Z" /></svg>;
}

function ArrowIcon({ className = "" }: IconProps): ReactNode {
  return <svg className={className} viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h13M13 6l6 6-6 6" /></svg>;
}

export default App;
