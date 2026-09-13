# MIDA 本地化编辑器 0.3.0

## 下载与安装

- macOS：下载 `MIDA-Localization-0.3.0-macOS-universal.dmg`，支持 Apple Silicon / Intel，最低 macOS 13。将 App 拖入「应用程序」后启动。
- Windows：下载 `MIDA-Localization-0.3.0-Windows-x64-setup.exe`，适用于 Windows 10 / 11 x64，按当前用户安装；缺少 WebView2 时安装程序会联网下载。
- `.app.tar.gz`、`.sig`、`latest.json` 是自动更新使用的文件，不需要手动安装。

## 本次更新

- 启动检查本 CNB 仓库的最新正式 Release；有新版时提示，不强制安装，可在偏好中手动检查。
- 用户确认后先保存并备份，下载签名更新包，校验通过才安装并重启。失败不清空语言空间。
- 语言空间独立存储任务、草稿、备份、布局与视频，同一空间只接受一种目标语言。
- 视频默认隐藏，点击查看定位；再次点击播放。支持右侧 / 下方布局、拖动调整大小和拖至边缘收起。
- 对话包默认收起，中英文逐条显示。可开启「自动下一条」，确认后定位并高亮下一条待处理内容。
- 修复离线图标，补齐 Windows 应用图标。

## 注意事项

- 此版本是首次加入更新机制；此前版本需要手动安装 0.3.0，之后才能在应用内更新。
- macOS 为 ad-hoc 签名，未进行 Apple Developer ID 签名或公证；Windows 未进行 Authenticode 签名。系统可能提示来源或发布者未知。更新包独立使用 Ed25519/minisign 签名校验，这不等同于操作系统发行签名。
- Windows 安装包由 macOS 交叉构建，未在真实 Windows 系统执行安装或启动验证。
- 本次发布未自动提交或推送本地源码修改，Git 标签指向远端现有 main。**准确的构建源码以附件 source.tar.gz 为准**，`build-info.json` 记录本地 HEAD、工作区源码哈希及产物哈希；`SHA256SUMS` 可用于核对下载。
- 不包含任何用户 ZIP、视频、工作区、依赖缓存或私钥。
