# MIDA 本地化编辑器 0.3.3

## 更新类型：可选更新

- 本版按 `optional` 发布，不自动下载安装。
- 从 0.3.3 起，启动发现未跳过的新版本时弹窗提示；可选版提供「稍后提醒」「跳过这个版本」及「保存并安装更新」。跳过选择仅针对该版本，手动检查仍可安装。
- 后续每版在 `release-policy.json` 中选择 `mandatory`（强制）或 `optional`（可选）。强制版没有跳过、稍后或关闭弹窗入口，不能通过 Escape 或点击遮罩关闭，可主动安装或退出应用；下载失败保留提示以便重试。
- 强制更新也必须点击安装后才下载，安装前保存并备份；失败不清空工作区。
- 0.3.0—0.3.2 尚不支持新的强制／跳过弹窗逻辑，需要先通过旧版的「检查应用更新」或手动安装升级到 0.3.3，之后才能执行新策略。旧更新清单没有策略字段时按可选更新兼容。

## 其他界面调整

- 「帮助」改为「关于」，原内容保留在「使用说明」；默认打开「仓库」，显示仓库地址及最近 5 条正式版本历史。
- 压缩任务标题栏与对话包标题行的高度，为对话正文留出更多空间。

## 下载与说明

- macOS：`MIDA-Localization-0.3.3-macOS-universal.dmg`，支持 Apple Silicon / Intel，最低 macOS 13。
- Windows：`MIDA-Localization-0.3.3-Windows-x64-setup.exe`，适用于 Windows 10 / 11 x64；缺少 WebView2 时安装程序会联网下载。
- `.app.tar.gz`、`.sig`、`latest.json` 用于签名更新，无需手动安装。沿用原 Ed25519/minisign 更新签名密钥。
- macOS 使用 ad-hoc 签名，未进行 Apple Developer ID 签名或公证；Windows 未进行 Authenticode 签名，系统可能提示来源或发布者未知。
- Windows 包在 macOS 交叉构建，未在真实 Windows 系统执行安装或启动验证；未对真实工作区执行升级或清空验证。
- 沿用现有发布方式：不自动提交本地源码，Release 标签指向远端现有 main。准确构建源码见 source.tar.gz 附件；build-info.json 记录源码与产物哈希，SHA256SUMS 用于核对下载。
- 不包含用户 ZIP、视频、工作区、依赖缓存或私钥，未修改 Unity 项目。
