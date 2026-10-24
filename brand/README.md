# 2-Pyramid 品牌资产（Precision Geometry）

维护者已于 2026-10-24 采纳《2-Pyramid 品牌视觉方向提案》的 **Precision Geometry（精密几何）** 方向。
本目录是该方向的唯一矢量源：所有应用内 / 网页 / OS 图标资产都应由这里的 SVG 生成，不要手改生成物。
提案原文与评审记录见本目录 PDF 与 [review-2026-10-24.md](./review-2026-10-24.md)。

## 方向摘要：Precision Geometry 四原则

1. **保持单色、几何** —— 只用黑色（或深色背景下的白色）线条，禁止渐变、多色与描边特效。
2. **内部斜线是结构 / 转换的暗示**，不是对几何形状的字面解释：它保留原标识的“切割”记忆，不追求把三角形解释清楚。
3. **清晰、一致优先** —— 拒绝通用 AI 渐变与 Minecraft 主题的陈词滥调；标识在任何尺寸下都必须可分辨。
4. **按真实使用尺寸验收** —— 必须在 16 / 24 / 32 / 48 / 128 / 256 px 以及 Windows 任务栏、标题栏、安装器、MSIX、README 实际尺寸下看过再替换发行资产。

与旧标识的关系：这是**现有三角形标识的演进 / 规整**，不是换品牌。旧 `2pyr-logo.svg`（680 画布、墨迹 45.1%×49.8%）
已由本目录的 512 画布版本取代；内线终点与外面 B–C 边之间保留约 **34/512（≈6.6%）间隙**，该“留间隙”已作为方向接受。

## 资产清单与用途

| 文件 | 画布 / 线宽 | 用途 |
|---|---|---|
| [2-pyramid-mark.svg](./2-pyramid-mark.svg) | 512 / 12·9 | **主标识**。≥64px 的界面、文档、开屏、大尺寸展示；`src/assets/logo-*.png`、`public/logo-mark.png` 的源 |
| [2-pyramid-mark-on-dark.svg](./2-pyramid-mark-on-dark.svg) | 512 / 12·9 | 主标识**白色版**。深色背景、README/网页深色主题、`logo-white.png` 的源 |
| [2-pyramid-favicon.svg](./2-pyramid-favicon.svg) | 430 视口（32 命名）/ 20·16 | **小尺寸加粗版**。≤48px：浏览器 tab、任务栏、小图标；`public/favicon*.png` 的源 |
| [2-pyramid-favicon-white.svg](./2-pyramid-favicon-white.svg) | 430 视口 / 20·16 | 加粗版的白色版，深色背景上的小尺寸 |
| [2-pyramid-micro.svg](./2-pyramid-micro.svg) | 430 视口 / 30·24 | **16–24px 专用**光学变体（线宽再加粗，避免亚像素变灰）。只用在 16–24px，**不要**用于 32px 以上，32px 以上用 favicon 版 |
| [2-pyramid-horizontal.svg](./2-pyramid-horizontal.svg) | 960×240 / mark 0.42 缩放 | **横向组合**（mark + “2-Pyramid” + 描述语），用于文档封面、README 页脚、商店材料等横向版位。文字已**转曲**，无字体依赖 |
| [2-pyramid-horizontal-white.svg](./2-pyramid-horizontal-white.svg) | 960×240 | 横向组合白色版，深色背景用 |
| [2-Pyramid_Brand_Direction_Proposal.pdf](./2-Pyramid_Brand_Direction_Proposal.pdf) | 5 页 | 2026-10-24 提案原文归档（方向、实测数据、阶段计划） |
| [review-2026-10-24.md](./review-2026-10-24.md) | — | 独立评审结论与维护者裁定记录（含实测数字） |

尺寸分档（重要，不要跨档乱用）：

- **16–24px**：`micro`（加粗光学版）
- **≤48px**：`favicon`（32 命名画布）
- **≥64px**：`mark`（512 画布，透明底）
- **横向版位**：`horizontal`（含描述语，文字已转曲）

## 横向组合：文字已转曲

`2-pyramid-horizontal.svg` / `...-white.svg` 不含任何 `<text>` / `font-family`，字形轮廓由 Windows WPF
`GlyphTypeface.GetGlyphOutline` 从 **Arial Bold（标题）/ Arial Regular（描述语）** 提取并转成 SVG path；
排版参数与提案一致：标题 62px、letter-spacing -2、基线 y=110、起点 x=220；描述语 `RESOURCE PACK CONVERTER`
23px、letter-spacing 0.6、基线 y=155、起点 x=223；mark 组 `translate(12 12) scale(0.42)`。
（提案写的是 Inter 优先，但参考渲染的实际环境没有 Inter，浏览器落到 Arial；转曲时用 Arial 以复现参考渲染的
字宽与字重，实测逐字位置与 Chrome 的 Canvas 排版结果一致。）

因此该 SVG **不依赖系统字体**，跨机渲染一致。若将来要改描述语文案或字号，必须重新转曲而不是直接编辑 path；
改完后用 headless Chrome 渲染 960×240，与提案原渲染（评审时的高分辨率对照图）逐像素 / 目检对比。

## 落地规则（仓库内）

1. **PNG 一律由脚本生成，不手改**：
   - 应用内 / 运行时 PNG：`node tools/generate-logos.mjs`（`npm run logos`），源是本目录 SVG，输出清单见脚本内 `TASKS`。
   - 输出契约（名字不要改）：`src/assets/logo-256.png`、`logo-512.png`、`logo-256-white.png`、`app-icon.png`；
     `public/favicon.png`、`favicon-192.png`、`logo-mark.png`、`logo-transparent.png`、`logo-white.png`、`logo-192.png`。
   - 所有 PNG 均为**透明底**；`logo-white.png` 是白色线条 + 透明底（旧文件曾是白底不透明，已修正）。
2. **OS 图标（Stage 2，由 `tools/generate-icons.mjs` 生成）**：采用**白底圆角板 + 尺寸分档 artwork**
   （16–48 用 favicon 加粗版、64+ 用 mark 细线版），不要把 87% 满墨迹的 favicon 直接塞进 MSIX 瓦片；
   瓦片 / StoreLogo 需要按各自尺寸留安全边距。`src-tauri/icons/*` 与 `installer-app/src-tauri/icons/*`
   都是生成物，不要手改，也不要直接用本目录 SVG 覆盖 `.ico` / MSIX 资产。
3. **应用内主题适配**：应用内引用黑色 PNG（`src/assets/logo-256.png` 等）；深色主题下通过 CSS `filter: invert(1)`
   复用同一张图（或按需改用 `logo-256-white.png`），不要为每个主题各存一份不必要的位图。
4. **README / 网页**：用 `<picture>` 按 `prefers-color-scheme` 切亮暗两版（GitHub 支持）：

   ```html
   <picture>
     <source media="(prefers-color-scheme: dark)" srcset="./src/assets/logo-256-white.png">
     <img src="./src/assets/logo-256.png" width="160" alt="2-Pyramid logo">
   </picture>
   ```

5. **小尺寸验收**：16 / 24 / 32 / 48 / 128 / 256 px 在浅底与深底上都必须可辨认；改动任何 SVG 后重跑一次
   本目录资产的下游生成，并检查 `public/favicon.png` 的 32px 观感（可放大 8 倍目检）。

## 再生成命令

```bash
# 品牌 PNG（应用内 + public/，sharp，源为本目录 SVG）
node tools/generate-logos.mjs      # 或 npm run logos

# OS 图标（ICO / MSIX / 安装器图标，Stage 2 起可用）
node tools/generate-icons.mjs      # 或 npm run icons
```

## 来源与评审

- 提案原文（归档）：[2-Pyramid_Brand_Direction_Proposal.pdf](./2-Pyramid_Brand_Direction_Proposal.pdf)
  （原始输入：`2-Pyramid_Brand_Assets.zip` 内 5 个 SVG + README）。
- 独立评审与实测：[review-2026-10-24.md](./review-2026-10-24.md)（旧稿墨迹比例、小尺寸像素实测、
  仓库落地依赖、Stage 1–3 顺序、维护者裁定）。
- 方向决定：采用 Precision Geometry；OS 图标采用白底圆角板 + 尺寸分档 artwork；mark / favicon / micro
  按尺寸分档使用（详见评审记录）。
