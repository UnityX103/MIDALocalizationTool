# MIDA 本地化编辑器 0.3.2

## 下载与安装

- macOS：`MIDA-Localization-0.3.2-macOS-universal.dmg`，支持 Apple Silicon / Intel，最低 macOS 13。
- Windows：`MIDA-Localization-0.3.2-Windows-x64-setup.exe`，适用于 Windows 10 / 11 x64；缺少 WebView2 时安装程序会联网下载。
- 已安装 0.3.0 或 0.3.1 的用户可启动应用检查更新，或在偏好中手动检查；确认安装前自动保存并备份。
- `.app.tar.gz`、`.sig`、`latest.json` 用于签名自动更新，无需手动安装。

## 本次更新

- 展开对话包后，逐句显示只读「旧本地化中文」，仅使用 `localizationSourceAtExport`；有非空白内容就显示，与最新原文相同也显示。缺失内容或纯空白时隐藏标题和整块内容。
- 收起时保留现有缩略对话与待处理提示，不把旧中文加入缩略内容。
- 不再用上次工作区原文或目标译文回填旧中文。
- 保留可选旧英文快照 `englishTranslationAtExport`，支持导入、合并、自动保存、备份与再导出。旧包缺字段时，兼容读取仅在 en 任务使用 `translationAtExport`，其他语言不误当英文；显式空串不回退，不新增英文对比界面。
- 同任务版本且原有哈希及词条来源快照一致时，可以补充缺失的旧英文快照，不覆盖本地译文、草稿或复核状态。
- ZIP / manifest 格式及每片段一个 JSON 的交付结构不变。

## 注意事项

- 本次只更新独立编辑器，未修改 Unity 工程；新增旧英文内容需由来源导出包提供。
- macOS 使用 ad-hoc 签名，未进行 Apple Developer ID 签名或公证；Windows 未进行 Authenticode 签名，系统可能提示来源或发布者未知。更新包沿用原 Ed25519/minisign 签名密钥。
- Windows 安装包在 macOS 交叉构建，未在真实 Windows 系统执行安装或启动验证；未使用真实游戏数据验证导入。
- 沿用现有发布方式：不自动提交本地源码，Release 标签指向远端现有 main。准确构建源码见 source.tar.gz 附件；build-info.json 记录源码及产物哈希，SHA256SUMS 用于核对下载。
- 不包含用户 ZIP、视频、工作区、依赖缓存或私钥。
