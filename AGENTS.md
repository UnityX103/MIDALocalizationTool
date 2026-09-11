# MIDA 本地化编辑器

## 范围

- 本仓库是独立桌面编辑器，不是 Unity 工程。前端共用 HTML/JavaScript，桌面外壳为 Tauri 2 / Rust，浏览器预览使用 Python。
- 不要求同机存在 mida2025，不读取它的目录、Excel、存档或 Unity Editor。与 Unity 的交付边界只使用用户选择的 ZIP。
- 初始工作区为空，不添加示例任务、默认项目、占位语言或虚假版本。已有真实用户工作区可以自动恢复，不为验证空态而删除用户存档。
- 不把用户 ZIP、视频、翻译输出、工作区、备份、依赖缓存或安装包提交到 Git；禁止提交凭据。

## 修改约定

- 单片段一份 JSON；导入支持数据清单 v2 和媒体清单 v3，导出仅为 v2 数据包，不回传视频。
- 保留来源身份、哈希、版本合并、原子保存及媒体清理约束；同步 Rust 与浏览器端对同一协议的处理。
- 仅按当前请求修改，不自动新建或运行测试。允许执行构建、语法和编译检查；不要对真实工作区做破坏性验证。
- 优先沿用既有脚本，不能通过复制旧 dist 或 target 中的包代替重新构建。
- 只提交当前任务改动，不带入其他任务修改；发布安装包、远端推送或变更仓库访问权限按用户请求执行。

## 命令

- 安装依赖：`npm ci`
- 桌面开发：`npm run desktop`
- 生成前端：`npm run prepare:desktop`
- macOS 通用 App / DMG：`npm run release:mac`
- Rust 编译检查：`cargo check --locked --manifest-path src-tauri/Cargo.toml`
- 浏览器预览：`python3 serve.py`；默认端口 8034，已占用时使用 `--port 8035`，不要终止其他服务。

项目目录与 ZIP 协议见 README.md 和 docs/package-format.md。
