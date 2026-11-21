//! **纹理卫生三任务**：格式识别 / 近透明背景清理 / 图层修复。
//!
//! 三者的共同点：**只改 PNG 字节、不碰其它条目**。它们共享本文件的"枚举 PNG"辅助，
//! 但各自独立成任务、独立 Tier、独立注入条件（由 `native_run::inject_hygiene_tasks` 注入）：
//!
//! | 任务 | Tier | 触发 | 作用 |
//! |---|---|---|---|
//! | [`fix_mislabeled_textures`] | Eraser | **始终**（生产 `NativeSwitches::all()`） | 内容是 JPEG 却叫 `.png` 的贴图 → 重新编码成真 PNG |
//! | [`fix_alpha_layers_in_textures`] | Surgeon | 用户开关 `fix_alpha_layers` | 历史「图层修复」的 9 条像素规则 |
//! | [`fix_item_transparency`] | Surgeon | 源 format ≤1 且目标 ≥2（1.6–1.8 → 1.9+） | 清掉大面积的 alpha∈{1,2} 假透明背景 |
//!
//! ## 来源与许可证边界（重要）
//!
//! * **A7 的像素规则是移植**：来自本仓库自己的 MIT 历史实现
//!   `archive/legacy-converters/textures/alpha_layers.rs`
//!   （`git show 7b81ed5:archive/legacy-converters/textures/alpha_layers.rs` 可复核）。
//!   规则数值与判定顺序逐条保留，只把「磁盘路径 + `img.save()`」换成 A-ROM 的
//!   「`tx.image()` → `tx.put_image()`」。
//! * **A1/A2 是按行为契约从零实现的**：竞品 Fast-Pack 是 **GPL-3.0-only**，只参考了
//!   它 README/功能清单里的**行为描述**（"识别 JPEG 伪装的 PNG"、"清理旧工具近透明
//!   背景"），**没有打开/复制它的源码或数据表**。
//!
//! ## 为什么都在 Surgeon/Eraser 而不是 Architect
//!
//! 三个任务都**不生成新资源**，只修正已有贴图字节，属于"就地修改"（Surgeon 的定义）；
//! A1 还会影响后续所有读贴图的任务（JPEG 伪装图会让它们解码失败静默跳过），
//! 因此放 **Eraser 级并插在计划最前**——越早把字节修正，后面的读者越多地受益。
    use super::*;

    /// 所有纹理所在根（A1 扫描它；A7 只扫其中历史实现列出的 7 个子目录）。
    pub(crate) const TEXTURES_ROOT: &str = "assets/minecraft/textures";

    /// 列出前缀下所有 `.png` 文件（大小写不敏感），路径排序 + 去重，保证**确定性**。
    ///
    /// 为什么用 `tx.list` 而不是 walkdir：A-ROM 里包是**条目表**，"目录"可能只由文件隐含
    /// （压缩包里通常没有显式目录条目）；`tx.list` 对两种形态都成立。排序用于让
    /// 日志/行为可复现（同一包两次转换的扫描顺序一致）。
    pub(crate) fn png_paths(tx: &Tx<'_>, prefixes: &[&str]) -> Result<Vec<String>, AromError> {
        let mut out = Vec::new();
        for prefix in prefixes {
            for entry in tx.list(prefix)? {
                if entry.is_dir {
                    continue;
                }
                if entry.path.to_ascii_lowercase().ends_with(".png") {
                    out.push(entry.path);
                }
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// **A1：真实格式识别**——内容是 JPEG（magic `FF D8 FF`）但名字是 `.png` 的贴图，
    /// 解码后用 PNG 编码写回；真 PNG 与其它文件**一个字节都不动**。
    ///
    /// ## 为什么需要它
    ///
    /// 1.6–1.8 时代的工具链/Bug 会产出「JPEG 内容 + `.png` 扩展名」的贴图。`image` crate
    /// 的 `load_from_memory` 按内容猜格式，很多读取路径**能**解出来，因此这类包在旧版本
    /// 看起来正常；但：
    ///
    /// * 现代客户端按**扩展名/资源路径**走 PNG 解码器，直接判为坏图 → missing texture；
    /// * 本仓库的部分任务用 `image::load_from_memory`（容错）而另一些按扩展名处理，
    ///   同一个包在不同阶段表现不一致，排障时会得到互相矛盾的读数。
    ///
    /// 在管线最前面统一改成真 PNG，是唯一不会"看运气"的做法。
    ///
    /// ## 为什么必须先读原始字节
    ///
    /// 不能先 `tx.image(path)` 再判断：`PackView::image` 用 `load_from_memory`，JPEG 伪装图
    /// 会**解码成功**，于是"重编码成 PNG"这件事无声地不发生。必须先 `tx.read(path)` 拿
    /// raw bytes、看 magic，再决定是否解码。
    ///
    /// ## 范围取舍：只扫 `assets/minecraft/textures`
    ///
    /// 其它命名空间（`assets/<modid>/textures/**`）理论上也可能有 JPEG 伪装图，但：
    /// ① 这类包绝大多数是 1.6–1.8 的原版替换包，扩展命名空间很少；
    /// ② 全包扫描会把 A1 的 I/O 从"贴图树"扩到"整包"，而收益只覆盖长尾。
    /// 当前只扫 `assets/minecraft/textures`；要扩时把前缀数组加一项即可（helper 已支持）。
    pub mod fix_mislabeled_textures {
        use super::*;

        /// JPEG 的 SOI + 第一个 marker 字节。只要 3 字节：完整合法性校验交给
        /// `image` 解码器；这里只回答"值不值得尝试解码"，读多了反而可能把畸形文件误判。
        pub const JPEG_MAGIC: [u8; 3] = [0xFF, 0xD8, 0xFF];

        pub fn decl() -> TaskDecl {
            TaskDecl::new("fix_mislabeled_textures", Tier::Eraser)
                .reads(ScopeSet::prefix(TEXTURES_ROOT))
                .writes(ScopeSet::prefix(TEXTURES_ROOT))
                .exclusive(true)
        }

        fn looks_like_jpeg(bytes: &[u8]) -> bool {
            bytes.len() >= JPEG_MAGIC.len() && bytes[..JPEG_MAGIC.len()] == JPEG_MAGIC
        }

        pub fn run(tx: &mut Tx<'_>) -> Result<Outcome, AromError> {
            let started = std::time::Instant::now();
            let paths = png_paths(tx, &[TEXTURES_ROOT])?;
            let scanned = paths.len();
            let mut outcome = Outcome::default();
            for path in paths {
                // raw bytes：见模块文档「为什么必须先读原始字节」。
                let Some(bytes) = tx.read(&path)? else {
                    continue;
                };
                if !looks_like_jpeg(&bytes) {
                    // 正常 PNG（或其它内容）**逐字节不动**：这是默认产物不变的保证。
                    continue;
                }
                match image::load_from_memory(&bytes) {
                    Ok(decoded) => {
                        let img = decoded.to_rgba8();
                        let (w, h) = (img.width(), img.height());
                        tx.put_image(&path, &img)?;
                        outcome.changed += 1;
                        outcome
                            .notes
                            .push(format!("{path}: JPEG 伪装 → PNG（{w}×{h}）"));
                    }
                    Err(e) => {
                        // 有 JPEG magic 却解不开 = 损坏文件。**不改写**（写坏 PNG 更糟），
                        // 只记 note 让人能从日志里看到它被跳过了。
                        outcome.skipped += 1;
                        outcome
                            .notes
                            .push(format!("{path}: JPEG magic 但解码失败，跳过（{e}）"));
                    }
                }
            }
            // **无条件**打一行扫描摘要（即使 0 处修改）：A1 是"每一条版本边都跑"的任务，
            // 用户/性能分析需要知道它到底扫了多少张图、耗时多少；只在有改动时打日志
            // 会让"它跑没跑"在默认包上完全不可见。
            // 实测（真实包 `§0Black Soul`，1068 张贴图，debug 构建）：99.4ms，
            // 约占整次转换 7.97s 的 1.2%；release 更快。这个数字也说明"始终执行"的成本可接受。
            crate::log_info!(
                "fix_mislabeled_textures: scanned={}, rewritten={}, skipped={}, elapsed={:.1}ms",
                scanned,
                outcome.changed,
                outcome.skipped,
                started.elapsed().as_secs_f64() * 1000.0
            );
            Ok(outcome)
        }
    }

    /// **A2：旧包近透明背景清理**（1.6–1.8 → 1.9+ 专用）。
    ///
    /// ## 行为
    ///
    /// 旧工具/旧客户端常把"透明背景"存成 **alpha=1 或 2**（近乎全透明但非 0）。1.9+
    /// 的物品模型生成会把"非全透明像素"当成实体面，于是物品侧面会沿这个假背景的
    /// **矩形边缘**生成，而不是沿真实轮廓——表现为一圈脏边/多余侧面。
    ///
    /// 本任务把**大块且触边**的 alpha∈{1,2} 连通区域清成真正的透明（alpha=0 且 RGB=0），
    /// 让 1.9+ 的模型生成只看到真实轮廓。其余像素、尺寸、`.png.mcmeta`（动画帧元数据）
    /// 全部保留——只重写 PNG 本身。
    ///
    /// ## 判定参数（为什么是这几个数）
    ///
    /// * **候选像素 alpha ∈ {1,2}**：alpha=0 是"真透明"，不算假背景。若把纯 0 也算候选，
    ///   1.6–1.8 正常透明背景会被无意义地重写（默认产物就变了）——因此连通块里
    ///   **至少要有一个 1/2 像素**才会触发清理（`has_near_transparent`）。
    /// * **4 邻域连通**：物品轮廓是上下左右连通的；8 邻域会把对角只挨一点的噪点并进来，
    ///   容易把作者画的独立像素一起清掉。
    /// * **面积下限 `max(64, 5%)`**：
    ///   - 16×16 物品图共 256px，5% 只有 13px，太小（一条 4px 边就够）；
    ///     64px ≈ 贴图的 1/4，是"一眼可见的背景块"的下限；
    ///   - 256×256 大图共 65536px，64px 又太小，5% = 3277px 才是"大面积"。
    ///   取两者较大值同时覆盖小图与大图。实测：16×16 的假背景通常 ≥64px（例如
    ///   左半 8×16=128px、四条边 60px），而抗锯齿留下的近透明像素一般是几个像素级。
    /// * **必须触边**：假背景一定延伸到贴图边界；内部的近透明小块（高光、抗锯齿）
    ///   是画出来的，不该清。
    ///
    /// ## 注入条件
    ///
    /// `native_run::inject_hygiene_tasks` 只在 `source_version <= 1 && target_version >= 2`
    /// 时注入：format 1 是 1.6–1.8，1.9 起 format=2，正是"物品贴图从整图平面变成
    /// 按像素轮廓生成侧面"的版本边界。其它版本边不动（避免无谓重写）。
    pub mod fix_item_transparency {
        use super::*;

        /// 两个物品目录：`items/` 是 1.6–1.8 旧路径，`item/` 是 1.9+ 路径。
        /// 旧包升新版时这两个目录可能同时存在（改名任务在 Eraser 级，本轮不依赖它）。
        pub const ITEM_ROOTS: [&str; 2] = [
            "assets/minecraft/textures/items",
            "assets/minecraft/textures/item",
        ];

        /// 面积下限的绝对像素数（见模块文档：16×16 的 1/4）。
        pub const MIN_AREA: usize = 64;
        /// 面积下限的相对比例（大图用）。
        pub const AREA_PERCENT: usize = 5;

        /// 区域面积门槛：`max(64, ceil(5%))`。向上取整，避免 5% 恰好为小数时少 1px 的歧义。
        pub(crate) fn area_threshold(total: usize) -> usize {
            let pct = (total * AREA_PERCENT + 99) / 100;
            pct.max(MIN_AREA)
        }

        pub fn decl() -> TaskDecl {
            let mut scope = ScopeSet::none();
            for root in ITEM_ROOTS {
                scope = scope.union(&ScopeSet::prefix(root));
            }
            TaskDecl::new("fix_item_transparency", Tier::Surgeon)
                .reads(scope.clone())
                .writes(scope)
                .exclusive(true)
        }

        /// 清掉一张图里所有"大块、触边、含 alpha 1/2"的连通近透明区域。
        ///
        /// 返回 `(新图, 被清掉的像素数)`；无改动时返回 `None`（调用方据此**不写回**，
        /// 保证正常 PNG 逐字节不变）。
        pub(crate) fn clear_fake_background(img: &RgbaImage) -> Option<(RgbaImage, usize)> {
            let (width, height) = img.dimensions();
            if width == 0 || height == 0 {
                return None;
            }
            let total = (width as usize) * (height as usize);
            let need = area_threshold(total);
            let index = |x: u32, y: u32| (y as usize) * (width as usize) + (x as usize);

            // 候选掩码：alpha <= 2（含 0；但要所在连通块里至少有一个 1/2 才触发）。
            let mut candidate = vec![false; total];
            for y in 0..height {
                for x in 0..width {
                    if img.get_pixel(x, y)[3] <= 2 {
                        candidate[index(x, y)] = true;
                    }
                }
            }

            let mut visited = vec![false; total];
            let mut out: Option<RgbaImage> = None;
            let mut cleared_total = 0usize;

            for y in 0..height {
                for x in 0..width {
                    let start = index(x, y);
                    if !candidate[start] || visited[start] {
                        continue;
                    }
                    // 显式栈 flood fill（物品图最大 256×256；递归既没必要也不安全）。
                    let mut stack = vec![(x, y)];
                    visited[start] = true;
                    let mut component = Vec::new();
                    let mut touches_edge = false;
                    let mut has_near_transparent = false;
                    while let Some((cx, cy)) = stack.pop() {
                        component.push((cx, cy));
                        if cx == 0 || cy == 0 || cx + 1 == width || cy + 1 == height {
                            touches_edge = true;
                        }
                        let alpha = img.get_pixel(cx, cy)[3];
                        if alpha == 1 || alpha == 2 {
                            has_near_transparent = true;
                        }
                        for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                            let nx = cx as i32 + dx;
                            let ny = cy as i32 + dy;
                            if nx < 0 || ny < 0 || nx as u32 >= width || ny as u32 >= height {
                                continue;
                            }
                            let n = index(nx as u32, ny as u32);
                            if candidate[n] && !visited[n] {
                                visited[n] = true;
                                stack.push((nx as u32, ny as u32));
                            }
                        }
                    }

                    // 四个条件缺一不可：触边、够大、确实是"近透明"而不是纯 0 背景。
                    // 注意纯 0 背景（has_near_transparent=false）不是候选区域的触发者——
                    // 这保证正常透明背景不会被无意义地重写（默认产物不变）。
                    if !touches_edge || component.len() < need || !has_near_transparent {
                        continue;
                    }
                    let image = out.get_or_insert_with(|| img.clone());
                    for (px, py) in component {
                        let pixel = image.get_pixel_mut(px, py);
                        if pixel[3] != 0 || pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0 {
                            *pixel = image::Rgba([0, 0, 0, 0]);
                            cleared_total += 1;
                        }
                    }
                }
            }

            out.map(|image| (image, cleared_total))
        }

        pub fn run(tx: &mut Tx<'_>) -> Result<Outcome, AromError> {
            let paths = png_paths(tx, &ITEM_ROOTS)?;
            let scanned = paths.len();
            let mut outcome = Outcome::default();
            for path in paths {
                let img = match tx.image(&path) {
                    Ok(img) => (*img).clone(),
                    Err(e) => {
                        outcome.skipped += 1;
                        outcome
                            .notes
                            .push(format!("{path}: 解码失败，跳过（{e}）"));
                        continue;
                    }
                };
                let Some((cleaned, count)) = clear_fake_background(&img) else {
                    continue;
                };
                if count == 0 {
                    continue;
                }
                tx.put_image(&path, &cleaned)?;
                outcome.changed += 1;
                outcome
                    .notes
                    .push(format!("{path}: 清理 {count} 个近透明背景像素"));
            }
            if outcome.changed > 0 || outcome.skipped > 0 {
                crate::log_info!(
                    "fix_item_transparency: scanned={}, rewritten={}, skipped={}",
                    scanned,
                    outcome.changed,
                    outcome.skipped
                );
            }
            Ok(outcome)
        }
    }

    /// **A7：图层修复**（历史任务 `fix_alpha_layers_in_textures`，用户开关 `fix_alpha_layers`）。
    ///
    /// 这是 `natives` 里唯一"移植自本仓库历史代码"的任务：像素规则逐条来自
    /// `archive/legacy-converters/textures/alpha_layers.rs`（MIT，本仓库自己的快照）。
    ///
    /// ## 规则清单（与历史实现逐条对应）
    ///
    /// | 规则 | 触发 | 动作 |
    /// |---|---|---|
    /// | 1 | a=0 且 RGB≠0 | RGB 清 0（全透明像素不该带颜色） |
    /// | 2 | 0<a<255 且 RGB=0 | RGB 提亮到 `round(a*0.8)` 灰 |
    /// | 3/4 | alpha/RGB 越界 | u8 通道天然有界，**不需要**（历史实现同样跳过） |
    /// | 5 | 0<a<255 且亮度与 `a*0.8` 差 >50 | RGB 按比例缩放到亮度 ≈ `a*0.8` |
    /// | 6 | a=255 且三通道标准差 >80 | 向灰色混合 30%，压掉色偏 |
    /// | 7/10 | 不透明像素旁的**半透明邻居**亮度偏差 >50 | 改**邻居** RGB（不是当前像素） |
    /// | 8 | a=255 的纯黑（<30）且 4 邻域有不透明亮像素（均值>100） | 改成邻域平均灰 |
    /// | 9 | 0<a<255 且饱和度与 `a/255` 差 >0.3（Rule 5 未触发时） | 饱和度向 alpha 靠拢 |
    ///
    /// ## 为什么 Rule 6 的 else-if 会让 Rule 8 几乎不可达
    ///
    /// 历史实现的判定链是 `a==255 && (r|g|b != 0)`（Rule 6）在 Rule 8 之前，因此
    /// **除纯黑 (0,0,0,255) 之外**的 a=255 像素都会先进入 Rule 6 分支（即使标准差不足、
    /// 不做修改），不会再落到 Rule 8。这是历史行为，**照抄不"修正"**——单测
    /// `rule8_lifts_pure_black_opaque_pixel` 用纯黑中心覆盖真正可达的那条路径。
    ///
    /// ## 注意
    ///
    /// 规则会修改**邻居**像素（Rule 7/10），因此像素遍历顺序影响结果；历史实现是
    /// `x` 外层、`y` 内层，这里保持一致，否则同一张图会产出不同字节。
    pub mod fix_alpha_layers_in_textures {
        use super::*;

        /// 历史实现扫描的 7 个目录（顺序即声明里的 reads/writes 顺序，不影响行为）。
        pub const ROOTS: [&str; 7] = [
            "assets/minecraft/textures/items",
            "assets/minecraft/textures/item",
            "assets/minecraft/textures/blocks",
            "assets/minecraft/textures/block",
            "assets/minecraft/textures/entity",
            "assets/minecraft/textures/gui",
            "assets/minecraft/textures/misc",
        ];

        pub fn decl() -> TaskDecl {
            let mut scope = ScopeSet::none();
            for root in ROOTS {
                scope = scope.union(&ScopeSet::prefix(root));
            }
            TaskDecl::new("fix_alpha_layers_in_textures", Tier::Surgeon)
                .reads(scope.clone())
                .writes(scope)
                .exclusive(true)
        }

        /// 单像素 alpha/RGB 修复（历史实现逐字移植，见模块文档的规则表）。
        ///
        /// 返回是否修改了当前像素或其邻居。`width`/`height` 用于边界判断（邻居越界即跳过）。
        pub(crate) fn fix_pixel(
            img: &mut RgbaImage,
            x: u32,
            y: u32,
            width: u32,
            height: u32,
        ) -> bool {
            let mut changed = false;
            let p = *img.get_pixel(x, y);
            let (r, g, b, a) = (p[0], p[1], p[2], p[3]);
            let mut out = [r, g, b, a];

            // Rule 1: 全透明像素带着颜色 → RGB 清 0
            if a == 0 && (r != 0 || g != 0 || b != 0) {
                out[0] = 0;
                out[1] = 0;
                out[2] = 0;
                changed = true;
            // Rule 2: 半透明但 RGB 全 0 → 用 a*0.8 的灰提亮
            } else if a > 0 && a < 255 && r == 0 && g == 0 && b == 0 {
                let gray = ((a as f32) * 0.8).round().clamp(0.0, 255.0) as u8;
                out[0] = gray;
                out[1] = gray;
                out[2] = gray;
                changed = true;
            // Rule 3+4: alpha/RGB 越界 — u8 通道天然有界，跳过
            // Rule 5: 半透明且亮度远离 a*0.8 → RGB 等比缩放
            } else if a > 0 && a < 255 {
                let brightness = (r as f32 + g as f32 + b as f32) / 3.0;
                let expected = (a as f32) * 0.8;
                if (brightness - expected).abs() > 50.0 {
                    let scale = expected / brightness.max(1.0);
                    out[0] = (r as f32 * scale).round().clamp(0.0, 255.0) as u8;
                    out[1] = (g as f32 * scale).round().clamp(0.0, 255.0) as u8;
                    out[2] = (b as f32 * scale).round().clamp(0.0, 255.0) as u8;
                    changed = true;
                } else {
                    // Rule 9：仅在 Rule 5 未触发时校正饱和度
                    let r2 = r as f32;
                    let g2 = g as f32;
                    let b2 = b as f32;
                    let maxc = r2.max(g2).max(b2);
                    let minc = r2.min(g2).min(b2);
                    if maxc > 0.0 {
                        let saturation = (maxc - minc) / maxc;
                        let expected_sat = (a as f32 / 255.0).clamp(0.0, 0.95);
                        if (saturation - expected_sat).abs() > 0.3 {
                            let new_min = maxc * (1.0 - expected_sat);
                            let range = (maxc - minc).max(1.0);
                            let scale = (maxc - new_min) / range;
                            let map = |c: f32| {
                                (new_min + (c - minc) * scale).clamp(0.0, 255.0) as u8
                            };
                            out[0] = map(r2);
                            out[1] = map(g2);
                            out[2] = map(b2);
                            changed = true;
                        }
                    }
                }
            // Rule 6: 不透明且三通道标准差大 → 向灰混合 30%
            } else if a == 255 && (r != 0 || g != 0 || b != 0) {
                let avg = (r as f32 + g as f32 + b as f32) / 3.0;
                let std_dev = (((r as f32 - avg).powi(2)
                    + (g as f32 - avg).powi(2)
                    + (b as f32 - avg).powi(2))
                    / 3.0)
                    .sqrt();
                if std_dev > 80.0 {
                    let balance = 0.3;
                    out[0] = (r as f32 * (1.0 - balance) + avg * balance)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    out[1] = (g as f32 * (1.0 - balance) + avg * balance)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    out[2] = (b as f32 * (1.0 - balance) + avg * balance)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    changed = true;
                }
            // Rule 8: 不透明纯黑、被不透明亮像素包围 → 取邻域平均灰（防"印章黑块"）
            } else if a == 255 && r < 30 && g < 30 && b < 30 {
                let mut neighbor_brightness = Vec::new();
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx >= 0 && ny >= 0 && (nx as u32) < width && (ny as u32) < height {
                        let n = img.get_pixel(nx as u32, ny as u32);
                        if n[3] == 255 {
                            neighbor_brightness
                                .push((n[0] as f32 + n[1] as f32 + n[2] as f32) / 3.0);
                        }
                    }
                }
                if !neighbor_brightness.is_empty() {
                    let avg =
                        neighbor_brightness.iter().sum::<f32>() / neighbor_brightness.len() as f32;
                    if avg > 100.0 {
                        let value = avg.round().clamp(0.0, 255.0) as u8;
                        out[0] = value;
                        out[1] = value;
                        out[2] = value;
                        changed = true;
                    }
                }
            }

            // Rule 7/10：不透明像素旁的半透明邻居 — 独立于 Rule 6/8 执行，
            // 否则非黑不透明像素会先被 Rule 6 的 else-if 吃掉。
            if a == 255 {
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || (nx as u32) >= width || (ny as u32) >= height {
                        continue;
                    }
                    let n = *img.get_pixel(nx as u32, ny as u32);
                    if n[3] == 0 || n[3] == 255 {
                        continue;
                    }
                    let nb = (n[0] as f32 + n[1] as f32 + n[2] as f32) / 3.0;
                    let expected = (n[3] as f32) * 0.8;
                    if (nb - expected).abs() <= 50.0 {
                        continue;
                    }
                    let scale = expected / nb.max(1.0);
                    let nr = (n[0] as f32 * scale).round().clamp(0.0, 255.0) as u8;
                    let ng = (n[1] as f32 * scale).round().clamp(0.0, 255.0) as u8;
                    let nbb = (n[2] as f32 * scale).round().clamp(0.0, 255.0) as u8;
                    img.put_pixel(nx as u32, ny as u32, image::Rgba([nr, ng, nbb, n[3]]));
                    changed = true;
                }
            }

            if out[0] != r || out[1] != g || out[2] != b || out[3] != a {
                img.put_pixel(x, y, image::Rgba(out));
            }

            changed
        }

        /// 跑完整张图（遍历顺序与历史实现一致：x 外层、y 内层）。
        pub(crate) fn fix_image(img: &mut RgbaImage) -> bool {
            let (width, height) = (img.width(), img.height());
            let mut changed = false;
            for x in 0..width {
                for y in 0..height {
                    if fix_pixel(img, x, y, width, height) {
                        changed = true;
                    }
                }
            }
            changed
        }

        pub fn run(tx: &mut Tx<'_>) -> Result<Outcome, AromError> {
            let paths = png_paths(tx, &ROOTS)?;
            let scanned = paths.len();
            let mut outcome = Outcome::default();
            for path in paths {
                let mut img = match tx.image(&path) {
                    Ok(img) => (*img).clone(),
                    Err(e) => {
                        outcome.skipped += 1;
                        outcome
                            .notes
                            .push(format!("{path}: 解码失败，跳过（{e}）"));
                        continue;
                    }
                };
                if !fix_image(&mut img) {
                    continue;
                }
                tx.put_image(&path, &img)?;
                outcome.changed += 1;
                outcome.notes.push(format!("{path}: 图层规则命中"));
            }
            if outcome.changed > 0 || outcome.skipped > 0 {
                crate::log_info!(
                    "fix_alpha_layers_in_textures: scanned={}, rewritten={}, skipped={}",
                    scanned,
                    outcome.changed,
                    outcome.skipped
                );
            }
            Ok(outcome)
        }
    }

    /// 任务名 → (声明, 实现)，供 `native_run::native_for` 派发。
    pub fn lookup(name: &str) -> Option<(TaskDecl, PilotFn)> {
        match name {
            "fix_mislabeled_textures" => Some((
                fix_mislabeled_textures::decl(),
                fix_mislabeled_textures::run,
            )),
            "fix_item_transparency" => Some((
                fix_item_transparency::decl(),
                fix_item_transparency::run,
            )),
            "fix_alpha_layers_in_textures" => Some((
                fix_alpha_layers_in_textures::decl(),
                fix_alpha_layers_in_textures::run,
            )),
            _ => None,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::arom::Pack;
        use crate::arom::source::MemSource;

        fn png_bytes(img: &RgbaImage) -> Vec<u8> {
            let mut buf = Vec::new();
            image::DynamicImage::ImageRgba8(img.clone())
                .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
                .expect("encode png");
            buf
        }

        /// 编码一张 RGB JPEG（image crate 的 JPEG 编码器不支持 alpha，必须先转 RGB8）。
        fn jpeg_bytes(size: (u32, u32), color: [u8; 3]) -> Vec<u8> {
            let img = image::RgbImage::from_pixel(size.0, size.1, image::Rgb(color));
            let mut buf = Vec::new();
            image::DynamicImage::ImageRgb8(img)
                .write_to(
                    &mut std::io::Cursor::new(&mut buf),
                    image::ImageFormat::Jpeg,
                )
                .expect("encode jpeg");
            buf
        }

        /// 造一个内存包（`pack.mcmeta` 写在 format 34，任务不依赖它）。
        fn pack_with(files: Vec<(&str, Vec<u8>)>) -> Pack {
            let mut all: Vec<(String, Vec<u8>)> = vec![(
                "pack.mcmeta".to_string(),
                br#"{"pack":{"pack_format":34}}"#.to_vec(),
            )];
            all.extend(files.into_iter().map(|(p, b)| (p.to_string(), b)));
            let src = MemSource::new(all).expect("mem source");
            Pack::from_source(Box::new(src), None).expect("pack")
        }

        fn run_task(pack: &mut Pack, task: PilotFn) -> Outcome {
            let mut tx = pack.tx("hygiene-test");
            let outcome = task(&mut tx).expect("run task");
            let layer = tx.into_layer();
            pack.commit(layer);
            outcome
        }

        fn raw(pack: &Pack, path: &str) -> Vec<u8> {
            pack.view()
                .read(path)
                .expect("read raw")
                .unwrap_or_else(|| panic!("missing `{path}`"))
        }

        fn decoded(pack: &Pack, path: &str) -> RgbaImage {
            (*pack.view().image(path).expect("decode")).clone()
        }

        // ══════════════════════════════════════════════════════════════════
        // A1：fix_mislabeled_textures
        // ══════════════════════════════════════════════════════════════════

        /// 正例：JPEG 内容 + `.png` 名字 → 重编码为真 PNG；原始字节接口读到的
        /// **确实是 JPEG 字节**（不是解码后的像素），这正是 A1 判断的依据。
        #[test]
        fn a1_rewrites_jpeg_content_to_real_png() {
            let jpeg = jpeg_bytes((8, 8), [200, 30, 30]);
            assert_eq!(&jpeg[..3], &[0xFF, 0xD8, 0xFF], "JPEG 编码器应输出 SOI magic");
            let path = "assets/minecraft/textures/item/mislabeled.png";
            let mut pack = pack_with(vec![(path, jpeg.clone())]);

            // 原始字节接口：A1 依赖它先看 magic；必须原样返回（不解码、不改写）。
            assert_eq!(raw(&pack, path), jpeg, "Tx/PackView::read 必须给原始字节");

            let outcome = run_task(&mut pack, fix_mislabeled_textures::run);
            assert_eq!(outcome.changed, 1);
            let rewritten = raw(&pack, path);
            assert_eq!(
                &rewritten[..4],
                &[0x89, 0x50, 0x4E, 0x47],
                "写回后必须是真 PNG（PNG magic）"
            );
            let img = decoded(&pack, path);
            assert_eq!(img.dimensions(), (8, 8), "尺寸必须保持");
            let p = img.get_pixel(0, 0);
            assert!(
                p[0] > 150 && p[1] < 90 && p[2] < 90,
                "像素内容必须来自 JPEG 的解码结果（允许有损误差），实际 {p:?}"
            );
        }

        /// 反例：正常 PNG **逐字节不动**——这是"默认产物不变"的核心保证。
        #[test]
        fn a1_leaves_real_png_byte_identical() {
            let img = image::RgbaImage::from_pixel(5, 7, image::Rgba([9, 8, 7, 255]));
            let png = png_bytes(&img);
            let path = "assets/minecraft/textures/item/normal.png";
            let mut pack = pack_with(vec![(path, png.clone())]);

            let outcome = run_task(&mut pack, fix_mislabeled_textures::run);
            assert_eq!(outcome.changed, 0, "正常 PNG 不该被改写");
            assert_eq!(outcome.skipped, 0);
            assert_eq!(raw(&pack, path), png, "正常 PNG 必须逐字节不动");
        }

        /// 边界：有 JPEG magic 但内容损坏 → 不写、不报错，记 skipped + note。
        #[test]
        fn a1_skips_corrupt_jpeg_without_writing() {
            let corrupt = vec![0xFF, 0xD8, 0xFF, 0x00, 0x11, 0x22, 0x33];
            let path = "assets/minecraft/textures/block/broken.png";
            let mut pack = pack_with(vec![(path, corrupt.clone())]);

            let outcome = run_task(&mut pack, fix_mislabeled_textures::run);
            assert_eq!(outcome.changed, 0, "坏文件不该被写成一个坏 PNG");
            assert_eq!(outcome.skipped, 1);
            assert!(!outcome.notes.is_empty(), "跳过必须有 note 可查");
            assert_eq!(raw(&pack, path), corrupt, "坏文件保持原样");
        }

        /// 范围：只扫 `assets/minecraft/textures`；其它扩展名的 JPEG 不碰。
        #[test]
        fn a1_ignores_non_png_and_out_of_scope_files() {
            let jpeg = jpeg_bytes((4, 4), [0, 200, 0]);
            let mut pack = pack_with(vec![
                // .png 之外不扫描
                ("assets/minecraft/textures/item/photo.jpg", jpeg.clone()),
                // 不在 textures 树下（根 pack.png）不扫描
                ("pack.png", jpeg.clone()),
            ]);
            let outcome = run_task(&mut pack, fix_mislabeled_textures::run);
            assert_eq!(outcome.changed, 0);
            assert_eq!(
                raw(&pack, "assets/minecraft/textures/item/photo.jpg"),
                jpeg
            );
            assert_eq!(raw(&pack, "pack.png"), jpeg);
        }

        // ══════════════════════════════════════════════════════════════════
        // A2：fix_item_transparency
        // ══════════════════════════════════════════════════════════════════

        /// 构造 `16×16` 图：左半 `fill` 色、右半不透明绿。
        fn fake_background_png(alpha: u8, rgb: [u8; 3]) -> Vec<u8> {
            image::RgbaImage::from_fn(16, 16, |x, _| {
                if x < 8 {
                    image::Rgba([rgb[0], rgb[1], rgb[2], alpha])
                } else {
                    image::Rgba([0, 255, 0, 255])
                }
            })
            .pipe_png()
        }

        /// 给小测试用的 PNG 编码扩展（避免每个用例都写一遍 Cursor）。
        trait PipePng {
            fn pipe_png(self) -> Vec<u8>;
        }

        impl PipePng for RgbaImage {
            fn pipe_png(self) -> Vec<u8> {
                png_bytes(&self)
            }
        }

        /// 正例：左半 alpha=2 假背景（8×16=128px ≥ max(64,5%)）→ 清成 (0,0,0,0)，
        /// 右半不透明像素保持原样。
        #[test]
        fn a2_clears_edge_fake_transparency() {
            let path = "assets/minecraft/textures/item/fake.png";
            let mut pack = pack_with(vec![(path, fake_background_png(2, [200, 100, 50]))]);
            let outcome = run_task(&mut pack, fix_item_transparency::run);
            assert_eq!(outcome.changed, 1);
            let img = decoded(&pack, path);
            for y in 0..16 {
                for x in 0..8 {
                    assert_eq!(
                        *img.get_pixel(x, y),
                        image::Rgba([0, 0, 0, 0]),
                        "假背景像素 ({x},{y}) 必须清成真透明"
                    );
                }
                assert_eq!(
                    *img.get_pixel(12, y),
                    image::Rgba([0, 255, 0, 255]),
                    "不透明区域必须原样保留"
                );
            }
        }

        /// 反例：内部（不触边）的近透明块不清——那是作者画的内容。
        #[test]
        fn a2_keeps_interior_near_transparent_block() {
            let png = image::RgbaImage::from_fn(16, 16, |x, y| {
                if (4..12).contains(&x) && (4..12).contains(&y) {
                    image::Rgba([0, 0, 0, 2])
                } else {
                    image::Rgba([0, 255, 0, 255])
                }
            })
            .pipe_png();
            let path = "assets/minecraft/textures/item/hole.png";
            let mut pack = pack_with(vec![(path, png.clone())]);
            let outcome = run_task(&mut pack, fix_item_transparency::run);
            assert_eq!(outcome.changed, 0, "内部透明块不触边，不该清");
            assert_eq!(raw(&pack, path), png, "无改动时必须逐字节不动");
        }

        /// 反例：正常 alpha=0 背景（旧包常见）不清、不重写——否则默认 1→N 转换会
        /// 无意义地改变所有正常包的字节。
        #[test]
        fn a2_keeps_normal_alpha_zero_background_untouched() {
            let png = image::RgbaImage::from_fn(16, 16, |x, _| {
                if x < 8 {
                    image::Rgba([0, 0, 0, 0])
                } else {
                    image::Rgba([10, 20, 30, 255])
                }
            })
            .pipe_png();
            let path = "assets/minecraft/textures/item/normal_old.png";
            let mut pack = pack_with(vec![(path, png.clone())]);
            let outcome = run_task(&mut pack, fix_item_transparency::run);
            assert_eq!(outcome.changed, 0);
            assert_eq!(raw(&pack, path), png, "纯 alpha=0 背景必须原样保留");
        }

        /// 边界：面积门槛 = max(64, ceil(5%))。
        /// 32×32 共 1024px → 5%=52、max=64：32px 高的一块不够，64px 才够。
        #[test]
        fn a2_area_threshold_boundary() {
            let make = |rows: u32| {
                image::RgbaImage::from_fn(32, 32, |_, y| {
                    if y < rows {
                        image::Rgba([100, 100, 100, 1])
                    } else {
                        image::Rgba([0, 255, 0, 255])
                    }
                })
                .pipe_png()
            };

            // 1 行 × 32 = 32px < 64 → 不清
            let small = make(1);
            let path = "assets/minecraft/textures/items/small.png";
            let mut pack = pack_with(vec![(path, small.clone())]);
            let outcome = run_task(&mut pack, fix_item_transparency::run);
            assert_eq!(outcome.changed, 0, "低于面积门槛的区域不该清");
            assert_eq!(raw(&pack, path), small);

            // 2 行 × 32 = 64px ≥ 64 → 清
            let enough = make(2);
            let path = "assets/minecraft/textures/items/enough.png";
            let mut pack = pack_with(vec![(path, enough)]);
            let outcome = run_task(&mut pack, fix_item_transparency::run);
            assert_eq!(outcome.changed, 1, "达到面积门槛的区域必须清");
            let img = decoded(&pack, path);
            assert_eq!(*img.get_pixel(0, 0), image::Rgba([0, 0, 0, 0]));
        }

        /// 目录覆盖：旧路径 `items/` 与新路径 `item/` 都要扫（1.6–1.8 → 1.9+ 时可能同时存在）。
        #[test]
        fn a2_scans_both_legacy_and_modern_item_dirs() {
            let png = fake_background_png(1, [0, 0, 0]);
            let mut pack = pack_with(vec![
                ("assets/minecraft/textures/items/old.png", png.clone()),
                ("assets/minecraft/textures/item/new.png", png.clone()),
            ]);
            let outcome = run_task(&mut pack, fix_item_transparency::run);
            assert_eq!(outcome.changed, 2, "items/ 与 item/ 两份都要处理");
        }

        // ══════════════════════════════════════════════════════════════════
        // A7：fix_alpha_layers_in_textures（历史 4 条 + 新增 2 条 + 端到端 1 条）
        // ══════════════════════════════════════════════════════════════════

        fn img2(pixels: [[u8; 4]; 4]) -> RgbaImage {
            let mut img = RgbaImage::new(2, 2);
            for (i, p) in pixels.iter().enumerate() {
                img.put_pixel((i % 2) as u32, (i / 2) as u32, image::Rgba(*p));
            }
            img
        }

        #[test]
        fn rule1_wipes_rgb_on_zero_alpha() {
            let mut img = img2([
                [10, 20, 30, 0],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
            ]);
            assert!(fix_alpha_layers_in_textures::fix_pixel(&mut img, 0, 0, 2, 2));
            assert_eq!(*img.get_pixel(0, 0), image::Rgba([0, 0, 0, 0]));
        }

        #[test]
        fn rule2_brightens_zero_rgb_semi_transparent() {
            // a=128 → gray = round(128*0.8) = 102
            let mut img = img2([
                [0, 0, 0, 128],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
            ]);
            assert!(fix_alpha_layers_in_textures::fix_pixel(&mut img, 0, 0, 2, 2));
            assert_eq!(*img.get_pixel(0, 0), image::Rgba([102, 102, 102, 128]));
        }

        #[test]
        fn rule5_rescales_semi_transparent_brightness() {
            // a=100 → expected 80；当前 brightness=200，差 120 > 50
            let mut img = img2([
                [200, 200, 200, 100],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
            ]);
            assert!(fix_alpha_layers_in_textures::fix_pixel(&mut img, 0, 0, 2, 2));
            let p = img.get_pixel(0, 0);
            let br = (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0;
            assert!((br - 80.0).abs() < 1.5, "brightness {br} 应接近 80");
            assert_eq!(p[3], 100);
        }

        #[test]
        fn rule6_blends_high_std_dev_opaque_pixel() {
            // [255,0,0,255]：avg=85，std≈120.2>80 → 向灰混合 30%
            // r=round(255*0.7+85*0.3)=204，g/b=round(85*0.3)=26
            let mut img = img2([
                [255, 0, 0, 255],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
            ]);
            assert!(fix_alpha_layers_in_textures::fix_pixel(&mut img, 0, 0, 2, 2));
            assert_eq!(*img.get_pixel(0, 0), image::Rgba([204, 26, 26, 255]));
        }

        #[test]
        fn rule7_fixes_neighbor_rgb_not_current() {
            // 当前 (0,0) 不透明亮色；邻居 (1,0) 半透明且过暗
            let mut img = img2([
                [220, 220, 220, 255],
                [10, 10, 10, 128],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
            ]);
            assert!(fix_alpha_layers_in_textures::fix_pixel(&mut img, 0, 0, 2, 2));
            let n = img.get_pixel(1, 0);
            let br = (n[0] as f32 + n[1] as f32 + n[2] as f32) / 3.0;
            let expected = 128.0 * 0.8;
            assert!(
                (br - expected).abs() < 1.5,
                "邻居亮度 {br} 应接近 {expected}"
            );
            // 当前像素不应被改写成邻居的亮度
            assert_eq!(*img.get_pixel(0, 0), image::Rgba([220, 220, 220, 255]));
        }

        #[test]
        fn rule8_lifts_pure_black_opaque_pixel() {
            // 纯黑不透明像素被 4 个亮不透明邻居包围（其余像素不会触发任何规则）
            let mut img = RgbaImage::from_pixel(3, 3, image::Rgba([200, 200, 200, 255]));
            img.put_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
            assert!(fix_alpha_layers_in_textures::fix_pixel(&mut img, 1, 1, 3, 3));
            assert_eq!(*img.get_pixel(1, 1), image::Rgba([200, 200, 200, 255]));
        }

        #[test]
        fn rule9_desaturates_oversaturated_semi_transparent() {
            // a=128, expected_sat≈0.5；当前饱和度 1.0
            let mut img = img2([
                [200, 0, 0, 128],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
            ]);
            assert!(fix_alpha_layers_in_textures::fix_pixel(&mut img, 0, 0, 2, 2));
            let p = img.get_pixel(0, 0);
            let maxc = p[0].max(p[1]).max(p[2]) as f32;
            let minc = p[0].min(p[1]).min(p[2]) as f32;
            let sat = if maxc > 0.0 {
                (maxc - minc) / maxc
            } else {
                0.0
            };
            assert!((sat - 0.5).abs() < 0.15, "饱和度 {sat} 应接近 0.5");
            assert_eq!(p[3], 128);
        }

        /// 端到端：run() 只写"确实命中规则"的图，未命中的图**逐字节不动**。
        #[test]
        fn a7_run_rewrites_only_changed_images() {
            let hit = img2([
                [10, 20, 30, 0],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
            ])
            .pipe_png();
            let clean = image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255])).pipe_png();
            let mut pack = pack_with(vec![
                ("assets/minecraft/textures/gui/hit.png", hit),
                ("assets/minecraft/textures/gui/clean.png", clean.clone()),
            ]);
            let outcome = run_task(&mut pack, fix_alpha_layers_in_textures::run);
            assert_eq!(outcome.changed, 1, "只有命中的图该被重写");
            assert_eq!(
                raw(&pack, "assets/minecraft/textures/gui/clean.png"),
                clean,
                "未命中的图必须逐字节不动"
            );
            let img = decoded(&pack, "assets/minecraft/textures/gui/hit.png");
            assert_eq!(*img.get_pixel(0, 0), image::Rgba([0, 0, 0, 0]));
        }
    }
