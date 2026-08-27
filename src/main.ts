import "./styles.css";
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
  lastError?: string | null;
};

type SettingsSnapshot = {
  config: AppConfig;
  status: ServerStatus;
  autostart: boolean;
};

type ToastTone = "success" | "error" | "neutral";

const DEFAULT_CONFIG: AppConfig = {
  host: "0.0.0.0",
  port: 8765,
  maxBodyBytes: 1024 * 1024,
  autostart: false,
};

const app = document.querySelector<HTMLDivElement>("#app");

if (!app) {
  throw new Error("App root is missing");
}

app.innerHTML = `
  <div class="app-shell">
    <aside class="sidebar">
      <div class="brand-lockup">
        <div class="brand-mark" aria-hidden="true">
          <span></span><span></span><span></span>
        </div>
        <div>
          <p class="eyebrow">ATTENTIVE KIT</p>
          <p class="brand-name">Desktop</p>
        </div>
      </div>

      <div class="sidebar-rule"></div>

      <nav class="side-nav" aria-label="主导航">
        <p class="nav-label">工作区</p>
        <a class="nav-item is-active" href="#service-settings" aria-current="page">
          <span class="nav-icon nav-icon-sliders" aria-hidden="true">
            <i></i><i></i><i></i>
          </span>
          <span>服务设置</span>
          <span class="nav-active-dot" aria-hidden="true"></span>
        </a>
      </nav>

      <div class="sidebar-footer">
        <div class="os-badge">
          <span class="windows-glyph" aria-hidden="true">⊞</span>
          <span>Windows 原生通知</span>
        </div>
        <p class="version-label">ATTENTIVE DESKTOP <span>v0.1</span></p>
      </div>
    </aside>

    <main class="main-content" id="service-settings">
      <header class="page-header">
        <div>
          <p class="eyebrow accent-eyebrow">LOCAL SERVICE / CONFIGURATION</p>
          <h1>服务设置</h1>
          <p class="page-intro">管理本机通知服务的监听方式，让其他工具可以稳定地把消息送到 Windows 通知中心。</p>
        </div>
        <div class="header-meta">
          <span class="local-chip"><span class="chip-dot"></span>仅运行在本机</span>
          <button class="icon-button" id="refresh-button" type="button" title="刷新状态" aria-label="刷新状态">
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 11a8.1 8.1 0 0 0-14.8-3L3 11m0 0V5m0 6h6M4 13a8.1 8.1 0 0 0 14.8 3L21 13m0 0v6m0-6h-6"/></svg>
          </button>
        </div>
      </header>

      <section class="status-banner" id="status-banner" aria-live="polite">
        <div class="status-orb is-loading" id="status-orb"><span></span></div>
        <div class="status-copy">
          <p class="status-kicker" id="status-kicker">正在读取服务状态</p>
          <p class="status-title" id="status-title">准备中…</p>
          <p class="status-detail" id="status-detail">正在连接本地运行时</p>
        </div>
        <div class="status-endpoint">
          <span class="endpoint-label">LOCAL ENDPOINT</span>
          <code id="endpoint-value">http://127.0.0.1:8765</code>
          <button class="copy-button" id="copy-endpoint" type="button">复制</button>
        </div>
      </section>

      <div class="content-grid">
        <div class="settings-column">
          <section class="settings-card" aria-labelledby="listener-title">
            <div class="card-heading">
              <div>
                <p class="section-number">01 / 02</p>
                <h2 id="listener-title">监听配置</h2>
              </div>
              <span class="heading-icon" aria-hidden="true">
                <svg viewBox="0 0 24 24"><path d="M4 7h10M18 7h2M4 12h3M11 12h9M4 17h10M18 17h2"/><circle cx="16" cy="7" r="2"/><circle cx="9" cy="12" r="2"/><circle cx="16" cy="17" r="2"/></svg>
              </span>
            </div>
            <p class="card-description">这些参数会作用于 notifier HTTP API。修改后需要重启服务才能生效。</p>

            <form id="settings-form" novalidate>
              <div class="form-row two-up">
                <label class="field-group">
                  <span class="field-label">监听地址 <span class="field-key">HOST</span></span>
                  <span class="field-control with-suffix">
                    <input id="host-input" name="host" type="text" autocomplete="off" spellcheck="false" placeholder="0.0.0.0" />
                    <span class="input-suffix">bind</span>
                  </span>
                  <span class="field-help">使用 0.0.0.0 接受局域网请求，127.0.0.1 仅允许本机访问。</span>
                </label>

                <label class="field-group">
                  <span class="field-label">端口 <span class="field-key">PORT</span></span>
                  <span class="field-control with-suffix">
                    <input id="port-input" name="port" type="number" min="0" max="65535" step="1" placeholder="8765" />
                    <span class="input-suffix">tcp</span>
                  </span>
                  <span class="field-help">默认端口 8765；设为 0 可使用随机空闲端口。</span>
                </label>
              </div>

              <label class="field-group full-field">
                <span class="field-label">请求体上限 <span class="field-key">MAX BODY BYTES</span></span>
                <span class="field-control with-suffix">
                  <input id="max-body-input" name="maxBodyBytes" type="number" min="1" step="1024" placeholder="1048576" />
                  <span class="input-suffix">bytes</span>
                </span>
                <span class="field-help">超过上限的请求会返回 413。默认 1 MB，适合标题、正文和少量 metadata。</span>
              </label>

              <div class="form-actions">
                <button class="secondary-button" id="reset-button" type="button">恢复默认值</button>
                <div class="action-cluster">
                  <span class="dirty-indicator" id="dirty-indicator">未修改</span>
                  <button class="primary-button" id="save-button" type="submit">
                    <span>保存并重启服务</span>
                    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h13M13 6l6 6-6 6"/></svg>
                  </button>
                </div>
              </div>
            </form>
          </section>

          <section class="settings-card compact-card" aria-labelledby="startup-title">
            <div class="card-heading">
              <div>
                <p class="section-number">02 / 02</p>
                <h2 id="startup-title">启动行为</h2>
              </div>
              <span class="heading-icon heading-icon-warm" aria-hidden="true">
                <svg viewBox="0 0 24 24"><path d="M12 3v10m0-10 4 4m-4-4-4 4"/><path d="M5 11v7a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-7"/></svg>
              </span>
            </div>
            <div class="setting-toggle-row">
              <div>
                <p class="toggle-title">开机自动启动</p>
                <p class="toggle-description">登录 Windows 后自动运行服务，确保编辑器和脚本无需手动唤醒。</p>
              </div>
              <button class="toggle" id="autostart-toggle" type="button" role="switch" aria-checked="false" aria-label="开机自动启动">
                <span></span>
              </button>
            </div>
          </section>
        </div>

        <aside class="utility-column">
          <section class="utility-card test-card">
            <div class="utility-topline"><span>快速验证</span><span class="live-mark">LIVE</span></div>
            <h2>发一条 Toast 测试</h2>
            <p>确认 Windows Toast 能正常显示，并检查系统通知权限。</p>
            <button class="test-button" id="test-button" type="button">
              <span class="test-button-icon"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 4 9 15"/><path d="m20 4-7 16-4-5-5-4 16-7Z"/></svg></span>
              <span>发送 Toast 测试</span>
              <svg class="button-arrow" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h13M13 6l6 6-6 6"/></svg>
            </button>
          </section>

          <section class="utility-card protocol-card">
            <div class="utility-topline"><span>协议摘要</span><span class="protocol-version">API v1</span></div>
            <div class="protocol-line"><span class="method-badge get">GET</span><code>/health</code></div>
            <div class="protocol-line"><span class="method-badge post">POST</span><code>/api/v1/notifications</code></div>
            <p class="protocol-note">通知支持 <code>http</code>、<code>https</code> 和 <code>vscode</code> action URI。</p>
          </section>

          <div class="quote-note">
            <span class="quote-mark">“</span>
            <p>让通知保持安静、可靠，并且恰到好处。</p>
          </div>
        </aside>
      </div>

      <footer class="page-footer">
        <span>所有配置保存在当前用户的应用配置目录</span>
        <span class="footer-separator">•</span>
        <span id="save-state">等待初始化</span>
      </footer>
    </main>
  </div>
  <div class="toast-region" id="toast-region" aria-live="polite" aria-atomic="true"></div>
`;

const hostInput = getElement<HTMLInputElement>("host-input");
const portInput = getElement<HTMLInputElement>("port-input");
const maxBodyInput = getElement<HTMLInputElement>("max-body-input");
const settingsForm = getElement<HTMLFormElement>("settings-form");
const saveButton = getElement<HTMLButtonElement>("save-button");
const resetButton = getElement<HTMLButtonElement>("reset-button");
const refreshButton = getElement<HTMLButtonElement>("refresh-button");
const testButton = getElement<HTMLButtonElement>("test-button");
const autostartToggle = getElement<HTMLButtonElement>("autostart-toggle");
const dirtyIndicator = getElement<HTMLSpanElement>("dirty-indicator");
const statusOrb = getElement<HTMLDivElement>("status-orb");
const statusKicker = getElement<HTMLParagraphElement>("status-kicker");
const statusTitle = getElement<HTMLParagraphElement>("status-title");
const statusDetail = getElement<HTMLParagraphElement>("status-detail");
const endpointValue = getElement<HTMLElement>("endpoint-value");
const copyEndpointButton = getElement<HTMLButtonElement>("copy-endpoint");
const saveState = getElement<HTMLSpanElement>("save-state");
const toastRegion = getElement<HTMLDivElement>("toast-region");

let currentSnapshot: SettingsSnapshot = {
  config: { ...DEFAULT_CONFIG },
  status: {
    running: false,
    host: DEFAULT_CONFIG.host,
    port: DEFAULT_CONFIG.port,
    endpoint: "http://127.0.0.1:8765",
    lastError: null,
  },
  autostart: false,
};
let savedConfig: AppConfig = { ...DEFAULT_CONFIG };
let toastTimer: number | undefined;
let hasLoadedSettings = false;

function getElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) {
    throw new Error(`Missing element: ${id}`);
  }
  return element as T;
}

function getFormConfig(): AppConfig {
  const portValue = portInput.value.trim();
  return {
    host: hostInput.value.trim(),
    port: portValue === "" ? Number.NaN : Number(portValue),
    maxBodyBytes: Number(maxBodyInput.value),
    autostart: currentSnapshot.autostart,
  };
}

function setFormConfig(config: AppConfig): void {
  hostInput.value = config.host;
  portInput.value = String(config.port);
  maxBodyInput.value = String(config.maxBodyBytes);
  setDirty(false);
}

function setDirty(dirty: boolean): void {
  dirtyIndicator.textContent = dirty ? "有未保存修改" : "已保存";
  dirtyIndicator.classList.toggle("is-dirty", dirty);
}

function setBusy(button: HTMLButtonElement, busy: boolean, label: string): void {
  button.disabled = busy;
  button.classList.toggle("is-busy", busy);
  const text = button.querySelector("span:last-of-type") ?? button.querySelector("span");
  if (text) {
    text.textContent = label;
  }
}

function renderSnapshot(snapshot: SettingsSnapshot): void {
  currentSnapshot = snapshot;
  const status = snapshot.status;
  const actualConfig = snapshot.config;

  if (!hasLoadedSettings || !isDirty()) {
    setFormConfig(actualConfig);
  }
  hasLoadedSettings = true;

  autostartToggle.setAttribute("aria-checked", String(snapshot.autostart));
  autostartToggle.classList.toggle("is-on", snapshot.autostart);
  endpointValue.textContent = status.endpoint || `http://127.0.0.1:${status.port}`;
  statusOrb.classList.remove("is-loading", "is-online", "is-offline");
  statusOrb.classList.add(status.running ? "is-online" : "is-offline");

  if (status.running) {
    statusKicker.textContent = "NOTIFIER SERVICE / ONLINE";
    statusTitle.textContent = "通知服务正在运行";
    statusDetail.textContent = status.lastError
      ? `${status.host}:${status.port} · ${status.lastError}`
      : `${status.host}:${status.port} · 可接收外部通知请求`;
    saveState.textContent = status.lastError ? "服务已运行，但配置有警告" : "服务已就绪";
  } else {
    statusKicker.textContent = "NOTIFIER SERVICE / OFFLINE";
    statusTitle.textContent = "通知服务未运行";
    statusDetail.textContent = status.lastError || "保存配置后，服务会自动尝试启动";
    saveState.textContent = "服务未运行";
  }
}

function isDirty(): boolean {
  const draft = getFormConfig();
  return draft.host !== savedConfig.host
    || draft.port !== savedConfig.port
    || draft.maxBodyBytes !== savedConfig.maxBodyBytes;
}

function showToast(message: string, tone: ToastTone = "neutral"): void {
  window.clearTimeout(toastTimer);
  toastRegion.innerHTML = `<div class="toast-message ${tone}"><span class="toast-message-dot"></span><span>${escapeHtml(message)}</span></div>`;
  window.requestAnimationFrame(() => toastRegion.querySelector(".toast-message")?.classList.add("is-visible"));
  toastTimer = window.setTimeout(() => {
    const toast = toastRegion.querySelector(".toast-message");
    toast?.classList.remove("is-visible");
  }, 3600);
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>'"]/g, (character) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    "'": "&#39;",
    '"': "&quot;",
  })[character] ?? character);
}

function validateConfig(config: AppConfig): string | undefined {
  if (!config.host) {
    return "监听地址不能为空";
  }
  if (!portInput.value.trim()) {
    return "端口不能为空（如需随机端口请输入 0）";
  }
  if (!Number.isInteger(config.port) || config.port < 0 || config.port > 65535) {
    return "端口必须是 0 到 65535 之间的整数";
  }
  if (!Number.isInteger(config.maxBodyBytes) || config.maxBodyBytes < 1) {
    return "请求体上限必须是大于 0 的整数";
  }
  return undefined;
}

async function loadSettings(): Promise<void> {
  statusOrb.classList.add("is-loading");
  try {
    const snapshot = await invoke<SettingsSnapshot>("get_settings");
    savedConfig = { ...snapshot.config };
    renderSnapshot(snapshot);
  } catch (error) {
    savedConfig = { ...DEFAULT_CONFIG };
    if (!hasLoadedSettings) {
      setFormConfig(DEFAULT_CONFIG);
      hasLoadedSettings = true;
    }
    renderSnapshot(currentSnapshot);
    showToast(`预览模式：${error instanceof Error ? error.message : "未连接到 Tauri 运行时"}`, "neutral");
  }
}

async function saveSettings(event: SubmitEvent): Promise<void> {
  event.preventDefault();
  const config = getFormConfig();
  const validationError = validateConfig(config);
  if (validationError) {
    showToast(validationError, "error");
    return;
  }

  setBusy(saveButton, true, "正在重启服务…");
  try {
    const snapshot = await invoke<SettingsSnapshot>("save_settings", { settings: config });
    savedConfig = { ...snapshot.config };
    renderSnapshot(snapshot);
    showToast("配置已保存，通知服务已重启", "success");
  } catch (error) {
    showToast(error instanceof Error ? error.message : String(error), "error");
    await loadSettings();
  } finally {
    setBusy(saveButton, false, "保存并重启服务");
  }
}

async function toggleAutostart(): Promise<void> {
  const nextValue = !currentSnapshot.autostart;
  autostartToggle.disabled = true;
  try {
    const snapshot = await invoke<SettingsSnapshot>("set_autostart", { enabled: nextValue });
    savedConfig = { ...snapshot.config };
    renderSnapshot(snapshot);
    showToast(nextValue ? "已设置为开机自动启动" : "已关闭开机自动启动", "success");
  } catch (error) {
    showToast(error instanceof Error ? error.message : String(error), "error");
  } finally {
    autostartToggle.disabled = false;
  }
}

async function sendTestNotification(): Promise<void> {
  setBusy(testButton, true, "正在提交 Toast…");
  try {
    await invoke("test_notification");
    showToast("Toast 测试已提交到 Windows", "success");
  } catch (error) {
    showToast(error instanceof Error ? error.message : String(error), "error");
  } finally {
    setBusy(testButton, false, "发送 Toast 测试");
  }
}

async function copyEndpoint(): Promise<void> {
  const endpoint = endpointValue.textContent ?? "";
  try {
    await navigator.clipboard.writeText(endpoint);
    showToast("接口地址已复制", "success");
  } catch {
    showToast(endpoint, "neutral");
  }
}

settingsForm.addEventListener("submit", (event) => void saveSettings(event));
for (const input of [hostInput, portInput, maxBodyInput]) {
  input.addEventListener("input", () => setDirty(isDirty()));
}
resetButton.addEventListener("click", () => {
  setFormConfig({ ...DEFAULT_CONFIG, autostart: currentSnapshot.autostart });
  setDirty(true);
  showToast("已恢复默认值，点击保存后生效", "neutral");
});
refreshButton.addEventListener("click", () => void loadSettings());
testButton.addEventListener("click", () => void sendTestNotification());
autostartToggle.addEventListener("click", () => void toggleAutostart());
copyEndpointButton.addEventListener("click", () => void copyEndpoint());
document.addEventListener("keydown", (event) => {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
    event.preventDefault();
    void settingsForm.requestSubmit();
  }
});

void loadSettings();
