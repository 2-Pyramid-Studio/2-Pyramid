//! **旧原版模型 / blockstate 指纹停用**（任务名 `drop_unmodified_legacy_models`）。
//!
//! ## 解决什么问题
//!
//! 1.9–1.12（pack_format 2/3）时代的资源包，经常把**整套原版 `models/**` 与
//! `blockstates/**` JSON** 原样打进包里（做材质包时"复制一份官方资源再改贴图"是常规做法）。
//! 当这种包被升级到 1.13+（format ≥4）时，包内那些**没改过的旧版原版 JSON** 会覆盖
//! 目标版本自己的原生定义：
//!
//! * 1.13 起 `blockstates` 的 `variants` 语义/属性名变了（如 `facing`/`axis` 的取值集合），
//!   旧 JSON 引用的模型路径在 1.13 之后可能已重命名或结构不同 → **紫黑方块**；
//! * 旧 `models/block/*` 的 `parent` 链/元素旋转在新版里几何含义变化 → **错误几何**；
//! * `models/item/*` 的 `display`（手持/掉落姿态）在新版被重定标 → **手持大小异常**。
//!
//! 修法只有一条：**停用（删除）未修改的旧原版 JSON**，让目标版本用自己 jar 里的定义；
//! 同时必须**保留作者改过的**（改了贴图引用/几何的自定义模型）。
//!
//! ## 怎么识别"未修改的旧原版"
//!
//! 数据表 `legacy-defaults.json`（与数据队友/`tools/gen-mc-reference.py` 的契约，编译期
//! `include_str!` 内嵌）：
//!
//! ```text
//! legacy["2"|"3"][ "models"|"blockstates" ][ 包根相对路径 ] = [ 规范化 JSON 的 sha256, ... ]
//! target_missing[目标 format] = { "models": [...], "blockstates": [...] }
//! ```
//!
//! **路径键是包根相对路径，不带 `assets/minecraft/` 前缀**（例如 `models/block/cube_all.json`、
//! `blockstates/stone.json`）——数据生成器（`tools/gen-mc-reference.py`）按 jar 内路径写表；
//! 而 `run` 从 `tx.list` 拿到的是**包内全路径**（`assets/minecraft/models/...`），因此查表前
//! 必须 `strip_prefix("assets/minecraft/")` 换算（见 `run_with_tables`）。
//!
//! 对包内每个 `models/**`/`blockstates/**` 的 `.json`：算规范化 JSON 的 sha256；
//! 若命中 `legacy["2"]` **或** `legacy["3"]` 的该路径允许列表 → 判定"未修改的旧原版"。
//! **两组都查**：任务不需要知道源包到底是 format 2 还是 3（注入条件已保证源是 2 或 3），
//! 两组合并命中是安全的——同一路径在两组里都出现时 hash 不同，只有真正匹配的那组会命中。
//!
//! ## 为什么还要 `target_missing`
//!
//! 如果目标版本的原生 jar 里**没有**这个路径（版本间模型重命名/删除），直接删掉它不是
//! "回退到原生"，而是"让引用的模型彻底消失"（同样会紫黑）。这种路径必须**保留**并记 note，
//! 把缺失暴露出来，而不是用删除制造第二个故障。
//! 数据是**按 kind 分开**的（`models` / `blockstates` 两个数组）：同一个相对路径字符串在两
//! 个 kind 下语义不同，命中判断按文件所属 kind 查对应数组（缺失 → 保留）。
//!
//! ## 规范化（canonicalization）契约——必须与 Python 生成器逐字一致
//!
//! 1. 去 UTF-8 BOM；
//! 2. 解析为 `serde_json::Value`；
//! 3. 递归输出：对象 key 按 **Unicode 码点升序**（UTF-8 字节序等价），分隔符 `,`/`:`，
//!    **无任何空白**；字符串用 serde_json 转义（非 ASCII 不转义）；数字保留 int/float
//!    类型（`Number::to_string()`，即 `1` 仍是 `1`、`1.0` 仍是 `1.0`）；数组保序；
//!    `true/false/null` 小写；
//! 4. 取 UTF-8 字节的 sha256 十六进制小写。
//!
//! 跨语言一致性由 `legacy-defaults.fixtures.json` 的夹具证明（Python 生成器写 canonical+sha256，
//! Rust 这里重算并逐条比对）。夹具文件是**数据队友拥有**的，本模块只读。
//!
//! ## 触发条件（由 `native_run::inject_legacy_models_task` 注入）
//!
//! `source_version ∈ {2,3}` 且 `target_version >= 4` 且 `target_version != 1000`。
//! 任务本身再从 `pack.mcmeta` 读目标格式做二次防御：`target < 4 || target == 1000` 直接 no-op。
    use super::*;

    use std::collections::{BTreeSet, HashMap};
    use std::sync::OnceLock;

    /// 任务名（计划里与 `task_registry` 里的唯一标识）。
    pub const TASK_NAME: &str = "drop_unmodified_legacy_models";

    /// 要清扫的两个根（读=写）。
    const MODELS: &str = "assets/minecraft/models";
    const BLOCKSTATES: &str = "assets/minecraft/blockstates";

    /// `Outcome.notes` 的样本上限：真实包里可能命中几百个文件，全量塞进报告会爆炸。
    const NOTES_LIMIT: usize = 20;

    pub fn decl() -> TaskDecl {
        let scope = ScopeSet::prefix(MODELS).union(&ScopeSet::prefix(BLOCKSTATES));
        TaskDecl::new(TASK_NAME, Tier::Eraser)
            .reads(scope.clone())
            .writes(scope)
            .exclusive(true)
    }

    /// UTF-8 BOM。生成器（Python）读文件时同样先剥掉它；否则同一份 JSON 在"带 BOM/不带 BOM"
    /// 两种写法下会算出两个 hash，指纹表就永远匹配不上。
    const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

    fn strip_bom(bytes: &[u8]) -> &[u8] {
        if bytes.starts_with(UTF8_BOM) {
            &bytes[UTF8_BOM.len()..]
        } else {
            bytes
        }
    }

    /// 按模块文档的规则做规范化序列化；返回 `Err` 表示"不是合法 JSON"。
    ///
    /// 调用方把 `Err` 一律当"保留"：自定义/损坏文件不动，是数据安全的第一原则。
    pub(crate) fn canonical_json(bytes: &[u8]) -> Result<String, String> {
        let value: serde_json::Value = serde_json::from_slice(strip_bom(bytes))
            .map_err(|e| format!("invalid json: {e}"))?;
        let mut out = String::new();
        write_canonical(&value, &mut out);
        Ok(out)
    }

    fn write_canonical(value: &serde_json::Value, out: &mut String) {
        use serde_json::Value;
        match value {
            Value::Null => out.push_str("null"),
            Value::Bool(true) => out.push_str("true"),
            Value::Bool(false) => out.push_str("false"),
            // `Number::to_string` 保留原始 JSON 类型：整数仍是整数、浮点仍是浮点。
            Value::Number(n) => out.push_str(&n.to_string()),
            // 字符串用 serde_json 的转义规则（与 Python `json.dumps(ensure_ascii=False)` 一致：
            // 引号/反斜杠/控制字符转义，中文等非 ASCII 原样保留）。
            Value::String(s) => {
                out.push_str(&serde_json::to_string(s).expect("string 序列化不可失败"));
            }
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_canonical(item, out);
                }
                out.push(']');
            }
            Value::Object(map) => {
                // **显式排序**，不依赖 serde_json 的 Map 实现（默认 BTreeMap，但开 `preserve_order`
                // feature 后会变成插入序）。Unicode 码点序 == UTF-8 字节序，`String::cmp` 即可。
                let mut entries: Vec<(&String, &Value)> = map.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                out.push('{');
                for (i, (key, child)) in entries.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(key).expect("key 序列化不可失败"));
                    out.push(':');
                    write_canonical(child, out);
                }
                out.push('}');
            }
        }
    }

    /// sha256 十六进制（小写）。
    pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    /// 原始字节 → 规范化 JSON → sha256；任一环节失败即 `Err`（调用方保留文件）。
    pub(crate) fn canonical_sha256(bytes: &[u8]) -> Result<String, String> {
        Ok(sha256_hex(canonical_json(bytes)?.as_bytes()))
    }

    /// `legacy-defaults.json` 的解析结果。
    ///
    /// 只保留任务用得到的两种数据；`source.jars` 等元信息忽略（它们是给数据生成/审计看的）。
    #[derive(Debug, Default)]
    pub(crate) struct Tables {
        /// group("2"/"3") → 包根相对路径 → 允许的 sha256 列表。
        ///
        /// `models` 与 `blockstates` 合并进同一张路径表：路径本身带完整相对目录
        /// （`models/...` / `blockstates/...`），跨 kind 不会撞键。
        legacy: HashMap<String, HashMap<String, Vec<String>>>,
        /// 目标 format → kind("models"/"blockstates") → 目标原生 jar 里**不存在**的相对路径集合。
        target_missing: HashMap<u32, HashMap<String, BTreeSet<String>>>,
    }

    impl Tables {
        /// 从 JSON 文本解析。测试用它注入合成小表；生产用编译期内嵌的正式表。
        ///
        /// 解析**严格但容错字段扩展**：未知顶层字段忽略（数据生成器可以加注释/manifest），
        /// 但已知字段形状不对就报错——宁可让数据问题在启动时暴露，也不要静默"表是空的"。
        pub(crate) fn from_json_str(text: &str) -> Result<Self, String> {
            let root: serde_json::Value =
                serde_json::from_str(text).map_err(|e| format!("legacy-defaults.json: {e}"))?;
            let obj = root
                .as_object()
                .ok_or("legacy-defaults.json: 顶层必须是对象")?;

            let mut legacy: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
            if let Some(groups) = obj.get("legacy") {
                let groups = groups
                    .as_object()
                    .ok_or("legacy-defaults.json: `legacy` 必须是对象")?;
                for (group, kinds) in groups {
                    let kinds = kinds.as_object().ok_or_else(|| {
                        format!("legacy-defaults.json: `legacy.{group}` 必须是对象")
                    })?;
                    let mut by_path: HashMap<String, Vec<String>> = HashMap::new();
                    for (kind, paths) in kinds {
                        let paths = paths.as_object().ok_or_else(|| {
                            format!("legacy-defaults.json: `legacy.{group}.{kind}` 必须是对象")
                        })?;
                        for (path, hashes) in paths {
                            let hashes = hashes.as_array().ok_or_else(|| {
                                format!(
                                    "legacy-defaults.json: `legacy.{group}.{kind}.{path}` 必须是数组"
                                )
                            })?;
                            let mut list = Vec::with_capacity(hashes.len());
                            for h in hashes {
                                let h = h.as_str().ok_or_else(|| {
                                    format!(
                                        "legacy-defaults.json: `legacy.{group}.{kind}.{path}` 的 sha256 必须是字符串"
                                    )
                                })?;
                                list.push(h.to_ascii_lowercase());
                            }
                            by_path.entry(path.clone()).or_default().extend(list);
                        }
                    }
                    legacy.insert(group.clone(), by_path);
                }
            }

            let mut target_missing: HashMap<u32, HashMap<String, BTreeSet<String>>> = HashMap::new();
            if let Some(map) = obj.get("target_missing") {
                let map = map
                    .as_object()
                    .ok_or("legacy-defaults.json: `target_missing` 必须是对象")?;
                for (format, kinds) in map {
                    let format: u32 = format.parse().map_err(|_| {
                        format!("legacy-defaults.json: target_missing 的键 `{format}` 不是数字")
                    })?;
                    // 真实数据（task-11 契约）是**按 kind 嵌套**的：
                    //   target_missing["34"] = { "models": [...], "blockstates": [...] }
                    // 一个相对路径字符串在 models/blockstates 下语义不同，不能把两个数组
                    // 糊成一个集合（否则 models 的清单会误保护同名 blockstates 路径）。
                    let kinds = kinds.as_object().ok_or_else(|| {
                        format!(
                            "legacy-defaults.json: `target_missing.{format}` 必须是对象（kind → 数组）"
                        )
                    })?;
                    let mut by_kind: HashMap<String, BTreeSet<String>> = HashMap::new();
                    for (kind, paths) in kinds {
                        let paths = paths.as_array().ok_or_else(|| {
                            format!(
                                "legacy-defaults.json: `target_missing.{format}.{kind}` 必须是数组"
                            )
                        })?;
                        let mut set = BTreeSet::new();
                        for path in paths {
                            let path = path.as_str().ok_or_else(|| {
                                format!(
                                    "legacy-defaults.json: `target_missing.{format}.{kind}` 里必须全是字符串"
                                )
                            })?;
                            set.insert(path.to_string());
                        }
                        by_kind.insert(kind.clone(), set);
                    }
                    target_missing.insert(format, by_kind);
                }
            }

            Ok(Self {
                legacy,
                target_missing,
            })
        }

        /// 该**包根相对路径**（`models/...` / `blockstates/...`，不带 `assets/minecraft/`）
        /// + 规范化 sha256 是否被登记为"未修改的旧原版"（group 2 与 group 3 都查）。
        pub(crate) fn is_known_unmodified(&self, root_relative_path: &str, sha: &str) -> bool {
            ["2", "3"].iter().any(|group| {
                self.legacy
                    .get(*group)
                    .and_then(|by_path| by_path.get(root_relative_path))
                    .is_some_and(|hashes| hashes.iter().any(|h| h == sha))
            })
        }

        /// 该路径在该目标格式的原生 jar 里是否**不存在**（= 不能删，删了就没有替代品）。
        ///
        /// kind 由相对路径的第一个目录段决定（`models/` → models 清单、`blockstates/` →
        /// blockstates 清单）；其它前缀返回 `false`（不认识的路径不参与保留判定）。
        pub(crate) fn is_target_missing(&self, target: u32, root_relative_path: &str) -> bool {
            let kind = if root_relative_path.starts_with("models/") {
                "models"
            } else if root_relative_path.starts_with("blockstates/") {
                "blockstates"
            } else {
                return false;
            };
            self.target_missing
                .get(&target)
                .and_then(|by_kind| by_kind.get(kind))
                .is_some_and(|set| set.contains(root_relative_path))
        }

        /// 数据表是否覆盖该目标格式（`target_missing` 有该档的键）。
        ///
        /// **全新目标格式的 fail-safe**：若未来新增了一个 Java 目标 format 而数据表尚未
        /// 重新生成，`is_target_missing` 会对任何路径都返回 `false`（查不到 = 当作"目标有
        /// 原生替代"），从而把旧原版 JSON 全删掉——这是 fail-open，方向错了。调用方在
        /// 删除前必须先问这个方法；没有该档数据时**保守不删**并记 warn/note。当前 23 个
        /// Java 目标格式（4–97）全部有键，此分支只在数据过期时触发。
        pub(crate) fn has_target_data(&self, target: u32) -> bool {
            self.target_missing.contains_key(&target)
        }

        /// 占位期（数据未填）时为空表：所有文件都会被当成"自定义"保留 → 默认产物不变。
        pub(crate) fn is_empty(&self) -> bool {
            self.legacy.values().all(|m| m.is_empty()) && self.target_missing.is_empty()
        }
    }

    /// 编译期内嵌的正式数据表（占位期为空表；真实数据由数据队友填）。
    const EMBEDDED_TABLES: &str = include_str!("legacy-defaults.json");

    static TABLES: OnceLock<Result<Tables, String>> = OnceLock::new();

    /// 解析并缓存内嵌表。数据文件损坏时**不 panic**（它是构建产物的一部分，坏了也必须能转换），
    /// 但要让日志说清楚"这个任务为什么不生效"。
    fn embedded_tables() -> Option<&'static Tables> {
        let parsed = TABLES.get_or_init(|| match Tables::from_json_str(EMBEDDED_TABLES) {
            Ok(tables) => Ok(tables),
            Err(e) => {
                crate::log_error!(
                    "legacy-defaults.json 解析失败，drop_unmodified_legacy_models 不生效：{e}"
                );
                Err(e)
            }
        });
        match parsed {
            Ok(tables) => Some(tables),
            Err(_) => None,
        }
    }

    pub fn run(tx: &mut Tx<'_>) -> Result<Outcome, AromError> {
        let Some(tables) = embedded_tables() else {
            return Ok(Outcome {
                skipped: 1,
                notes: vec!["legacy-defaults 数据表不可用，整任务跳过".to_string()],
                ..Outcome::default()
            });
        };
        // 占位期（正式表为空）直接 no-op：不为一张空表去扫几百个 models/*.json。
        // 数据填充后自动生效，无需改代码。
        if tables.is_empty() {
            return Ok(Outcome::default());
        }
        run_with_tables(tx, tables)
    }

    /// 执行体（可注入数据表，供单测/夹具使用；生产路径与它共用同一份逻辑）。
    pub(crate) fn run_with_tables(tx: &mut Tx<'_>, tables: &Tables) -> Result<Outcome, AromError> {
        // 目标格式：`run_native` 在管线开头（§9.155）已把 pack.mcmeta 改写成目标格式，
        // 因此这里读到的一定是本次转换的目标。读不到就保守 no-op——不知道目标绝不删。
        let target = match tx.view().mcmeta().ok().and_then(|m| m.effective_format()) {
            Some(target) => target,
            None => {
                return Ok(Outcome {
                    notes: vec!["读不到 pack.mcmeta 的目标格式，跳过".to_string()],
                    ..Outcome::default()
                })
            }
        };
        // 二次防御（注入条件已保证，但任务可能被其它驱动直接调用）：
        // 1.13 以下没有"新版原生定义"可回退；Bedrock（1000）不是本任务的语义。
        if target < 4 || target == 1000 {
            return Ok(Outcome::default());
        }
        // **数据覆盖范围的 fail-safe**：目标格式没有 target_missing 键 = 数据表比转换器旧，
        // 此时任何"目标是否有原生替代"的判断都不可信；保守做法是一个文件都不删。
        if !tables.has_target_data(target) {
            crate::log_warn!(
                "drop_unmodified_legacy_models: target format {} 不在参考数据覆盖范围（target_missing 无该档），本次整任务保守跳过",
                target
            );
            return Ok(Outcome {
                skipped: 1,
                notes: vec![format!(
                    "目标 format {target} 无参考数据（target_missing 无该档），保守保留全部旧原版文件"
                )],
                ..Outcome::default()
            });
        }

        let mut paths: Vec<String> = Vec::new();
        for prefix in [MODELS, BLOCKSTATES] {
            for entry in tx.list(prefix)? {
                if entry.is_dir {
                    continue;
                }
                if entry.path.to_ascii_lowercase().ends_with(".json") {
                    paths.push(entry.path);
                }
            }
        }
        paths.sort();
        paths.dedup();

        let mut outcome = Outcome::default();
        let mut scanned = 0usize;
        let mut target_missing_kept = 0usize;
        for path in paths {
            scanned += 1;
            // **查表键是包根相对路径**（数据表由 jar 路径生成，不带 `assets/minecraft/`）。
            // tx 里的路径是包内全路径，先换算；换算失败（不在 assets/minecraft/ 下）则跳过。
            let Some(table_key) = path.strip_prefix("assets/minecraft/") else {
                continue;
            };
            // **原始字节**而不是 `tx.json()`：canonical 化要保留数字的原始 int/float 类型，
            // 而 Value 往返本来也能保留；这里统一走 `canonical_sha256`（它内部就是 Value 解析）。
            let Some(bytes) = tx.read(&path)? else {
                continue;
            };
            // 解析失败 / 非 JSON 内容 → 保留（自定义文件、损坏文件都不该被我们删）。
            let Ok(sha) = canonical_sha256(&bytes) else {
                continue;
            };
            if !tables.is_known_unmodified(table_key, &sha) {
                continue; // 作者改过（hash 不匹配）→ 保留
            }
            if tables.is_target_missing(target, table_key) {
                // 删了就没有原生替代 → 保留并记 note，把版本缺口暴露出来。
                target_missing_kept += 1;
                push_note(
                    &mut outcome,
                    format!("{path}: 目标 format {target} 无原生替代，保留"),
                );
                continue;
            }
            tx.remove(&path)?;
            outcome.changed += 1;
            push_note(&mut outcome, format!("{path}: 停用未修改的旧原版"));
        }

        crate::log_info!(
            "drop_unmodified_legacy_models: target={}, scanned={}, removed={}, target_missing_kept={}",
            target,
            scanned,
            outcome.changed,
            target_missing_kept
        );
        Ok(outcome)
    }

    /// 只记样本、不无限追加（`notes` 会进转换报告/界面，几百条会淹没其它信息）。
    fn push_note(outcome: &mut Outcome, note: String) {
        if outcome.notes.len() < NOTES_LIMIT {
            outcome.notes.push(note);
        }
    }

    /// 任务名 → (声明, 实现)，供 `native_run::native_for` 派发。
    pub fn lookup(name: &str) -> Option<(TaskDecl, PilotFn)> {
        if name == TASK_NAME {
            // 显式转 fn 指针：`Option::then` 的闭包返回类型是 fn item，与 `PilotFn` 不统一。
            Some((decl(), run as PilotFn))
        } else {
            None
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::arom::Pack;
        use crate::arom::source::MemSource;

        /// 造一个内存包（`pack.mcmeta` 的目标格式由调用方决定）。
        fn pack_with(format: u32, files: Vec<(&str, Vec<u8>)>) -> Pack {
            let mut all: Vec<(String, Vec<u8>)> = vec![(
                "pack.mcmeta".to_string(),
                format!(r#"{{"pack":{{"pack_format":{format}}}}}"#).into_bytes(),
            )];
            all.extend(files.into_iter().map(|(p, b)| (p.to_string(), b)));
            let src = MemSource::new(all).expect("mem source");
            Pack::from_source(Box::new(src), None).expect("pack")
        }

        fn run_on(pack: &mut Pack, tables: &Tables) -> Outcome {
            let mut tx = pack.tx("legacy-defaults-test");
            let outcome = run_with_tables(&mut tx, tables).expect("run");
            let layer = tx.into_layer();
            pack.commit(layer);
            outcome
        }

        /// 合成一张表：group 2 的 models 一条路径、group 3 的 blockstates 一条路径，
        /// 并给出它们规范化 JSON 的 sha256。
        ///
        /// **路径键一律是包根相对路径**（`models/...` / `blockstates/...`，与真实数据同契约）；
        /// 包内全路径（`assets/minecraft/...`）只在 `pack_with` 造包时使用，由 `run_with_tables`
        /// 负责 `strip_prefix` 换算。
        ///
        /// 手工建 `serde_json::Map` 而不是 `json!({ path: ... })`：`json!` 的对象键必须是
        /// 字符串字面量，动态路径要自己插进 Map。
        fn synthetic_tables(
            path2: (&str, &[u8]),
            path3: (&str, &[u8]),
            target_missing: &[(u32, &str, &[&str])],
        ) -> Tables {
            let sha2 = canonical_sha256(path2.1).expect("sha2");
            let sha3 = canonical_sha256(path3.1).expect("sha3");
            let mut g2_models = serde_json::Map::new();
            g2_models.insert(path2.0.to_string(), serde_json::json!([sha2]));
            let mut g2 = serde_json::Map::new();
            g2.insert("models".to_string(), serde_json::Value::Object(g2_models));
            let mut g3_blockstates = serde_json::Map::new();
            g3_blockstates.insert(path3.0.to_string(), serde_json::json!([sha3]));
            let mut g3 = serde_json::Map::new();
            g3.insert(
                "blockstates".to_string(),
                serde_json::Value::Object(g3_blockstates),
            );
            let mut legacy = serde_json::Map::new();
            legacy.insert("2".to_string(), serde_json::Value::Object(g2));
            legacy.insert("3".to_string(), serde_json::Value::Object(g3));
            // target_missing 按真实数据契约构造：format → { kind → [相对路径] }。
            let mut tm: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
            for (format, kind, paths) in target_missing {
                let entry = tm
                    .entry(format.to_string())
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                entry
                    .as_object_mut()
                    .expect("target_missing.[format] 必须是对象")
                    .insert(kind.to_string(), serde_json::json!(paths));
            }
            let root = serde_json::json!({
                "source": { "generator": "test" },
                "legacy": legacy,
                "target_missing": tm,
            });
            Tables::from_json_str(&root.to_string()).expect("synthetic tables")
        }

        // ══════════════════════════════════════════════════════════════════
        // canonicalization
        // ══════════════════════════════════════════════════════════════════

        /// 键序升序、无空白：空白/键序不同的两份 JSON 必须规范成同一串。
        #[test]
        fn canonicalization_sorts_keys_and_strips_whitespace() {
            let a = canonical_json(br#"{"b": 1, "a": { "d": true, "c": null }}"#).expect("a");
            let b = canonical_json(
                b"{\n  \"a\": {\"c\": null, \"d\": true},\n  \"b\": 1\n}\n",
            )
            .expect("b");
            assert_eq!(a, r#"{"a":{"c":null,"d":true},"b":1}"#);
            assert_eq!(a, b, "空白与键序不该影响规范化结果");
            // 同一份内容带 BOM 也等价（生成器/文本编辑器可能加 BOM）
            let mut with_bom = vec![0xEF, 0xBB, 0xBF];
            with_bom.extend_from_slice(br#"{"b":1,"a":{"d":true,"c":null}}"#);
            assert_eq!(canonical_json(&with_bom).expect("bom"), a);
        }

        /// int / float / 科学计数法保持解析后的 JSON 类型；数组保序；空容器。
        #[test]
        fn canonicalization_preserves_number_types_and_arrays() {
            assert_eq!(
                canonical_json(br#"[1, 1.0, 1e3, 2.5, true, false, null, "x"]"#).expect("numbers"),
                r#"[1,1.0,1000.0,2.5,true,false,null,"x"]"#
            );
            assert_eq!(canonical_json(br#"{"a":[],"b":{}}"#).expect("empty"), r#"{"a":[],"b":{}}"#);
            // 数组顺序是语义的一部分，不能排序
            assert_eq!(canonical_json(br#"[3,1,2]"#).expect("array"), "[3,1,2]");
        }

        /// 字符串转义：引号/反斜杠/控制字符转义；非 ASCII（中文）**不**转义。
        #[test]
        fn canonicalization_string_escaping_matches_python() {
            let json = "{\"zh\":\"中文\",\"quote\":\"a\\\"b\",\"nl\":\"a\\nb\",\"slash\":\"a/b\"}";
            let canonical = canonical_json(json.as_bytes()).expect("strings");
            assert_eq!(
                canonical,
                "{\"nl\":\"a\\nb\",\"quote\":\"a\\\"b\",\"slash\":\"a/b\",\"zh\":\"中文\"}"
            );
            assert!(!canonical.contains("\\u4e2d"), "非 ASCII 不许被转义");
        }

        /// **跨语言夹具**：Python 生成器（`tools/gen-mc-reference.py`）写的 `canonical` /
        /// `sha256` 与 Rust 实现**逐条比对**。
        ///
        /// 真实数据已就绪（11 条 synthetic），因此这里要求**非空且 ≥8 条**：这是本任务
        /// "canonicalization 与生成器逐字一致"的验收口径。夹具由数据队友拥有，本用例只读。
        #[test]
        fn canonicalization_matches_python_fixtures() {
            const FIXTURES: &str = include_str!("legacy-defaults.fixtures.json");
            let root: serde_json::Value =
                serde_json::from_str(FIXTURES).expect("fixtures json 可解析");
            // 兼容两种 schema：{"fixtures": [...]}（当前文件）与裸数组（早期描述）。
            let fixtures = root
                .get("fixtures")
                .and_then(|v| v.as_array())
                .or_else(|| root.as_array())
                .expect("fixtures 必须是数组（或 { fixtures: [...] }）");

            assert!(
                !fixtures.is_empty(),
                "legacy-defaults.fixtures.json 的跨语言夹具不能为空（真实数据已就绪）"
            );
            assert!(
                fixtures.len() >= 8,
                "夹具至少 8 条才算完成跨语言校验，实际 {} 条",
                fixtures.len()
            );
            for (i, fixture) in fixtures.iter().enumerate() {
                let json = fixture
                    .get("json")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("fixture[{i}] 缺 json 字符串字段"));
                let expected_canonical = fixture
                    .get("canonical")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("fixture[{i}] 缺 canonical 字符串字段"));
                let expected_sha = fixture
                    .get("sha256")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("fixture[{i}] 缺 sha256 字符串字段"));
                let canonical = canonical_json(json.as_bytes())
                    .unwrap_or_else(|e| panic!("fixture[{i}] 无法规范化：{e}（json={json}）"));
                assert_eq!(
                    canonical, expected_canonical,
                    "fixture[{i}]（{}）canonical 不一致（Python vs Rust）：json={json}",
                    fixture
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unnamed")
                );
                assert_eq!(
                    sha256_hex(canonical.as_bytes()),
                    expected_sha.to_ascii_lowercase(),
                    "fixture[{i}] sha256 不一致：json={json}"
                );
            }
        }

        // ══════════════════════════════════════════════════════════════════
        // 数据表解析
        // ══════════════════════════════════════════════════════════════════

        /// **正式数据表（真实数据，1MB / 27 jar）必须可解析且 schema 形状正确**；
        /// 合成表按 group/kind 正确合并。
        ///
        /// 真实数据契约（task-11）：`legacy["2"|"3"][kind][包根相对路径] = [sha256...]`；
        /// `target_missing[fmt] = {"models":[...], "blockstates":[...]}`（23 个目标档位）。
        #[test]
        fn tables_parse_embedded_and_synthetic_schema() {
            let embedded = Tables::from_json_str(EMBEDDED_TABLES).expect("正式表可解析");
            assert!(!embedded.is_empty(), "真实数据表不应为空");
            assert!(
                embedded.legacy.contains_key("2") && embedded.legacy.contains_key("3"),
                "legacy 必须同时有 group \"2\"（1.9–1.10）与 \"3\"（1.11–1.12）"
            );
            let target34 = embedded
                .target_missing
                .get(&34)
                .expect("target_missing 必须有格式 34 档");
            assert!(
                target34.contains_key("models") && target34.contains_key("blockstates"),
                "target_missing[34] 必须按 kind 分成 models / blockstates 两组"
            );
            eprintln!(
                "真实数据表：legacy[2]={} paths, legacy[3]={} paths, target_missing={} formats",
                embedded.legacy["2"].len(),
                embedded.legacy["3"].len(),
                embedded.target_missing.len()
            );

            // 合成表：路径键一律**包根相对**（与真实数据同契约）。
            let tables = synthetic_tables(
                (MODEL_KEY, br#"{"parent":"block/cube_all"}"#),
                (
                    BLOCKSTATE_KEY,
                    br#"{"variants":{"":{"model":"block/stone"}}}"#,
                ),
                &[(34, "models", &[MODEL_KEY])],
            );
            let sha = canonical_sha256(br#"{"parent":"block/cube_all"}"#).expect("sha");
            assert!(tables.is_known_unmodified(MODEL_KEY, &sha));
            assert!(tables.is_target_missing(34, MODEL_KEY));
            assert!(!tables.is_target_missing(46, MODEL_KEY), "46 档未登记 → 不保留");
            assert!(!tables.is_known_unmodified("models/block/dirt.json", &sha));
            // 带 `assets/minecraft/` 前缀的**包内路径**不是表键（由 run 负责 strip）：
            assert!(!tables.is_known_unmodified(MODEL_PATH, &sha));
        }

        // ══════════════════════════════════════════════════════════════════
        // 删除 / 保留 / no-op
        // ══════════════════════════════════════════════════════════════════

        /// 包根相对路径（**表键**，与数据契约一致）。
        const MODEL_KEY: &str = "models/block/stone.json";
        const BLOCKSTATE_KEY: &str = "blockstates/stone.json";
        /// 包内全路径（`tx.list` 返回的形状；`run_with_tables` 会 strip 成表键）。
        const MODEL_PATH: &str = "assets/minecraft/models/block/stone.json";
        const BLOCKSTATE_PATH: &str = "assets/minecraft/blockstates/stone.json";

        const LEGACY_MODEL: &[u8] = br#"{"parent": "block/cube_all", "textures": {"all": "blocks/stone"}}"#;
        const LEGACY_BLOCKSTATE: &[u8] =
            br#"{"variants": {"normal": {"model": "stone"}}}"#;

        /// 表里登记的是**相对键**；包里放的是**全路径**——正好覆盖生产路径的 strip 换算。
        ///
        /// `target_missing` 也必须带上目标 34 的两个 kind 键：生产表里每档都有键；缺键在
        /// 真实运行中会触发「数据未覆盖 → 保守跳过」的 fail-safe，测试要覆盖正常删除路径，
        /// 因此这里显式给 34 档空清单。
        fn legacy_tables() -> Tables {
            synthetic_tables(
                (MODEL_KEY, LEGACY_MODEL),
                (BLOCKSTATE_KEY, LEGACY_BLOCKSTATE),
                &[(34, "models", &[]), (34, "blockstates", &[])],
            )
        }

        /// 正例：group 2 的 models 命中 → 删除；作者改过的同路径 JSON 保留。
        #[test]
        fn removes_unmodified_legacy_model_and_keeps_custom() {
            let custom = br#"{"parent": "block/cube", "textures": {"all": "mypack:custom"}}"#;
            let mut pack = pack_with(
                34,
                vec![
                    (MODEL_PATH, LEGACY_MODEL.to_vec()),
                    ("assets/minecraft/models/block/dirt.json", custom.to_vec()),
                ],
            );
            let tables = legacy_tables();
            // 让 dirt 路径也登记一组旧原版 hash，但包内内容是作者改过的 → 保留。
            let tables = {
                let sha = canonical_sha256(br#"{"parent":"block/cube_all"}"#).expect("sha");
                let mut t = tables;
                t.legacy
                    .get_mut("2")
                    .expect("group2")
                    .insert("models/block/dirt.json".to_string(), vec![sha]);
                t
            };
            let outcome = run_on(&mut pack, &tables);
            assert_eq!(outcome.changed, 1, "只应删掉未修改的 stone：{outcome:?}");
            assert!(pack.view().resolve(MODEL_PATH).is_none(), "未修改的旧原版必须被停用");
            assert!(pack
                .view()
                .resolve("assets/minecraft/models/block/dirt.json")
                .is_some(), "作者改过的必须保留");
            assert_eq!(outcome.notes.len(), 1);
        }

        /// 正例：group 3 的 blockstates 命中 → 删除（证明两个 group / 两个根都查）。
        #[test]
        fn removes_matched_blockstate_from_group_three() {
            let mut pack = pack_with(34, vec![(BLOCKSTATE_PATH, LEGACY_BLOCKSTATE.to_vec())]);
            let outcome = run_on(&mut pack, &legacy_tables());
            assert_eq!(outcome.changed, 1);
            assert!(pack.view().resolve(BLOCKSTATE_PATH).is_none());
        }

        /// `target_missing`：命中旧原版指纹，但目标格式没有原生替代 → 保留 + note。
        #[test]
        fn keeps_path_that_target_format_lacks() {
            let tables = synthetic_tables(
                (MODEL_KEY, LEGACY_MODEL),
                (BLOCKSTATE_KEY, LEGACY_BLOCKSTATE),
                &[(34, "models", &[MODEL_KEY])],
            );
            let mut pack = pack_with(34, vec![(MODEL_PATH, LEGACY_MODEL.to_vec())]);
            let outcome = run_on(&mut pack, &tables);
            assert_eq!(outcome.changed, 0, "目标缺原生替代时不能删");
            assert!(pack.view().resolve(MODEL_PATH).is_some());
            assert!(
                outcome.notes.iter().any(|n| n.contains("无原生替代")),
                "必须记 note 说明为什么保留：{:?}",
                outcome.notes
            );
        }

        /// `target_missing` **按 kind 分开**：models 清单不能误保护同名字符串的 blockstates 文件。
        #[test]
        fn target_missing_is_checked_per_kind() {
            // 故意把 blockstates 的相对路径塞进 models 清单：按 kind 查表时不该命中。
            let tables = synthetic_tables(
                (MODEL_KEY, LEGACY_MODEL),
                (BLOCKSTATE_KEY, LEGACY_BLOCKSTATE),
                &[(34, "models", &[MODEL_KEY, BLOCKSTATE_KEY])],
            );
            let mut pack = pack_with(
                34,
                vec![
                    (MODEL_PATH, LEGACY_MODEL.to_vec()),
                    (BLOCKSTATE_PATH, LEGACY_BLOCKSTATE.to_vec()),
                ],
            );
            let outcome = run_on(&mut pack, &tables);
            assert!(pack.view().resolve(MODEL_PATH).is_some(), "models 路径在缺失清单里 → 保留");
            assert!(
                pack.view().resolve(BLOCKSTATE_PATH).is_none(),
                "blockstates 路径不在 blockstates 缺失清单里 → 照常停用"
            );
            assert_eq!(outcome.changed, 1, "只有 blockstates 被删：{outcome:?}");
        }

        /// 边界：target < 4 与 target == 1000 直接 no-op（即使表命中也不动）。
        #[test]
        fn noop_below_format_four_and_for_bedrock() {
            for format in [2u32, 3, 1000] {
                let mut pack = pack_with(format, vec![(MODEL_PATH, LEGACY_MODEL.to_vec())]);
                let outcome = run_on(&mut pack, &legacy_tables());
                assert_eq!(outcome.changed, 0, "format {format} 不该删除任何文件");
                assert!(
                    pack.view().resolve(MODEL_PATH).is_some(),
                    "format {format} 下文件必须原样保留"
                );
            }
        }

        /// 反例：解析失败的 `.json`、非 `.json` 文件都不碰（数据安全优先）。
        #[test]
        fn ignores_broken_json_and_non_json_files() {
            let mut pack = pack_with(
                34,
                vec![
                    ("assets/minecraft/models/block/broken.json", b"{not json".to_vec()),
                    ("assets/minecraft/models/block/readme.txt", b"custom".to_vec()),
                ],
            );
            // 用一张**正常**的表跑：损坏 JSON 根本算不出 canonical sha（解析失败），
            // 不可能命中任何指纹；非 .json 根本不在扫描范围内。两者都必须原样保留。
            let outcome = run_on(&mut pack, &legacy_tables());
            assert_eq!(outcome.changed, 0);
            assert!(pack
                .view()
                .resolve("assets/minecraft/models/block/broken.json")
                .is_some());
            assert!(pack
                .view()
                .resolve("assets/minecraft/models/block/readme.txt")
                .is_some());
        }

        /// notes 上限：命中很多文件时 notes 仍 ≤20（避免报告爆炸）。
        #[test]
        fn notes_are_capped_at_twenty() {
            let mut files: Vec<(String, Vec<u8>)> = Vec::new();
            let mut legacy = serde_json::Map::new();
            let mut models = serde_json::Map::new();
            for i in 0..30 {
                // 表键是包根相对路径；pack 里用的是带 assets/minecraft/ 的全路径。
                let key = format!("models/block/generated_{i}.json");
                let full = format!("assets/minecraft/{key}");
                let body = format!(r#"{{"parent":"block/cube_all","index":{i}}}"#);
                let sha = canonical_sha256(body.as_bytes()).expect("sha");
                models.insert(key, serde_json::json!([sha]));
                files.push((full, body.into_bytes()));
            }
            legacy.insert("2".to_string(), serde_json::json!({"models": models}));
            let root = serde_json::json!({
                "source": {"generator": "test"},
                "legacy": legacy,
                // 有该目标档的键才走正常删除路径；缺键会触发 fail-safe（见下一个用例）。
                "target_missing": {"34": {"models": [], "blockstates": []}},
            });
            let tables = Tables::from_json_str(&root.to_string()).expect("tables");
            let file_refs: Vec<(&str, Vec<u8>)> =
                files.iter().map(|(p, b)| (p.as_str(), b.clone())).collect();
            let mut pack = pack_with(34, file_refs);
            let outcome = run_on(&mut pack, &tables);
            assert_eq!(outcome.changed, 30, "30 个都该命中");
            assert_eq!(
                outcome.notes.len(),
                NOTES_LIMIT,
                "notes 必须截到 {NOTES_LIMIT} 条样本"
            );
        }

        /// **fail-safe**：目标格式不在数据表覆盖范围（例如未来新增了 Java 目标 format，
        /// 但还没有重新运行 `tools/gen-mc-reference.py`）→ 整任务保守跳过、不删任何文件。
        ///
        /// 为什么这条必须有：`is_target_missing` 对"没有该档键"的格式一律返回 false，
        /// 若不加此闸门，数据过期时会把旧原版 JSON 全删掉（fail-open，方向相反）。
        #[test]
        fn unknown_target_format_is_skipped_conservatively() {
            let mut pack = pack_with(45, vec![(MODEL_PATH, LEGACY_MODEL.to_vec())]);
            // legacy_tables() 只登记了 34 档 → 45 档触发 fail-safe。
            let outcome = run_on(&mut pack, &legacy_tables());
            assert_eq!(outcome.changed, 0, "数据未覆盖的目标格式不能删文件：{outcome:?}");
            assert_eq!(outcome.skipped, 1, "必须显式标记为跳过：{outcome:?}");
            assert!(
                pack.view().resolve(MODEL_PATH).is_some(),
                "fail-safe 下旧原版文件必须原样保留"
            );
            assert!(
                outcome.notes.iter().any(|n| n.contains("无参考数据")),
                "必须记 note 说明为什么整任务跳过：{:?}",
                outcome.notes
            );
        }
    }
