import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ChangeEvent, FormEvent, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Card, FormField, SectionHeader, TextArea, TextInput } from "./components/ui";
import {
  Bell,
  Check,
  CheckCircle,
  CircleNotch,
  Code,
  Copy,
  FloppyDisk,
  GearSix,
  Globe,
  Heartbeat,
  House,
  Info,
  PaperPlaneTilt,
  Power,
  WarningCircle,
  WindowsLogo,
  X,
} from "@phosphor-icons/react";

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
  canEnableAutostart: boolean;
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
  canEnableAutostart: false,
};

const DEFAULT_TEST_TITLE = "测试通知";
const DEFAULT_TEST_BODY = "这是一条来自通知服务管理器的测试消息。";

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
  const [testTitle, setTestTitle] = useState(DEFAULT_TEST_TITLE);
  const [testBody, setTestBody] = useState(DEFAULT_TEST_BODY);
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
    if (nextValue && !snapshot.canEnableAutostart) {
      showToast("开发版本不能开启开机自动启动，请安装并使用正式版本设置自启", "error");
      return;
    }
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
    if (!testTitle.trim()) {
      showToast("通知标题不能为空", "error");
      return;
    }
    if (!testBody.trim()) {
      showToast("通知内容不能为空", "error");
      return;
    }

    setBusyAction("test");
    try {
      await invoke("test_notification", {
        title: testTitle,
        body: testBody,
      });
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
  const saveState = isLoading
    ? "等待初始化"
    : status.running
      ? status.lastError ? "服务已运行，但配置有警告" : "服务已就绪"
      : "服务未运行";
  const statusIconClass = isLoading
    ? "bg-slate-100 text-slate-500 ring-slate-200"
    : status.running
      ? "bg-emerald-50 text-emerald-600 ring-emerald-100"
      : "bg-amber-50 text-amber-600 ring-amber-100";
  const toastClass = toast?.tone === "success"
    ? "border-emerald-200 bg-emerald-50 text-emerald-800"
    : toast?.tone === "error"
      ? "border-rose-200 bg-rose-50 text-rose-800"
      : "border-slate-200 bg-white text-slate-700";

  return (
    <div className="flex h-screen min-w-[320px] flex-col overflow-hidden bg-[#f7f8fb] text-slate-900">
      <div className="flex min-h-0 flex-1">
        <aside className="hidden w-[260px] shrink-0 flex-col border-r border-slate-200 bg-white md:flex" aria-label="应用侧栏">
          <nav className="px-2 pt-[18px]" aria-label="主导航">
            <a
              className="relative flex h-[54px] items-center gap-3 rounded-xl bg-[#eaf2ff] px-5 text-[15px] font-medium text-[#1464dc] before:absolute before:inset-y-0 before:left-0 before:w-1 before:rounded-r-full before:bg-[#1464dc]"
              href="#service-status"
              aria-current="page"
            >
              <House size={22} weight="fill" aria-hidden="true" />
              <span>概览</span>
            </a>
          </nav>

          <div className="mt-auto space-y-4 px-5 pb-5">
            <div className="flex items-center gap-2.5 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2.5 text-xs text-slate-500">
              <WindowsLogo size={18} weight="duotone" className="text-[#1464dc]" aria-hidden="true" />
              <span>Windows 原生通知</span>
            </div>
            <p className="px-1 text-[10px] font-medium uppercase tracking-[0.12em] text-slate-300">Attentive Desktop <span className="text-slate-400">v0.1</span></p>
          </div>
        </aside>

        <main className="app-scroll min-w-0 flex-1 overflow-y-auto">
          <div className="w-full px-5 py-4 sm:px-5">
            <div className="grid items-start gap-[18px] xl:grid-cols-[minmax(0,0.935fr)_minmax(0,1.065fr)]">
              <Card id="service-status" className="flex min-h-[294px] flex-col p-5" aria-labelledby="status-title">
                <SectionHeader
                  id="status-title"
                  icon={<Heartbeat size={25} weight="regular" className="text-slate-800" aria-hidden="true" />}
                  title="服务运行状态"
                />

                <div className="mt-4 flex flex-1 items-start gap-7 border-t border-slate-100 pt-3 sm:px-3">
                  <div className="flex min-w-0 items-center gap-7">
                    <div className={`flex h-[74px] w-[74px] shrink-0 items-center justify-center rounded-full ring-8 ${statusIconClass}`}>
                      {isLoading ? (
                        <CircleNotch className="animate-spin" size={29} weight="bold" aria-hidden="true" />
                      ) : status.running ? (
                        <Check size={35} weight="bold" aria-hidden="true" />
                      ) : (
                        <X size={32} weight="bold" aria-hidden="true" />
                      )}
                    </div>
                    <div className="min-w-0">
                      <p className={`text-[21px] font-semibold tracking-[-0.03em] ${
                        isLoading ? "text-slate-600" : status.running ? "text-emerald-600" : "text-slate-700"
                      }`}>
                        {isLoading ? "准备中" : status.running ? "运行中" : "未运行"}
                      </p>
                      <p className="mt-1 text-sm text-slate-500">{isLoading ? "正在连接本地运行时" : status.running ? "服务已稳定运行" : "保存配置后将自动尝试启动"}</p>
                      <p className="mt-2 font-mono text-xs text-slate-400">{status.host}:{status.port}</p>
                    </div>
                  </div>

                  <div className={`ml-auto w-[180px] shrink-0 border-l border-slate-100 pl-7 ${status.lastError ? "text-amber-700" : "text-slate-600"}`}>
                    <p className="text-sm font-medium">最近错误信息</p>
                    <div className="mt-3 flex items-start gap-2">
                      {status.lastError ? (
                        <WarningCircle className="mt-0.5 shrink-0 text-amber-600" size={18} weight="fill" aria-hidden="true" />
                      ) : (
                        <CheckCircle className="mt-0.5 shrink-0 text-emerald-500" size={18} weight="fill" aria-hidden="true" />
                      )}
                      <span className="text-sm leading-5">{status.lastError || "暂无错误"}</span>
                    </div>
                  </div>
                </div>
              </Card>

              <Card id="service-config" className="min-h-[294px] p-5" aria-labelledby="config-title">
                <SectionHeader
                  id="config-title"
                  icon={<GearSix size={25} weight="regular" className="text-slate-800" aria-hidden="true" />}
                  title="服务配置"
                />

                <form ref={formRef} className="mt-4" onSubmit={(event) => void saveSettings(event)} noValidate>
                  <div className="space-y-3">
                    <FormField label="监听地址" className="grid-cols-[143px_minmax(0,1fr)]">
                      <span className="relative block">
                        <TextInput
                          name="host"
                          type="text"
                          value={draft.host}
                          onChange={(event) => updateDraft("host", event)}
                          autoComplete="off"
                          spellCheck={false}
                          placeholder="0.0.0.0"
                        />
                        <span className="pointer-events-none absolute inset-y-0 right-3 flex items-center font-mono text-[10px] uppercase tracking-[0.08em] text-slate-400">bind</span>
                      </span>
                    </FormField>

                    <FormField label="监听端口" className="grid-cols-[143px_minmax(0,1fr)]">
                      <span className="relative block">
                        <TextInput
                          className="pr-12"
                          name="port"
                          type="number"
                          min="0"
                          max="65535"
                          step="1"
                          value={draft.port}
                          onChange={(event) => updateDraft("port", event)}
                          placeholder="8765"
                        />
                        <span className="pointer-events-none absolute inset-y-0 right-3 flex items-center font-mono text-[10px] uppercase tracking-[0.08em] text-slate-400">tcp</span>
                      </span>
                    </FormField>

                    <FormField label="请求体大小上限" className="grid-cols-[143px_minmax(0,1fr)]">
                      <span className="relative block">
                        <TextInput
                          className="pr-16"
                          name="maxBodyBytes"
                          type="number"
                          min="1"
                          step="1024"
                          value={draft.maxBodyBytes}
                          onChange={(event) => updateDraft("maxBodyBytes", event)}
                          placeholder="1048576"
                        />
                        <span className="pointer-events-none absolute inset-y-0 right-3 flex items-center font-mono text-[10px] uppercase tracking-[0.08em] text-slate-400">bytes</span>
                      </span>
                    </FormField>
                  </div>

                  <div className="mt-4 flex gap-3 border-t border-slate-100 pt-3">
                    <Button
                      variant="secondary"
                      className="flex-1"
                      type="button"
                      onClick={resetSettings}
                    >
                      恢复默认值
                    </Button>
                    <Button
                      className="flex-1"
                      type="submit"
                      disabled={busyAction !== null}
                    >
                      {busyAction === "save" ? <CircleNotch className="animate-spin" size={16} aria-hidden="true" /> : <FloppyDisk size={16} aria-hidden="true" />}
                      <span>{busyAction === "save" ? "正在重启…" : "保存并重启服务"}</span>
                    </Button>
                  </div>
                </form>
              </Card>
            </div>

            <div className="mt-4 grid items-start gap-4 xl:grid-cols-[minmax(0,0.625fr)_minmax(0,1fr)]">
              <div className="space-y-4">
                <Card className="min-h-[216px] p-5" aria-labelledby="endpoint-title">
                  <SectionHeader
                    id="endpoint-title"
                    icon={<Globe size={25} weight="regular" className="text-slate-800" aria-hidden="true" />}
                    title="接口地址"
                  />

                  <div className="mt-4 space-y-1">
                    <EndpointRow label="本机地址" endpoint={localEndpoint} onCopy={() => void copyEndpoint(localEndpoint, "本机地址")} />
                    {status.lanEndpoint ? (
                      <EndpointRow label="局域网地址" endpoint={status.lanEndpoint} onCopy={() => void copyEndpoint(status.lanEndpoint ?? "", "局域网地址")} />
                    ) : (
                      <div className="grid grid-cols-[90px_minmax(0,1fr)_auto] items-center gap-3 py-2">
                        <span className="text-sm text-slate-600">局域网地址</span>
                        <span className="truncate text-xs text-slate-400">监听地址未开放局域网访问</span>
                        <span className="rounded-md bg-slate-100 px-2 py-1 text-[10px] text-slate-400">不可用</span>
                      </div>
                    )}
                  </div>

                  <div className="mt-3 flex items-center gap-2 text-xs text-slate-400">
                    <Info className="text-blue-600" size={16} weight="fill" aria-hidden="true" />
                    <span>请确保防火墙已允许对应端口访问</span>
                  </div>
                </Card>

                <Card className="flex min-h-[116px] items-center gap-3 px-5 py-4" aria-labelledby="startup-title">
                  <Power size={25} weight="regular" className="shrink-0 text-slate-800" aria-hidden="true" />
                  <div className="min-w-0">
                    <h2 id="startup-title" className="text-[19px] font-semibold tracking-[-0.02em] text-slate-900">启动设置</h2>
                    <p className="mt-1 text-sm text-slate-500">开机自动启动</p>
                    <p className="mt-0.5 truncate text-xs text-slate-400">
                      {!snapshot.canEnableAutostart && !snapshot.autostart
                        ? "开发版本不可开启，请使用正式安装版"
                        : "登录 Windows 后自动启动服务"}
                    </p>
                  </div>
                  <button
                    className={`relative ml-auto inline-flex h-7 w-12 shrink-0 items-center rounded-full p-1 transition ${snapshot.autostart ? "bg-blue-600" : "bg-slate-300"} disabled:cursor-not-allowed disabled:opacity-60`}
                    type="button"
                    role="switch"
                    aria-checked={snapshot.autostart}
                    aria-label="开机自动启动"
                    onClick={() => void toggleAutostart()}
                    disabled={busyAction !== null || (!snapshot.autostart && !snapshot.canEnableAutostart)}
                  >
                    <span className={`h-5 w-5 rounded-full bg-white shadow-sm transition-transform ${snapshot.autostart ? "translate-x-5" : "translate-x-0"}`} />
                  </button>
                </Card>
              </div>

              <Card className="min-h-[348px] p-5" aria-labelledby="test-title">
                <SectionHeader
                  id="test-title"
                  icon={<PaperPlaneTilt size={25} weight="regular" className="text-slate-800" aria-hidden="true" />}
                  title="测试通知"
                />

                <div className="mt-4 grid gap-5 xl:grid-cols-[minmax(0,1fr)_300px]">
                  <div className="flex min-h-[258px] flex-col gap-4">
                    <FormField label="通知标题" className="grid-cols-[72px_minmax(0,1fr)]">
                      <TextInput
                        type="text"
                        value={testTitle}
                        onChange={(event) => setTestTitle(event.target.value)}
                        aria-label="通知标题"
                      />
                    </FormField>
                    <FormField
                      label="通知内容"
                      className="grid-cols-[72px_minmax(0,1fr)] min-h-[132px]"
                      align="start"
                      labelClassName="pt-2"
                    >
                      <TextArea
                        value={testBody}
                        onChange={(event) => setTestBody(event.target.value)}
                        aria-label="通知内容"
                      />
                    </FormField>
                    <Button
                      className="mt-auto shadow-[0_5px_12px_rgba(23,105,232,0.2)]"
                      type="button"
                      onClick={() => void sendTestNotification()}
                      disabled={busyAction !== null}
                    >
                      {busyAction === "test" ? <CircleNotch className="animate-spin" size={16} aria-hidden="true" /> : <WindowsLogo size={16} weight="duotone" aria-hidden="true" />}
                      <span>{busyAction === "test" ? "正在提交 Toast…" : "发送 Windows Toast 测试通知"}</span>
                    </Button>
                  </div>

                  <div className="relative hidden min-h-[258px] overflow-hidden rounded-[10px] bg-gradient-to-br from-[#a9c8f7] via-[#dceafd] to-[#88afe9] xl:block">
                    <div className="absolute -left-12 -top-12 h-44 w-44 rounded-full border border-white/40 bg-white/10" />
                    <div className="absolute -bottom-14 -right-8 h-52 w-52 rounded-full border border-white/40 bg-white/10" />
                    <div className="absolute left-5 right-5 top-12 rounded-lg bg-white/90 px-4 py-3 shadow-[0_8px_22px_rgba(48,91,156,0.2)] backdrop-blur-sm">
                      <div className="flex items-center gap-2">
                        <div className="flex h-7 w-7 items-center justify-center rounded-full bg-blue-600 text-white">
                          <Bell size={14} weight="fill" aria-hidden="true" />
                        </div>
                        <span className="max-w-[170px] truncate text-xs font-semibold text-slate-700">{testTitle || "测试通知"}</span>
                        <X className="ml-auto text-slate-400" size={13} aria-hidden="true" />
                      </div>
                      <p className="mt-3 whitespace-pre-line break-words text-xs leading-5 text-slate-600">{testBody || "这是一条来自通知服务管理器的测试消息。"}</p>
                      <p className="mt-2 text-right text-[10px] text-slate-400">现在</p>
                    </div>
                  </div>
                </div>
              </Card>
            </div>

            <Card className="mt-[18px] overflow-hidden" aria-labelledby="api-title">
              <SectionHeader
                id="api-title"
                icon={<Code size={25} weight="regular" className="text-slate-800" aria-hidden="true" />}
                title="通知 API"
                className="border-b border-slate-100 px-5 py-3"
              />
              <div className="overflow-x-auto px-3 pb-3 pt-1">
                <table className="w-full min-w-[540px] border-collapse text-left text-sm">
                  <thead className="text-xs text-slate-400">
                    <tr>
                      <th className="border-b border-slate-100 px-3 py-2.5 font-medium">方法</th>
                      <th className="border-b border-slate-100 px-3 py-2.5 font-medium">接口</th>
                      <th className="border-b border-slate-100 px-3 py-2.5 font-medium">说明</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr>
                      <td className="border-b border-slate-100 px-3 py-2"><span className="rounded-md bg-emerald-50 px-2 py-1 font-mono text-[11px] font-semibold text-emerald-700">POST</span></td>
                      <td className="border-b border-slate-100 px-3 py-2 font-mono text-xs text-slate-700">/api/v1/notifications</td>
                      <td className="border-b border-slate-100 px-3 py-2 text-xs text-slate-500">发送通知</td>
                    </tr>
                    <tr>
                      <td className="px-3 py-2"><span className="rounded-md bg-blue-50 px-2 py-1 font-mono text-[11px] font-semibold text-blue-700">GET</span></td>
                      <td className="px-3 py-2 font-mono text-xs text-slate-700">/health</td>
                      <td className="px-3 py-2 text-xs text-slate-500">健康检查</td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </Card>

            <footer className="flex flex-col gap-1 px-1 pb-1 pt-3 text-[11px] text-slate-400 sm:flex-row sm:items-center sm:gap-2">
              <span>所有配置保存在当前用户的应用配置目录</span>
              <span className="hidden text-slate-300 sm:inline">•</span>
              <span className={status.running ? "text-emerald-600" : "text-slate-500"}>{saveState}</span>
            </footer>
          </div>
        </main>
      </div>

      <div className="pointer-events-none fixed bottom-5 right-5 z-50 max-w-[min(360px,calc(100vw-2.5rem))]" aria-live="polite" aria-atomic="true">
        {toast ? (
          <div key={toast.id} className={`pointer-events-auto flex items-start gap-3 rounded-xl border px-4 py-3 text-sm shadow-[0_10px_30px_rgba(27,54,93,0.14)] ${toastClass}`}>
            <span className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${toast.tone === "success" ? "bg-emerald-500" : toast.tone === "error" ? "bg-rose-500" : "bg-slate-400"}`} />
            <span className="leading-5">{toast.message}</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}

type EndpointRowProps = {
  label: string;
  endpoint: string;
  onCopy: () => void;
};

function EndpointRow({ label, endpoint, onCopy }: EndpointRowProps): ReactNode {
  return (
    <div className="grid grid-cols-[90px_minmax(0,1fr)_auto] items-center gap-3 py-2">
      <span className="text-sm text-slate-600">{label}</span>
      <code className="truncate font-mono text-xs text-blue-700" title={endpoint}>{endpoint}</code>
      <button
        className="inline-flex h-8 min-w-[86px] items-center justify-center gap-1.5 rounded-lg border border-slate-200 bg-white px-2.5 text-xs font-medium text-slate-500 transition hover:border-blue-200 hover:bg-blue-50 hover:text-blue-600"
        type="button"
        onClick={onCopy}
      >
        <Copy size={14} aria-hidden="true" />
        <span>复制</span>
      </button>
    </div>
  );
}

export default App;
