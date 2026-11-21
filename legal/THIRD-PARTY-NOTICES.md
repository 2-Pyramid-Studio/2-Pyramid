# 第三方组件声明 / Third-Party Notices

**生效日期：** 2026-10-16

2-Pyramid 包含或链接若干第三方开源组件。这些组件按其**各自许可证**提供；本声明仅作汇总，完整文本以各组件仓库 / `Cargo.lock` / `package-lock.json` 为准。

主程序许可证见仓库 [`LICENSE`](../LICENSE)（MIT）。第三方许可证条款与 MIT 冲突时，**就该组件而言**以第三方自身许可证为准；主程序自身仍以 MIT 为准。

---

## 1. Rust / Tauri 后端（主程序）

| 组件 | 许可证（常见） | 用途 |
|------|----------------|------|
| [Tauri](https://tauri.app) | MIT OR Apache-2.0 | 桌面壳与系统能力 |
| tauri-plugin-dialog / opener / notification | MIT OR Apache-2.0 | 对话框、打开链接、系统通知 |
| [serde](https://serde.rs) / serde_json | MIT OR Apache-2.0 | 序列化 |
| [image](https://crates.io/crates/image) | MIT OR Apache-2.0 | 贴图读写 |
| [zip](https://crates.io/crates/zip) | MIT | 资源包打包 |
| [flate2](https://crates.io/crates/flate2) | MIT OR Apache-2.0 | 分享码压缩 |
| [base64](https://crates.io/crates/base64) | MIT OR Apache-2.0 | 分享码编码 |
| [walkdir](https://crates.io/crates/walkdir) | Unlicense OR MIT（crates.io 记作 `Unlicense/MIT`） | 目录遍历 |
| [rayon](https://crates.io/crates/rayon) | MIT OR Apache-2.0 | 并行处理 |
| [tokio](https://tokio.rs) | MIT | 异步运行时 |
| [reqwest](https://crates.io/crates/reqwest) | MIT OR Apache-2.0 | 更新检查与下载 |
| [rustls](https://github.com/rustls/rustls) | MIT OR Apache-2.0 OR ISC | TLS（reqwest 依赖） |
| [chrono](https://crates.io/crates/chrono) | MIT OR Apache-2.0 | 时间 |
| [uuid](https://crates.io/crates/uuid) | MIT OR Apache-2.0 | 项目 ID |
| [regex](https://crates.io/crates/regex) | MIT OR Apache-2.0 | 文本处理 |
| [dirs](https://crates.io/crates/dirs) | MIT OR Apache-2.0 | 系统目录 |
| [tempfile](https://crates.io/crates/tempfile) | MIT OR Apache-2.0 | 临时目录 |
| [fs_extra](https://crates.io/crates/fs_extra) | MIT | 文件操作 |
| [lazy_static](https://crates.io/crates/lazy_static) | MIT OR Apache-2.0 | 静态初始化 |
| [sha2](https://crates.io/crates/sha2) | MIT OR Apache-2.0 | 哈希校验 |
| [winreg](https://crates.io/crates/winreg) | MIT | Windows 注册表 |
| [windows-sys](https://crates.io/crates/windows-sys) | MIT OR Apache-2.0 | Windows API 绑定 |

> 权威清单以仓库内 `src-tauri/Cargo.lock`、`installer-app/src-tauri/Cargo.lock` 为准。

## 2. 前端

| 组件 | 许可证（常见） | 用途 |
|------|----------------|------|
| [Vue.js](https://vuejs.org) | MIT | UI 框架 |
| [vue-i18n](https://kazupon.github.io/vue-i18n/) | MIT | 多语言 |
| [Vite](https://vitejs.dev) | MIT | 构建 |
| [TypeScript](https://www.typescriptlang.org) | Apache-2.0 | 类型 |
| [@tauri-apps/api](https://github.com/tauri-apps/tauri) | MIT OR Apache-2.0 | 前端桥接 |
| @tauri-apps/plugin-dialog / plugin-notification / plugin-opener | MIT OR Apache-2.0 | 文件对话框、系统通知、打开链接（**随包分发**） |
| [Remix Icon](https://remixicon.com) | Apache-2.0（见 `package-lock.json` 的 `license` 字段） | 图标字体 |
| [@vitejs/plugin-vue](https://github.com/vitejs/vite-plugin-vue) | MIT | Vue 插件 |

> 以 `package.json` 与 lockfile 为准。`@tauri-apps/cli`、`vue-tsc`、`sharp` 等构建期依赖不随安装包分发，故未逐条列出。

## 3. 安装器（installer-app）

与主程序同栈（Tauri 2 + Rust），**本身同样以 MIT 许可证提供**，依赖见 installer-app/src-tauri/Cargo.lock。其直接依赖（`tauri` / `tauri-plugin-dialog` / `tauri-plugin-opener` / `serde` / `serde_json` / `zip` / `winreg` / `dirs`）均在上表「Rust / Tauri 后端」中列出。

## 4. 平台与游戏资产

- **Minecraft** 相关名称、商标与游戏内行为归 Mojang Studios / Microsoft 所有。本软件不包含 Minecraft 客户端或官方资源；转换对象为你自己合法持有的资源包。
- 转换过程中生成的贴图衍生自**你导入的**资源包，著作权仍归原权利人。
- **参考数据（从官方客户端重新生成）**：转换规则需要「物品模型定义模板」「旧原版模型/方块状态的规范化指纹」等参考数据。这些数据由本仓库的生成脚本（`tools/gen-item-definitions.py`、`tools/gen-mc-reference.py`）从**官方 Minecraft Java 客户端 jar / Mojang 官方版本清单**重新生成，随源码提交的只是**包根相对路径**与**规范化 JSON 的 SHA-256**（或方块的物品定义 JSON 结构），**不包含、不分发任何官方纹理、模型、音效或其它游戏资源文件**。它们用于识别「未修改的原版文件」并在目标版本生成等价定义；Minecraft 官方资源仍归 Mojang Studios / Microsoft 所有，使用本软件不授予任何官方资源的分发权。生成脚本下载的 Mojang 官方客户端 jar 只作为**本机构建期缓存**存在于 `tools/.mc-cache/`（已被 `.gitignore` 排除），不随源码、安装包或转换产物分发；仓库与发行版中只有脚本及其生成的「路径 + SHA-256」数据，以及 `source.jars` 里每个客户端 jar 的版本号 / SHA-256 / 字节数等来源元信息（同样不含任何资源内容）。

## 5. 如何获取完整许可证文本

- **安装包内**：`legal/third-party-licenses/`（构建时自动收集随包分发组件的许可证与 NOTICE 正文；
  含 MPL-2.0 / Apache-2.0 这类要求随二进制提供条款的组件）
- Rust：`cargo license` 或各 crate 在 [crates.io](https://crates.io) 的 LICENSE 文件
- 前端：`node_modules/<pkg>/LICENSE`
- 本仓库根目录：`LICENSE`（MIT）

若你认为本声明遗漏了应列组件，请开 Issue，我们会尽快补全。

---

**English summary:** 2-Pyramid ships with open-source libraries under MIT OR Apache-2.0 and similar licenses. See `Cargo.lock` and `package-lock.json` for the authoritative dependency list. Minecraft branding belongs to Mojang/Microsoft; we ship no official game assets.
