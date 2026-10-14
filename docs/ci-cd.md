# GitHub 云构建与 CNB 国内镜像

## 职责与触发

`GitHub main 推送 → GitHub Mac / Windows 并行构建 → 双端成功 → GitHub 正式版 → CNB 每 30 分钟同步 → 客户端默认 CNB 更新`

- 主仓库：<https://github.com/UnityX103/MIDALocalizationTool>，本地 remote 名 `origin`。
- 国内镜像：<https://cnb.cool/nanzhaigame-xpy/MIDALocalizationTool>，本地 remote 名 `cnb`。更新描述先请求 GitHub，仅连接失败、超时或服务不可用时回退 CNB；已知目标版本时只接受该版本的描述。安装资源独立按 CNB → GitHub 顺序下载，两边都失败时提示等待国内节点同步，不降级到旧版本。来源、版本及签名均校验，不跨项目下载。旧客户端仍通过 CNB 升级；“关于”的仓库地址及复制操作统一指向 GitHub，历史优先读 GitHub、失败回退 CNB。
- GitHub `main` push 和手动 workflow_dispatch：GitHub 托管 `macos-14` 构建 Apple Silicon / Intel 通用 DMG 及更新包；`windows-2022` 原生构建 x64 NSIS 安装包。不使用本机安装包，不依赖 CNB 自托管 Mac 节点。
- PR：同样检查并构建，但不读取签名秘密、不发布，安装包保留为 Actions 附件。
- 每次构建产物保留 14 天。双端都成功后，新版本自动建立草稿、上传完整产物、发布正式版。同版本已发布时只保留本次 Actions 附件，不覆盖正式安装包；需要对用户发布变更必须升版。
- CNB `main` 定时任务每 30 分钟运行，也支持 `api_trigger` 手动同步。CNB 不再构建程序，push 不触发同步，避免循环。

## 版本与密钥

同步修改 package.json、package-lock.json、Tauri 配置、Cargo.toml、Cargo.lock 的应用版本；填写 release-policy.json 的 optional / mandatory 和 docs/release-<版本>.md 用户简述，然后推送 main 即可。无需手动创建发布标签。

无需 Apple Developer ID、Apple 公证或 Windows 平台证书。现有自动更新仍使用同一把已有校验密钥，保存在 GitHub 仓库 Actions Secret `TAURI_SIGNING_PRIVATE_KEY`；密码非空时配置 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。密钥不提交到 Git、不放入构建附件、不传给 CNB。PR 不访问这些 Secret。GitHub 发布只使用该次构建的临时 GITHUB_TOKEN。禁止向 GitHub 的源码、提交历史、Actions Secrets、Variables、日志或构建附件写入 CNB 凭据；脚本中的 CNB_TOKEN 仅是 CNB 运行时变量名称，不是密钥值。

## CNB 同步约束

1. 从公开 GitHub main 拉取源码，快进推送到 CNB main；有分叉时明确失败，不强推覆盖任何历史。
2. 读取 GitHub 最新正式 Release；没有正式版或已同步时保留 CNB 当前版本。只支持 v主.次.补丁，忽略预发布，拒绝降级。
3. 从指定 GitHub 仓库下载完整的 Mac、Windows、签名、源码、构建信息和 SHA256 清单，逐个验证大小及哈希；验证构建 SHA 对应发布标签及 main 历史。
4. 根据发布提交读取版本、策略及说明，不使用可能已经升版的 main 配置。只创建草稿，附件全部完成后正式发布；失败可重跑，上一正式版继续服务。
5. 安装包与更新签名保持 GitHub 原始字节；重新生成 CNB latest.json，将下载地址改为 CNB，并重新生成镜像校验清单。
6. 同步使用 CNB 临时 CNB_TOKEN，需要代码推送及 repo-release:rw 权限。不上传个人 token 或更新私钥。若服务返回权限错误，需由仓库管理员检查构建凭证权限。

CNB 初次需同步配置到 main，并执行 `cnb build build-crontab-sync --repo nanzhaigame-xpy/MIDALocalizationTool --branch main` 注册计划。后续流水线同步自身源码，不回写 GitHub。旧 CNB Releases 保留，不覆盖、不删除。

## 验证范围

CI 包含 JS/Python 语法、版本一致性、更新策略、前端生成与 Rust 编译。按项目规则不创建/运行测试、不触碰真实工作区。云构建成功不等于 Windows 实机安装、视频播放和 UI 全流程已验收。

本地仅执行 `npm run ci:check`、`cargo check --locked --manifest-path src-tauri/Cargo.toml`、actionlint 和 CNB 配置校验。安装包统一采用 CD 自动化构建，不在本地运行 `npm run release:mac`、`npm run release:windows` 或 `tauri build`；发布安装包必须来自 GitHub Actions，不使用本地 releases 目录中的旧产物。

## 客户端更新源顺序

1. 从 GitHub 查找最新正式版并读取该版本的 `latest.json`。GitHub 正常返回有效描述后，不再请求 CNB 描述；只有请求不可用时才回退 CNB。格式、版本、平台和更新策略错误不作为网络回退条件。
2. 描述确定后固定版本、说明、mandatory 和签名。若 GitHub 已公布新版本但描述请求失败，CNB 必须匹配同一版本；镜像仍是旧版时报告等待同步。
3. 用户确认安装后，先查找并下载 CNB 上该版本、该平台的安装资源。未同步、缺文件、连接失败或签名验证失败时，改为从 GitHub 下载同一资源；每次都使用原描述中的签名校验，不从下载源换取另一份签名或更新策略。
4. 两个资源渠道均失败时提示“更新失败，请等待国内节点同步后重试”，保留现有工作区并允许稍后重试。下载和校验成功后才安装；本机安装失败直接报告安装错误，不再次下载另一源。

下载切换来源时重置该次下载的字节进度，界面显示当前来源、签名校验和安装阶段。

## 用户数据边界

更新检查与下载只处理版本描述及安装资源，不访问工作区。用户确认安装后，前端先等待已有保存完成，再保存“应用更新前”备份；保存失败时停止安装。下载或签名校验失败不执行安装，并恢复编辑界面供重试。Mac 更新替换应用包，Windows 使用 NSIS `/UPDATE`；应用标识、工作区目录、语言空间标识、存档格式和更新签名密钥保持原值，不执行数据迁移或缓存清空。

1.0.12 将已保存录像映射的兼容校验与新 ZIP 校验分开：已有存档继续保留旧只读备注原值，来源身份、必需哈希、事件和时间仍校验。此次检查不对真实存档执行写入、升级或清理；云构建验收不能替代 Windows 实机升级验收。
