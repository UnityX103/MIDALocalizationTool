# 问题反馈与运行日志

左侧“问题反馈”可免登录提交文字及可选截图，固定发布到 [当前 CNB 仓库](https://cnb.cool/nanzhaigame-xpy/MIDALocalizationTool)。运行日志自动准备，默认勾选附送；不需要用户选择日志文件，提交前可以预览或取消勾选。

反馈文字、截图和附送日志会公开。客户端不包含 CNB 写入 Token 或 SSH 私钥；服务器使用现有凭据创建 Issue。开源仓库允许公开读取，不代表匿名 CNB API 可以写 Issue。

```mermaid
flowchart LR
  U[文字与可选截图] --> L[自动准备本次日志]
  L --> Q[本机保存请求与附件]
  Q --> S[固定仓库的反馈服务]
  S --> I[CNB Issue 与附件链接]
  I --> C[既有事件进入 CodexSuper]
```

## 使用与恢复

填写标题、问题描述和可选复现步骤，点击“提交反馈”。最多 10 个附件，单张截图 5 MiB，总附件 25 MiB。自动日志只附送当前运行最近最多 5 MiB，避免上传全部历史。

成功后显示真实 Issue 与上传确认链接，并锁定已完成表单；点击“新建另一条反馈”才能开始下一条。请求和附件先保存到独立的本机反馈数据库；超时、关闭弹窗或刷新后重新打开反馈，核对原请求，不重建不明结果。未提交的接收批次在服务器暂存 24 小时。每来源每小时最多 3 条、每天 10 条，入口有整体限额。

本机反馈存储属于独立数据库 `mida-localization-editor-feedback`，不随翻译空间切换，也不扫描旧编辑器数据。桌面和浏览器预览共用反馈界面，分别通过 Rust 和同源 Python 代理访问固定 HTTPS 服务；不接受用户指定任意代理 URL。

## 日志范围与位置

记录前端启动、工作区恢复/保存、空间切换、导入验证与合并、导出、译文 JSON 操作、更新、媒体请求、反馈请求、耗时和异常堆栈。Rust 命令及 Python HTTP/媒体后端记录各自的执行结果；前后端通信以 `operationId` 关联，同次启动以 `runId` 关联。记录操作名和状态，不记录译文、ZIP 内容、输入字段、反馈正文或请求凭据。

桌面日志在 Tauri 应用数据目录的 `logs`：macOS 通常为 `~/Library/Application Support/com.mida.localization.editor/logs`；浏览器预览在独立的 `com.mida.localization.editor.preview/logs` 下。Windows 预览使用 `LOCALAPPDATA`，Linux 预览使用 `XDG_STATE_HOME` 或 `~/.local/state`。以“偏好 → 日志与诊断”展示的实际目录为准，可以预览和导出，桌面端还可打开目录。

采用 JSONL，时间统一为 Unix 毫秒，前端事件以 `occurredAt` 保留发生时间，`time` 为后端写入时间。单份最多 5 MiB、最多 20 份、总量最多 50 MiB，超出自动删除最早日志。Unix 目录 0700、文件 0600；路径和常见凭据字符串脱敏。磁盘或初始化失败在诊断页面显示；反馈准备日志失败时保留表单，用户可取消日志附送继续提交。

日志用于定位已覆盖的操作；进程被强制终止、系统崩溃或内存不足前尚未刷新到磁盘的事件可能缺失。前端队列溢出会在快照标记 `droppedEvents`。Rust 失败记录调用边界堆栈，panic 记录触发位置；不是所有底层异常的原始栈。视频和翻译工作区不作为日志附件上传。

## 服务维护

服务端通过已有 CodexSuper CNB 反馈服务接入当前仓库的事件链，不要求用户安装 CodexSuper。服务的固定仓库、启停配置、附件回执与发布方式见 [服务端接入文档](https://cnb.cool/nanzhaigame-xpy/CnbFeedbackAutomation/-/blob/main/docs/mida-public-feedback.md)。编辑器安装包仍由本仓库 CD 构建发布。

## 本次接入验收

2026-10-08 通过实际浏览器预览表单发布 [测试 Issue #3](https://cnb.cool/nanzhaigame-xpy/MIDALocalizationTool/-/issues/3)：正文、截图和自动日志均核对成功，两份附件为 `uploaded`，服务器已清理暂存原片。已有 [Issue 通知流水线](https://cnb.cool/nanzhaigame-xpy/MIDALocalizationTool/-/build/logs/cnb-17t-1k4d91con) 成功，服务收到 `issue.open` 事件。

服务端健康版本为 1.2.7，实际发布提交 `0658ec5905b4c4a1d39a53d422e7f28ca1d48d86`，与 [push 发布流水线](https://cnb.cool/nanzhaigame-xpy/CnbFeedbackAutomation/-/build/logs/cnb-jl1-1k4d8vava) 对应。真实提交发现的 CNB 附件标记兼容问题已修复，从原始回执恢复同一请求，没有重复创建 Issue。

前端生成、JavaScript/Python 语法、Rust `cargo check --locked` 和差异空白检查通过。未编写或运行自动测试，未触碰真实翻译工作区；桌面原生交互仅完成编译检查，尚未执行安装包运行验收；客户端 1.0.10 安装包由 GitHub Actions CD 构建，发布结果以正式 Release 为准。事件接收已验证，未实际打开 CodexSuper 面板核对显示。
