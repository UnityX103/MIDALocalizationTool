# CNB 自动检查与发布

## 内部工具不需要平台发布证书

当前流程不要求 Apple Developer ID、Apple 公证或 Windows Authenticode 证书，也不发布到应用商店。push / PR / 手动 API 触发的源码与编译检查完全不使用签名密钥。

现有自动更新的校验密钥与平台证书不同：Mac 和 Windows 共用同一把已有的密钥，不需要每端单独申请或新建。本机已存在的密钥继续保留在仓库外；只有生成可供现有客户端安装的更新包时才使用。若使用另一台发布节点，仍需安全配置该已有密钥，不能靠提交源码把私钥同步过去。Tauri 自动更新不能关闭签名校验，移除它会破坏现有升级能力。

## 触发规则

| 操作 | 自动执行 | 发布权限 |
| --- | --- | --- |
| 分支 push / PR | 安装锁定依赖、JS/Python 语法、版本一致性、更新策略、前端生成、Rust 编译 | 不签名、不发布，不使用 Mac 节点 |
| 推送 `v主.次.补丁` 标签 | 同样的检查 → Mac 通用包 → Windows x64 交叉构建 → 整理与校验 → 草稿上传 → 正式发布 | 专用 Mac 节点 |
| 其他标签 | 跳过发布 | 不发布 |

按项目规则，当前“检查”不运行或创建单元测试，也不读写真实工作区。编译通过不代表已验证 Windows 实机安装、视频播放或完整 UI 回归；发布前应人工验收这些行为。

## 首次接入（必须完成后才能远端发布）

1. 将本次配置及其依赖源码审查后提交到 CNB。当前工作目录尚有之前任务未提交的功能修改；不能只上传 `.cnb.yml`，遗漏 `release-policy.json`、发布脚本、图标或更新模块。不要夹带真实数据、构建缓存、安装包或凭据。
2. 根组织管理员在「组织设置 → 构建节点」接入一台专用 Mac Runner，标签设为 `mida-release-mac`。配置通过 `namespace: group` 选择该节点。若组织没有该功能或没有在线节点，发布任务无法运行；普通 Linux CI 不受影响。不要未经确认把日常工作电脑注册为可执行远端代码的节点。
3. Mac Runner 安装 Node.js 22、Python 3.9+、Git、Rust stable、Xcode Command Line Tools、cargo-xwin、LLVM、NSIS，并把 LLVM 和 NSIS 的 bin 加入 Runner 服务的 PATH。Rust targets 需要 `aarch64-apple-darwin`、`x86_64-apple-darwin`、`x86_64-pc-windows-msvc`。服务进程不一定读取交互终端的 shell 配置。
4. 在该节点仓库外放置原有更新签名私钥，权限 600。默认读取 Runner 用户的 `~/.local/share/mida-localization-release/updater.key`；也可通过 `TAURI_SIGNING_PRIVATE_KEY` 指定绝对文件路径。私钥密码通过受保护的 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 注入，不写入 YAML，不更换现有密钥。不要将密钥内容或环境变量全集输出到日志。
5. 发布使用 CNB 在构建中提供的 `CNB_TOKEN`，需要有创建 Release、上传附件及更新 Release 的权限；遇到 403 先检查构建权限，不将个人 token 写入源码。配置版本标签保护，只允许维护者创建 `v*`；保护 main，审查流水线及发布脚本的修改。专用节点只承接可信发布标签，不能同时用来执行外部 PR。

官方节点说明：[CNB 构建节点](https://docs.cnb.cool/zh/build/build-node.html)。本任务只生成仓库配置，不自动注册宿主机或上传私钥。

## 发布新版本

1. 同步修改 `package.json`、`package-lock.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` 的应用版本。
2. 为新版本填写 `release-policy.json` 的 `optional` 或 `mandatory`；添加 `docs/release-<版本>.md`，仅写给用户看的更新简述。
3. 审查、提交并推送 main，等待 CI 通过；在经过验收的提交上创建对应 `v<版本>` 标签并推送。发布脚本还会检查标签版本、仓库身份、干净检出以及标签提交是否包含在远端 main 中。
4. Mac、Windows 任一构建失败，均不发布。每次使用全新的 Cargo 构建目录，不复用旧安装包。完整构建后创建/复用草稿，上传双平台安装包、更新签名、源码及校验清单，最后才将 Release 设为正式版。
5. 已发布版本拒绝覆盖；失败留下的草稿可在排障后重跑同一标签流水线。发布由仓库级锁串行执行，不取消正在上传的任务。

当前 `1.0.0` 已发布，不能用它再次验证正式发布；应先升版。配置落地不等于远端已启用：必须先完成提交推送、节点接入和密钥配置。

## 本地检查

```sh
npm ci
npm run ci:check
cargo check --locked --manifest-path src-tauri/Cargo.toml
```

`.ci/Dockerfile` 为无签名凭据的 Linux 检查环境；使用 Debian Bookworm、Node 22 和 Rust stable 系列镜像。依赖锁文件保持锁定，基础镜像随该系列更新。

macOS 目前仍为 ad-hoc 签名，Windows 无 Authenticode 签名；自动流水线不等于 Apple 公证或 Windows 签名服务。
