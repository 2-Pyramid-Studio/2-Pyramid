//! 无界面转换 CLI：`2-pyramid.exe --convert <路径> [--to <版本>] [--out <目录>] [--report <file.json>] [--delete-shaders] [--no-item-definitions] [--cit] [--delete-cit] [--no-gen-metadata]`
//!
//! 供拖放脚本使用：把资源包（或含资源包的文件夹）拖到脚本上 → 跑完整转换 →
//! 输出**结构化 JSON 报告**（结构分析 + 两个耗时口径 + 逐任务画像 + 体积变化）。
//!
//! 与 GUI 完全同一条管线（`process_zip_timed`），因此报告里的数字可直接用于
//! 性能优化前后的对比。
//!
//! `--delete-shaders` 与 GUI 转换页上的同名开关是**同一个参数**（默认关）：
//! 转完后把 `assets/minecraft/shaders` 与顶层 `shaders/` 整棵删掉。它存在的目的
//! 之一是让这个行为能被**端到端对照**（同一个小包，开关两次，比条目数）。
//!
//! `--no-item-definitions` / `--cit` / `--delete-cit` 同样对应转换页的三个
//! 新开关（默认：物品定义转换开、**CIT 转换关**、删除关）：
//! * `--no-item-definitions`：不写 `assets/minecraft/items/*.json`（旧 overrides 不迁移）；
//! * `--cit`：把 OptiFine CIT 转成物品定义。**默认关**（2026-10 A1 止血：CIT 的附魔
//!   条件在 1.21.9+ 可能让整份物品定义失效 ⇒ 黑紫格，理由见
//!   `native_run::MixedRunOptions::convert_cit`）。历史参数 `--no-cit` 仍被接受，
//!   但默认关之后它不再改变任何行为（仅为旧脚本兼容保留，不报错）；
//! * `--delete-cit`：跳过 CIT 转换并把两棵源目录整个删掉（**优先于** `--cit`）。
//!
//! `--no-gen-metadata`：不在产物根写 `2pyr-gen.json`。**默认写**（与 GUI 一致）：
//! 文件记录本次转换的版本/BUILD、源与目标格式、六个开关和 UTC 时间戳，不含任何
//! 路径或用户名；Minecraft 不读取它。字段与关闭方式同时披露在 `legal/EULA.md`
//! 与 `legal/PRIVACY.md`（中英各一份）——改行为要同步那四份。

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use two_pyramid_lib::arom::engine::scheduler::TaskTiming;
use two_pyramid_lib::{
    analyze_zip, pack_format_label_for_output, process_zip_timed, resolve_target_format,
    ConversionTiming, PackAnalysis,
};

/// 单个任务的画像条目（报告用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskProfileEntry {
    task: String,
    tier: String,
    seconds: f32,
    parallel: bool,
}

impl From<TaskTiming> for TaskProfileEntry {
    fn from(t: TaskTiming) -> Self {
        Self {
            task: t.task,
            tier: t.tier.to_string(),
            seconds: (t.seconds * 1000.0).round() / 1000.0,
            parallel: t.parallel,
        }
    }
}

/// 单个资源包的转换结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PackReport {
    input: String,
    output: Option<String>,
    status: String,
    error: Option<String>,
    input_bytes: Option<u64>,
    output_bytes: Option<u64>,
    /// 纯转换 / 总时间（含 IO）与 IO 分解。
    timing: Option<ConversionTiming>,
    /// 转换前的结构分析（分层/区间/折叠目录/多根）。
    structure: Option<PackAnalysis>,
    /// 逐任务耗时（按耗时降序，**完整列表**，便于分析长尾）。
    task_profile: Vec<TaskProfileEntry>,
    /// 任务画像条目数与耗时合计（秒），用于判断"长尾"占比。
    task_count: usize,
    task_sum_s: f32,
}

/// 整份报告。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConvertReport {
    generated_at: String,
    target_format: u32,
    target_label: String,
    out_dir: Option<String>,
    packs: Vec<PackReport>,
    summary: ReportSummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportSummary {
    packs: usize,
    success: usize,
    failed: usize,
    pure_sum_s: f32,
    total_sum_s: f32,
    io_sum_s: f32,
    wall_clock_s: f32,
    input_bytes: u64,
    output_bytes: u64,
}

fn collect_inputs(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.is_dir() {
        return Err(format!("路径不存在：{}", path.display()));
    }
    let mut packs: Vec<PathBuf> = walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .filter(|p| {
            let ext = p
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            ext == "zip" || ext == "mcpack"
        })
        .collect();
    packs.sort();
    if packs.is_empty() {
        return Err(format!("目录中没有找到 .zip / .mcpack：{}", path.display()));
    }
    Ok(packs)
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// `--convert` 入口。返回进程退出码。
pub fn run_convert(args: &[String], idx: usize) -> i32 {
    // CLI 是短命进程：临时目录清理交给脱离进程的 rmdir 子进程，
    // 既不等待、也不会留下残留。
    two_pyramid_lib::set_cleanup_mode(two_pyramid_lib::CleanupMode::Detached);

    let input = match args.get(idx + 1).filter(|v| !v.starts_with("--")) {
        Some(v) => v.clone(),
        None => {
            eprintln!(
                "用法: 2-pyramid.exe --convert <资源包.zip | 目录> [--to <版本|pack_format>] [--out <目录>] [--report <报告.json>] [--fast] [--delete-shaders] [--no-item-definitions] [--cit] [--delete-cit] [--no-gen-metadata]"
            );
            return 2;
        }
    };

    let target = match arg_value(args, "--to") {
        Some(spec) => match resolve_target_format(&spec) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("目标版本解析失败：{}", e);
                return 2;
            }
        },
        // 未指定时默认最新 Java 26.3（format 97）
        None => 97,
    };

    // §9.138：可选处理项。**默认全开**（产物完整）。
    // `--fast` 关闭 GUI sprite 手术——实测它是唯一能显著缩短耗时的处理项
    // （两次 `cut_gui`，各约 1.5s）。代价是丢失 GUI 切图产物（血条/护甲槽等）。
    let fast = args.iter().any(|a| a == "--fast");
    let run_gui_surgeon = !fast;
    // 破坏性开关，**默认关**：不加这个 flag 时产物与改动前逐字节相同。
    // 与 GUI 转换页的开关共用同一条管线参数（`process_zip_timed` 的最后一个 bool）。
    let delete_shaders = args.iter().any(|a| a == "--delete-shaders");
    // 物品定义 / CIT 三项，与 GUI 转换页同义（默认：物品定义转换开、CIT 转换关、删除关）。
    // 反向 flag 而不是 `--convert-cit=false`：CLI 的布尔开关全部是"出现即生效"，
    // 保持同一种读法，脚本里不会出现两种写法。
    let convert_item_definitions = !args.iter().any(|a| a == "--no-item-definitions");
    // **CIT 转换默认关**（2026-10 A1 止血：附魔条件可能让 1.21.9+ 的物品定义整份
    // 失效 ⇒ 黑紫格，理由见 `native_run::MixedRunOptions::convert_cit`）。
    // 这里用**正向** `--cit` 显式打开：改默认之后，原来的 `--no-cit` 就成了恒等参数。
    let convert_cit = args.iter().any(|a| a == "--cit");
    // 历史脚本兼容：旧版 `--no-cit` 仍被接受（出现即忽略），不报"未知参数"。
    // 不删掉是因为它此前是**默认行为**的反向开关，很多拖放脚本里还留着；
    // 新脚本请用 `--cit` 表达"我要开"。
    let _legacy_no_cit_ignored = args.iter().any(|a| a == "--no-cit");
    let delete_cit = args.iter().any(|a| a == "--delete-cit");
    // 2pyr-gen 元数据：与 GUI 默认一致（默认写），`--no-gen-metadata` 关闭。
    // 关闭后产物里不会出现 `2pyr-gen.json`；文件内容与披露见 `legal/EULA.md`。
    let write_gen_metadata = !args.iter().any(|a| a == "--no-gen-metadata");
    let out_dir = arg_value(args, "--out").map(PathBuf::from);
    let report_path = arg_value(args, "--report").map(PathBuf::from);

    let inputs = match collect_inputs(Path::new(&input)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{}", e);
            return 2;
        }
    };

    // 进程内阶段计时：转换之外的开销（结构分析、报告写入）一目了然
    let process_start = Instant::now();
    let mut analyze_secs = 0.0f32;
    let mut convert_secs = 0.0f32;

    println!("==> 2-Pyramid CLI 转换");
    println!("    输入：{}（{} 个资源包）", input, inputs.len());
    println!(
        "    目标：{}（pack_format {}）",
        pack_format_label_for_output(target),
        target
    );
    if let Some(dir) = &out_dir {
        println!("    输出目录：{}", dir.display());
    }
    // 元数据开关也打印出来：产物里多/少一个文件时，用户先看这里就能确认原因。
    // Bedrock 目标（1000）由管线层判掉、不写该文件，横幅必须如实显示
    // "不写（Bedrock 目标）"——否则用户会照着横幅去找一个不存在的文件。
    let gen_meta_banner = if !write_gen_metadata {
        "关闭"
    } else if target == 1000 {
        "不写（Bedrock 目标）"
    } else {
        "写入"
    };
    println!(
        "    2pyr-gen 元数据：{}（CIT 转换：{}）",
        gen_meta_banner,
        if convert_cit { "开" } else { "关" }
    );
    println!();

    let wall_start = Instant::now();
    let mut packs: Vec<PackReport> = Vec::new();

    for (i, pack) in inputs.iter().enumerate() {
        let name = pack
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        println!("[{}/{}] {}", i + 1, inputs.len(), name);

        let analyze_start = Instant::now();
        let structure = analyze_zip(pack).ok();
        analyze_secs += analyze_start.elapsed().as_secs_f32();
        if let Some(s) = &structure {
            println!("    结构：{}", s.summary());
            for w in &s.warnings {
                println!("      ! {}", w);
            }
        }
        let input_bytes = std::fs::metadata(pack).ok().map(|m| m.len());

        let convert_start = Instant::now();
        match process_zip_timed(
            &pack.to_string_lossy(),
            target,
            None,
            1.0,
            None,
            out_dir.as_ref().map(|d| d.to_string_lossy().to_string()).as_deref(),
            false,
            true,
            run_gui_surgeon,
            delete_shaders,
            convert_item_definitions,
            convert_cit,
            delete_cit,
            write_gen_metadata,
        ) {
            Ok((output, timing)) => {
                convert_secs += convert_start.elapsed().as_secs_f32();
                let output_bytes = std::fs::metadata(&output).ok().map(|m| m.len());
                let profile: Vec<TaskProfileEntry> = timing
                    .task_profile
                    .iter()
                    .cloned()
                    .map(TaskProfileEntry::from)
                    .collect();
                let task_count = profile.len();
                let task_sum_s: f32 = profile.iter().map(|t| t.seconds).sum();
                println!(
                    "    ✓ 纯转换 {:.2}s / 总时间 {:.2}s（IO {:.2}s）",
                    timing.pure_s,
                    timing.total_s,
                    timing.extract_s + timing.pack_s
                );
                for t in profile.iter().take(5) {
                    println!(
                        "      · {:<28} {:.2}s{}",
                        t.task,
                        t.seconds,
                        if t.parallel { " (parallel)" } else { "" }
                    );
                }
                if let (Some(a), Some(b)) = (input_bytes, output_bytes) {
                    println!(
                        "    ✓ 体积 {:.2} MB → {:.2} MB",
                        a as f64 / 1048576.0,
                        b as f64 / 1048576.0
                    );
                }
                println!("    ✓ 输出：{}", output);
                println!(
                    "    · 任务画像：{} 个任务，合计 {:.2}s（引擎纯转换 {:.2}s）",
                    task_count, task_sum_s, timing.pure_s
                );
                // §9.142：管线相位分解——任务只占流水线的一部分，找瓶颈要看这里。
                println!(
                    "    · 管线相位：open {:.2}s · materialize {:.2}s · tasks {:.2}s · direct {:.2}s · harvest {:.2}s · tail {:.2}s · output {:.2}s · other {:.2}s",
                    timing.phases.open_s,
                    timing.phases.materialize_s,
                    timing.phases.tasks_s,
                    timing.phases.direct_s,
                    timing.phases.harvest_s,
                    timing.phases.tail_s,
                    timing.phases.output_s,
                    timing.phases.other_s
                );
                packs.push(PackReport {
                    input: pack.to_string_lossy().to_string(),
                    output: Some(output),
                    status: "success".to_string(),
                    error: None,
                    input_bytes,
                    output_bytes,
                    timing: Some(timing),
                    structure,
                    task_profile: profile,
                    task_count,
                    task_sum_s,
                });
            }
            Err(e) => {
                convert_secs += convert_start.elapsed().as_secs_f32();
                let profile: Vec<TaskProfileEntry> = Vec::new();
                println!("    ✗ 失败：{}", e);
                packs.push(PackReport {
                    input: pack.to_string_lossy().to_string(),
                    output: None,
                    status: "error".to_string(),
                    error: Some(e),
                    input_bytes,
                    output_bytes: None,
                    timing: None,
                    structure,
                    task_profile: profile,
                    task_count: 0,
                    task_sum_s: 0.0,
                });
            }
        }
        println!();
    }

    let wall = wall_start.elapsed().as_secs_f32();
    let process_wall = process_start.elapsed().as_secs_f32();
    let success = packs.iter().filter(|p| p.status == "success").count();
    let pure_sum: f32 = packs
        .iter()
        .filter_map(|p| p.timing.as_ref())
        .map(|t| t.pure_s)
        .sum();
    let total_sum: f32 = packs
        .iter()
        .filter_map(|p| p.timing.as_ref())
        .map(|t| t.total_s)
        .sum();
    let io_sum: f32 = packs
        .iter()
        .filter_map(|p| p.timing.as_ref())
        .map(|t| t.extract_s + t.pack_s)
        .sum();
    let input_bytes: u64 = packs.iter().filter_map(|p| p.input_bytes).sum();
    let output_bytes: u64 = packs.iter().filter_map(|p| p.output_bytes).sum();

    let summary = ReportSummary {
        packs: packs.len(),
        success,
        failed: packs.len() - success,
        pure_sum_s: (pure_sum * 100.0).round() / 100.0,
        total_sum_s: (total_sum * 100.0).round() / 100.0,
        io_sum_s: (io_sum * 100.0).round() / 100.0,
        wall_clock_s: (wall * 100.0).round() / 100.0,
        input_bytes,
        output_bytes,
    };

    println!("================ 汇总 ================");
    println!(
        "资源包：{} 个（成功 {}，失败 {}）",
        summary.packs, summary.success, summary.failed
    );
    println!(
        "纯转换时间合计：{:.2}s ｜ 总时间合计（含 IO）：{:.2}s ｜ IO {:.2}s",
        summary.pure_sum_s, summary.total_sum_s, summary.io_sum_s
    );
    println!("墙钟时间：{:.2}s", summary.wall_clock_s);
    // **测量提示（§9.146）**：只有**进程内**这几个数（纯转换 / 含 IO 总时间 / 墙钟）彼此可比。
    // 外部用 PowerShell `Start-Process -Wait` 计时会**每次虚增 2.4–2.7s**
    // （实测：进程内 1.49s 被报成 3.93s；改用 `cmd /C` 同步执行则 1.52s ≈ 进程内 1.52s，
    // 真实进程开销只有约 0.05s）。我据此做过一次错误的优化取舍，故把结论留在输出里。
    println!("  （外部计时请用同步方式：Start-Process -Wait 会虚增约 2.5s，以进程内数为准）");
    if summary.input_bytes > 0 {
        println!(
            "体积：{:.2} MB → {:.2} MB",
            summary.input_bytes as f64 / 1048576.0,
            summary.output_bytes as f64 / 1048576.0
        );
    }

    let report = ConvertReport {
        generated_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        target_format: target,
        target_label: pack_format_label_for_output(target).to_string(),
        out_dir: out_dir.map(|d| d.to_string_lossy().to_string()),
        packs,
        summary,
    };

    if let Some(path) = report_path {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&report) {
            Ok(text) => match std::fs::write(&path, text) {
                Ok(()) => println!("\n报告已写入：{}", path.display()),
                Err(e) => eprintln!("报告写入失败：{}", e),
            },
            Err(e) => eprintln!("报告序列化失败：{}", e),
        }
    }

    // 阶段耗时：区分"转换"与"转换之外"（结构分析、报告写入、进程自身开销）。
    // 注意：临时目录清理默认交给脱离进程的 rmdir 子进程，不计入本行，
    // 因此某些外壳（会等待整个进程树）测到的墙钟会比这里更长。
    println!("阶段耗时：结构分析 {:.2}s · 转换 {:.2}s · 其他 {:.2}s = 进程内 {:.2}s",
        analyze_secs,
        convert_secs,
        (process_wall - analyze_secs - convert_secs).max(0.0),
        process_wall
    );

    if report.summary.failed > 0 { 1 } else { 0 }
}

/// CLI 退出前不做任何等待：清理已交给**脱离本进程**的 `rmdir` 子进程，
/// 进程立刻返回，删除在后台继续完成（因此也不会有临时目录残留）。
pub fn finish_pending_cleanups() {
    // 若仍有后台线程模式的清理在跑（例如模式被环境变量改过），最多等 3 秒
    if two_pyramid_lib::pending_cleanups() == 0 {
        return;
    }
    let _ = two_pyramid_lib::wait_for_cleanups(std::time::Duration::from_secs(3));
}
