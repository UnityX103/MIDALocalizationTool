# 本项目的有界响应补丁

来源：crates.io tauri-plugin-updater 2.11.0（保留原版 MIT / Apache-2.0 许可证）。
本地只修改 src/updater.rs 与 src/error.rs：更新描述 4 MiB、安装包 512 MiB 的声明长度和实际累计长度检查，在追加内存前拒绝超限。保留原版签名验证与安装路径。升级上游时必须重新核对或移除补丁，禁止恢复无限制读取。
