# 2-Pyramid 最终用户协议（End User Agreement）

**生效日期：** 2026-10-24  
**适用对象：** 获取并使用 2-Pyramid 安装包或可执行程序的个人或组织。

---

## 0. 与开源许可证的关系（请先读这一条）

本项目**源代码、官方构建产物与自研安装器（`installer-app`）**均以 **MIT 许可证** 提供，见仓库 [`LICENSE`](../LICENSE)。

- 你依据 MIT 已享有使用、复制、修改、合并、发布、再分发、再许可及销售副本的权利。**本协议不撤销、不缩减这些权利**，也不向你施加 MIT 之外的额外限制。这包括适用法律允许范围内的逆向工程与研究；本项目不含闭源「专有部分」，故不另设此类限制。
- MIT 与本协议表述不一致时，**以 MIT 为准**。
- 本协议的作用仅限于：
  1. 提醒你遵守 **Minecraft EULA** 等第三方规则；
  2. 请你阅读并确认 [`DISCLAIMER.md`](./DISCLAIMER.md) 与 [`PRIVACY.md`](./PRIVACY.md)；
  3. 说明 **商标** 与「非官方」身份的正确表述方式。

建议你在使用前阅读上述文件。**本协议不构成 MIT 之外的许可条件**——你依 MIT 行使权利不以接受本协议为前提。（若你是从源码自行构建，本协议同样不适用。）

---

## 1. 你使用软件时的自身责任

1. **Minecraft 条款。** 转换、使用或分发资源包时，你须自行确保符合 Mojang Studios / Microsoft 的 EULA 与使用条款，以及资源包原作者的授权。本软件不帮你判断合法性。
2. **合法用途。** 不得将本软件用于侵犯他人权利或任何违法活动。
3. **备份与验证。** 转换属启发式处理，可能失败或损坏文件。请先备份原件并在测试环境验证。

## 2. 转换产物中的元数据文件（2pyr-gen）

转换成功的 **Java 版**资源包产物会在**包根目录**写入一个 `2pyr-gen.json`，记录本次转换的参数。该行为：

- **默认开启**，可在转换页关闭（设置随配置保存）。关闭后产物中不会生成该文件。
- **记录内容**：生成器名称与版本号、构建号（BUILD）、生成时间（UTC，RFC 3339 秒级）、源与目标的 `pack_format`、目标版本标签（能可靠取得时），以及本次转换的六个开关（图层修复、着色器适配、彻底删除 shaders、物品定义升级、OptiFine CIT 转换、彻底删除 CIT）。此外还有一行**中性说明**，告知拿到包的人该文件由 2-Pyramid 生成、Minecraft 不读取、可在转换页关闭。
- **不包含任何个人信息**：不含文件路径、Windows 用户名、机器名、硬件 ID、UUID、IP 地址或网络信息；它只描述转换参数。
- **用途**：让转换产物可追溯（判断一份包由哪一版工具、以什么参数生成），并在排障时提供上下文。
- **Minecraft 客户端不读取该文件**，它不参与资源包加载：文件位于包根目录，客户端忽略未知根文件。
- 该文件**随产物包一起分发**，接收方同样能看到上述转换参数（其中不含你的个人信息）。敏感场景请在转换页关闭该开关，或转换后手动删除该文件。

## 3. 商标与「非官方」表述

- 你可以按 MIT 再分发本软件；再分发时请保留 `LICENSE` 与版权声明（MIT 唯一硬性条件）。
- 「2-Pyramid」名称与标识用于指称本项目。请勿暗示你分发的版本得到 2-Pyramid Studio 官方背书，除非确属官方发布。
- MIT 许可证**不授予**「2-Pyramid」名称、标识的任何商标许可；再分发时仅可作合理指称。
- 本软件与 **Mojang Studios / Microsoft 无隶属、无赞助、未获认可**。请勿使用其商标暗示官方合作；指称 Minecraft 时仅作说明性使用。

## 4. 免责（摘要）

本软件按「现状」提供，不附带担保；作者不承担因使用产生的损失。完整文本见 [`DISCLAIMER.md`](./DISCLAIMER.md)。该摘要不替代 MIT 中的 AS IS 条款，亦不额外加重你的义务。

## 5. 隐私

默认完全本地处理、无账号、无遥测；联网仅用于更新检查（**「自动检查更新」默认开启，可在设置关闭**）、更新下载，以及你自行启用的 Foray AI 分析。导出的日志默认脱敏。转换产物默认包含一份转换参数记录（`2pyr-gen.json`，不含个人信息，见第 2 节，可在转换页关闭）。详见 [`PRIVACY.md`](./PRIVACY.md)。

## 6. 第三方组件

依赖库按各自许可证提供，见 [`THIRD-PARTY-NOTICES.md`](./THIRD-PARTY-NOTICES.md)。

## 7. 变更与联系

本协议更新会在仓库说明。更新**不影响**你依 MIT 已取得的权利，也不新增任何 MIT 之外的义务。疑问请开 [Issues](https://github.com/realLivedInCorner/2-Pyramid/issues)。

---

**English (summary):** The Software is MIT-licensed; this agreement does **not** add restrictions beyond MIT. It reminds you to comply with the Minecraft EULA, read the disclaimer and privacy notice, and use trademarks without implying official Mojang/Microsoft affiliation. Successful conversions write a small parameter record (`2pyr-gen.json`) into the output pack by default — it contains no personal information, is ignored by the Minecraft client, and can be disabled on the conversion page (see §2 and [`PRIVACY.md`](./PRIVACY.md)). If anything here conflicts with MIT, **MIT controls**.
