# MIDA 本地化编辑器

独立的 macOS / Windows 桌面本地化编辑项目，使用 Tauri 2、HTML/JavaScript 和 Rust。当前源码版本 **0.2.2**；应用通过 ZIP 与 Unity 交换数据，不依赖 Unity 工程目录、Excel 或运行中的 Unity Editor。

远端仓库：`https://cnb.cool/nanzhaigame-xpy/MIDALocalizationTool`。

## 从空白开始

- 首次启动没有项目、片段、语言任务或示例对话；导入用户选取的 ZIP 后才显示内容。
- 可以点击「导入 ZIP」或拖入 ZIP。没有任务时不能保存或导出空交付包。
- 已经导入过的真实工作区继续支持自动恢复；旧版的内置示例工作区不再展示，也不为清空界面删除真实用户数据。
- 本仓库不包含任何业务 ZIP、视频、翻译输出、自动保存工作区或备份。

## 开发启动

安装 Node.js、Rust 及系统桌面构建工具后，在此仓库根目录运行：

```sh
npm ci
npm run desktop
```

Tauri 调用 `scripts/prepare-desktop.cjs`，将共用界面复制到 `dist/` 后启动原生窗口。开发和构建无需把仓库放在某个固定位置，也不读取原 Unity 仓库。

### 浏览器预览

```sh
python3 serve.py
# 如果 8034 已占用，使用另一个端口
python3 serve.py --port 8035
```

Windows 可用 `py -3 serve.py`。默认只监听 `127.0.0.1:8034`，使用 Python 3.9 或以上，不依赖第三方 Python 包。浏览器导出的 ZIP 默认写入本仓库的 `LocalOutput/`，也可用 `--output <目录>` 指定位置。该目录不提交。

浏览器 IndexedDB 与原生 App 工作区相互独立；不同来源地址的浏览器工作区也不会自动共享。需要迁移内容时使用 ZIP，不拷贝浏览器缓存。

## macOS 构建

需要 Xcode 构建工具和 `aarch64-apple-darwin`、`x86_64-apple-darwin` 两个 Rust target：

```sh
npm run release:mac
```

生成通用 Apple Silicon / Intel App 与 DMG，最低 macOS 13。产物位于 `src-tauri/target/universal-apple-darwin/release/bundle/`。当前配置使用 ad-hoc 本地签名，不包含 Apple Developer ID 或 Apple 公证，不能将它当作已通过公开发行审核的安装包。

Windows 桌面代码保留，但本次拆分未生成或验证 Windows 安装包。此前旧目录发布的 0.2.1 安装包不包含此次空白启动改动，需要从本仓库重新构建后安装。

## 编辑与交付

1. Unity 导出本地化 ZIP，包含清单和每片段独立的 JSON；可附带预览视频及对话包时间映射。
2. 编辑器导入 ZIP，按片段和目标语言呈现任务。默认预览中文与译文，点击编辑后修改，点「确定」退出编辑。
3. 同一项目导入新版本时匹配已有词条；中文基线相同则复用已有译文，变化则保留译文并标记待处理。
4. 支持对话包收起、中文摘要与待处理提示；仅当视频可用且含该包节点时显示「查看视频」。播放器位于窗口最底部，保存 / 导出在播放器上方。
5. 选择交付任务后导出数据 ZIP，由 Unity 负责回收与覆盖。编辑器只导出数据，不录制或回传视频。

文件结构、版本及容量边界见 [ZIP 协议](docs/package-format.md)。Unity 的录制实现仍属于 Unity 仓库，不移动到本项目。

## 保存与备份

- 修改停止 800 毫秒后保存，持续编辑每 5 秒保存；底部「保存」或 Command/Ctrl+S 可立即保存。
- 保存已确认译文、未确认输入、版本信息及编辑布局；未点「确定」的内容不会被自动确认。
- 保留最近 10 份本机备份，在「偏好 → 查看自动备份」中查看；这不代替独立设备备份。
- macOS 原生数据位于 `~/Library/Application Support/com.mida.localization/workspace/`。应用标识保持不变，避免升级丢失已有真实任务；无需将此目录放入 Git。
- 导入同一片段的新任务成功后清理旧媒体，新包没有视频也会清理。取消或保存失败保留旧媒体，其他片段不受影响。备份只保留媒体引用，不恢复已删除的视频。
- 磁盘空间不足、版本冲突或保存失败会显示错误，不假报保存成功。

## 目录

| 路径 | 职责 |
| --- | --- |
| `prototype.html` | 共用编辑器界面与交互，已移除内置示例数据 |
| `workspace-store.js` | 桌面 / 浏览器工作区持久化桥接 |
| `preview-player.js` | 视频加载与对话包节点定位 |
| `src-tauri/` | 桌面外壳、ZIP、文件选择、媒体与原子存储 |
| `serve.py`、`package_io.py`、`media_store.py` | 可选本机浏览器服务 |
| `scripts/prepare-desktop.cjs` | 前端构建准备 |
| `docs/package-format.md` | 与 Unity 的数据边界 |

依赖由 `package-lock.json` 和 `src-tauri/Cargo.lock` 锁定；`node_modules/`、`dist/`、`target/`、用户数据和安装产物不提交。
