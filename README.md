# Attentive Desktop

一个基于 Tauri 的 Windows notifier 桌面应用。它复刻了 `@attentive-kit/notifier` 的 HTTP 协议：

- `GET /health` 返回 `{ "status": "ok" }`
- `POST /api/v1/notifications` 接收 `title`、`body`、可选 `source`、`action` 和 `metadata`
- `action` 只接受 `open-uri`，并允许 `http`、`https`、`vscode` URI
- 请求体超过配置上限时返回结构化的 `413 INVALID_REQUEST`
- 通知提交到 Windows Toast；点击带 action 的通知时通过 `explorer.exe` 打开 URI
- 支持 `--host`、`--port`、`ATTENTIVE_NOTIFIER_HOST`、`ATTENTIVE_NOTIFIER_PORT`
- 设置页同时显示本机接口地址和可用的局域网接口地址
- 关闭窗口时保留后台服务并隐藏到系统托盘；托盘左键恢复窗口，右键菜单支持恢复和真正退出

## 开发

```sh
npm install
npm run tauri dev
```

设置页使用 React + TypeScript 构建，提供监听地址、端口、请求体上限和开机自动启动。配置保存到当前用户的应用配置目录。
