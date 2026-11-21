#!/usr/bin/env python3
"""从 **Mojang 官方客户端 jar** 生成「旧原版模型 / 方块状态」参考数据。

为什么需要这个脚本
==================

2-Pyramid 的 eraser（擦除器）要能判断「这个 `models/**.json` / `blockstates/**.json`
是不是**没改过的旧原版文件**」：如果目标版本官方 jar 里**已经没有**这个路径，而包里这份
文件又是旧原版原样内容，就应当**保留**（直接删掉会让方块变黑紫/缺模型）；如果是作者改过的，
则不能认成原版。判定需要两样东西，都只能从 Mojang 官方 jar 重新生成：

1. 旧格式（pack_format 2 / 3）里每个官方路径的**规范化 JSON 指纹**（同一路径在不同小版本
   可能不同，所以每个路径存一组去重排序的 SHA-256）；
2. 每个目标格式（4–97）的官方 jar 里**不存在**哪些旧路径（→ 目标没有原生资源，保留旧文件）。

**为什么不用现成数据**：Mojang 的模型/方块状态是官方数据，手抄必错、抄第三方又有许可问题。
从官方客户端 jar 重新提取是唯一同时满足「权威」「可复现」「无许可风险」的路。
提交进版本库的只有**包根相对路径**与**规范化 JSON 的 SHA-256**，不含任何纹理/模型/音效等
官方资源文件（见 legal/THIRD-PARTY-NOTICES.md）。

产出的两个文件
==============

* `src-tauri/src/natives/eraser/legacy-defaults.json`（Rust 运行时 `include_str!` 读取）
  形状见 `build_payload()`；schema 由 Lead 的占位文件钉死，本脚本逐字段照做。
* `src-tauri/src/natives/eraser/legacy-defaults.fixtures.json`（Rust 单测用）
  11 条**完全 synthetic** 的 canonicalization 夹具（跨语言：Python ↔ Rust 必须逐字节一致）。

canonicalization（Rust 侧逐字复现，**不要改顺序/参数**）
======================================================

    字节 → 去 UTF-8 BOM → json.loads（标准 JSON；重复键取后者）
         → json.dumps(obj, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
         → UTF-8 编码 → sha256 hex（小写）

* `ensure_ascii=False`：Unicode 原样输出（不转 `\\uXXXX`）；
* `sort_keys=True`：对象键按 Unicode 码点排序（Rust 用 `BTreeMap` 即等价）；
* `separators=(",", ":")`：不含任何多余空白；
* 数字保留 int/float 区别（`1` 与 `1.0` 的规范化结果不同）——生成数据时按真实文件内容；
* **不接受** `NaN` / `Infinity`（标准 JSON 里没有；遇到就报错，而不是悄悄写进表）。

jar 从哪来 / 如何校验格式
=========================

1. 优先用 `tools/.mc-cache/jars/<v>.jar`（上次下载的缓存）；
2. 否则用本机 `%APPDATA%\\.minecraft\\versions\\<v>\\<v>.jar`（官方 launcher 装的）；
3. 都没有才从 Mojang 官方 manifest 下载，**校验 sha1 + 字节数**后写进 `.mc-cache/jars/`，
   同时把 Mojang 的版本 json 缓存到 `.mc-cache/version-json/<v>.json`。

pack_format 的来源（`source.jars[].format_source`）：

* `"jar"` —— 客户端 jar 根目录自带 `version.json`，读 `pack_version.resource`
  （1.21.10+ 改叫 `resource_major`，两种都支持）并与矩阵里的预期值**校验相等**；
* `"mapping"` —— **旧版客户端 jar（1.9.4–1.12.2 等）根本没有 `version.json`**
  （实测 1.9.4 / 1.12.2 jar 根目录只有 `META-INF/` 等，无 version.json），
  只能回退脚本里的 `LEGACY_FORMAT_BY_VERSION`（= Lead 给的格式矩阵）。
  这不是「猜」：同时用 launcher / Mojang 的版本 json **校验 `id` == 预期版本**
  （版本 json 里**没有** pack_version 字段——实测 1.9.4/1.12.2/1.13.2/1.21.10/26.3 都没有，
  所以它只能证明「这个 jar 确实是这个版本」，格式号仍来自映射表）。
  绝不用错误的格式号静默写表：生成时对每个能自报格式的 jar 逐条与矩阵比对
  （`jar_self_format()` vs `EXPECTED_FORMAT`），对不上直接退出。

输出确定性
==========

输出不含时间戳/随机数/机器路径；两周内反复运行，只要 jar 不变就**逐字节相同**。
`--check` 用于 CI/人工复核：重新生成到内存并与已提交文件逐字节比较。

用法
====

    python tools/gen-mc-reference.py            # 生成（缺 jar 会下载）
    python tools/gen-mc-reference.py --check    # 只校验已提交文件是否与 jar 一致
    python tools/gen-mc-reference.py --offline  # 只用缓存，不联网

许可
====

脚本本身是 2-Pyramid 原创（MIT）。输出是「路径 + 规范化 JSON 的 SHA-256」，
不含 Mojang 资源文件内容；Minecraft 资源仍归 Mojang Studios / Microsoft 所有。
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
import sys
import urllib.request
import zipfile

# ── 版本矩阵（Lead 定；改矩阵 = 改数据，必须同时跑 --check 并重新提交 json）────────
#
# legacy：旧格式 → 代表性客户端版本。两个小版本都要，因为「同一路径不同内容」必须都收录
# （例：1.9.4 与 1.10.2 的 models/block/xxx.json 有差异）。
LEGACY_VERSIONS: dict[int, list[str]] = {
    2: ["1.9.4", "1.10.2"],
    3: ["1.11.2", "1.12.2"],
}
# target：pack_format → 代表版本。空列表合法的格式不必出现；这里 4..97 共 23 档。
TARGET_VERSIONS: dict[int, str] = {
    4: "1.13.2",
    5: "1.15.2",
    6: "1.16.5",
    7: "1.17.1",
    8: "1.18.2",
    9: "1.19.2",
    12: "1.19.3",
    13: "1.19.4",
    15: "1.20.1",
    18: "1.20.2",
    22: "1.20.4",
    32: "1.20.6",
    34: "1.21.1",
    42: "1.21.3",
    46: "1.21.4",
    55: "1.21.5",
    63: "1.21.6",
    64: "1.21.8",
    69: "1.21.10",
    75: "1.21.11",
    84: "26.1.1",
    88: "26.2",
    97: "26.3",
}

# 反向表：版本 → 预期 pack_format（`--verify-formats` 与 source.jars 共用）。
EXPECTED_FORMAT: dict[str, int] = {}
for _fmt, _versions in LEGACY_VERSIONS.items():
    for _v in _versions:
        EXPECTED_FORMAT[_v] = _fmt
for _fmt, _v in TARGET_VERSIONS.items():
    EXPECTED_FORMAT[_v] = _fmt
LEGACY_VERSION_SET = {v for versions in LEGACY_VERSIONS.values() for v in versions}

# 规范化规则说明：两个输出文件**共用同一字符串**，Rust 侧按它实现。
CANONICALIZATION = (
    "去 UTF-8 BOM → json.loads（标准 JSON，重复键取后者）→ "
    'json.dumps(obj, ensure_ascii=False, sort_keys=True, separators=(",", ":")) → '
    "UTF-8 编码 → sha256 hex（小写）"
)
NOTE = "仅含包根相对路径与规范化 JSON 的 SHA-256；不含任何 Mojang 资源文件。"

GENERATOR = "tools/gen-mc-reference.py"
MANIFEST_URL = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"

# assets/minecraft/ 下这两棵子树；路径按「包根相对路径」保存（去掉 assets/minecraft/ 前缀）。
MODELS_PREFIX = "assets/minecraft/models/"
BLOCKSTATES_PREFIX = "assets/minecraft/blockstates/"
KINDS = ("models", "blockstates")
PREFIXES = {"models": MODELS_PREFIX, "blockstates": BLOCKSTATES_PREFIX}

SCRIPT = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(SCRIPT)
CACHE = os.path.join(SCRIPT, ".mc-cache")
JAR_CACHE = os.path.join(CACHE, "jars")
VERSION_JSON_CACHE = os.path.join(CACHE, "version-json")
MANIFEST_CACHE = os.path.join(CACHE, "version_manifest.json")

OUT_DATA = os.path.join(REPO, "src-tauri", "src", "natives", "eraser", "legacy-defaults.json")
OUT_FIXTURES = os.path.join(
    REPO, "src-tauri", "src", "natives", "eraser", "legacy-defaults.fixtures.json"
)


# ── canonicalization（Rust 侧必须逐字复现）────────────────────────────────────


def _reject_constant(value: str):
    """`NaN` / `Infinity` / `-Infinity` 不是标准 JSON；显式拒绝，避免写出不可复现的指纹。"""
    raise ValueError(f"非标准 JSON 常量 {value!r}（canonicalization 只接受标准 JSON）")


def canonical_text(raw: bytes) -> str:
    """规范化 JSON 文本（去 BOM → parse → dump）。重复键由 json 取后者（标准行为）。"""
    if raw.startswith(b"\xef\xbb\xbf"):
        raw = raw[3:]
    obj = json.loads(raw.decode("utf-8"), parse_constant=_reject_constant)
    return json.dumps(obj, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def canonical_sha(raw: bytes) -> str:
    return hashlib.sha256(canonical_text(raw).encode("utf-8")).hexdigest()


def sha256_file(path: str) -> tuple[str, int]:
    h = hashlib.sha256()
    total = 0
    with open(path, "rb") as fh:
        while True:
            chunk = fh.read(1 << 20)
            if not chunk:
                break
            h.update(chunk)
            total += len(chunk)
    return h.hexdigest(), total


# ── jar 定位 / 下载 / 格式自报 ─────────────────────────────────────────────────


def mc_versions_dir() -> str:
    appdata = os.environ.get("APPDATA")
    if not appdata:
        sys.exit("APPDATA 未设置——本脚本需要在 Windows 上运行")
    return os.path.join(appdata, ".minecraft", "versions")


def local_jar_path(version: str) -> str:
    return os.path.join(mc_versions_dir(), version, version + ".jar")


def local_version_json_path(version: str) -> str:
    return os.path.join(mc_versions_dir(), version, version + ".json")


def cache_jar_path(version: str) -> str:
    return os.path.join(JAR_CACHE, version + ".jar")


def cache_version_json_path(version: str) -> str:
    return os.path.join(VERSION_JSON_CACHE, version + ".json")


def load_manifest(offline: bool) -> dict:
    os.makedirs(CACHE, exist_ok=True)
    if offline:
        if not os.path.isfile(MANIFEST_CACHE):
            sys.exit(f"--offline 需要缓存清单：{MANIFEST_CACHE}")
        with open(MANIFEST_CACHE, "r", encoding="utf-8") as fh:
            return json.load(fh)
    with urllib.request.urlopen(MANIFEST_URL, timeout=60) as resp:
        raw = resp.read()
    os.makedirs(CACHE, exist_ok=True)
    with open(MANIFEST_CACHE, "wb") as fh:
        fh.write(raw)
    return json.loads(raw)


def fetch_json(url: str) -> dict:
    with urllib.request.urlopen(url, timeout=60) as resp:
        return json.loads(resp.read())


def download_bytes(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=300) as resp:
        return resp.read()


def ensure_version_json(version: str, manifest_by_id: dict, offline: bool) -> dict:
    """拿到一个「id == version」的版本 json；本地 launcher 的优先，其次缓存/Mojang。"""
    for path in (local_version_json_path(version), cache_version_json_path(version)):
        if os.path.isfile(path):
            with open(path, "r", encoding="utf-8") as fh:
                meta = json.load(fh)
            if meta.get("id") != version:
                sys.exit(f"{path} 的 id={meta.get('id')!r} 与版本 {version} 不符")
            return meta
    if offline:
        sys.exit(f"--offline 下找不到 {version} 的 version json（launcher 或缓存）")
    entry = manifest_by_id.get(version)
    if entry is None:
        sys.exit(f"Mojang manifest 里没有版本 {version}")
    meta = fetch_json(entry["url"])
    if meta.get("id") != version:
        sys.exit(f"Mojang version json 的 id={meta.get('id')!r} 与 {version} 不符")
    os.makedirs(VERSION_JSON_CACHE, exist_ok=True)
    with open(cache_version_json_path(version), "w", encoding="utf-8", newline="\n") as fh:
        json.dump(meta, fh, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return meta


def ensure_jar(version: str, manifest_by_id: dict, offline: bool) -> tuple[str, bool]:
    """返回 (jar 路径, 是否本次下载)。下载的 jar 用 Mojang 给的 sha1/size 校验。"""
    cached = cache_jar_path(version)
    if os.path.isfile(cached):
        return cached, False
    local = local_jar_path(version)
    if os.path.isfile(local):
        return local, False
    if offline:
        sys.exit(f"--offline 下缺少 {version} 的客户端 jar（{cached} 或 {local}）")
    entry = manifest_by_id.get(version)
    if entry is None:
        sys.exit(f"Mojang manifest 里没有版本 {version}")
    meta = fetch_json(entry["url"])
    client = meta["downloads"].get("client")
    if not client:
        sys.exit(f"{version} 的 version json 没有 downloads.client")
    # 顺手缓存 Mojang 版本 json：后面 ensure_version_json 的 id 校验要用它，
    # 且让 --offline 重跑不再依赖网络。
    os.makedirs(VERSION_JSON_CACHE, exist_ok=True)
    if not os.path.isfile(cache_version_json_path(version)):
        with open(cache_version_json_path(version), "w", encoding="utf-8", newline="\n") as fh:
            json.dump(meta, fh, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    print(f"  下载 {version} 客户端 jar（{client['size']} 字节）…", flush=True)
    raw = download_bytes(client["url"])
    got_sha1 = hashlib.sha1(raw).hexdigest()
    if got_sha1 != client["sha1"] or len(raw) != client["size"]:
        sys.exit(
            f"{version} jar 校验失败：sha1 {got_sha1} != {client['sha1']}"
            f" 或大小 {len(raw)} != {client['size']}"
        )
    os.makedirs(JAR_CACHE, exist_ok=True)
    with open(cached, "wb") as fh:
        fh.write(raw)
    return cached, True


def jar_self_format(jar_path: str) -> int | None:
    """jar 根目录 `version.json` 自报的 pack_format；旧 jar 没有它 → None。

    这个字段有**三代形状**（实测，别"统一"掉）：

    * 1.15.2 / 1.16.5：`"pack_version": 5`  —— 裸整数；
    * 1.17.1 – 1.21.8：`"pack_version": {"resource": 7, "data": 7}`；
    * 1.21.10+：`"pack_version": {"resource_major": 69, "resource_minor": 0, ...}`。

    另外 1.13.2 及更早的客户端 jar **根本没有 version.json**（实测 1.13.2 / 1.12.2 /
    1.9.4 jar 根目录只有 `META-INF/` 等），只能回退版本矩阵。
    """
    try:
        with zipfile.ZipFile(jar_path) as z:
            if "version.json" not in z.namelist():
                return None
            meta = json.loads(z.read("version.json").decode("utf-8"))
    except (zipfile.BadZipFile, json.JSONDecodeError, UnicodeDecodeError) as exc:
        sys.exit(f"{jar_path} 读取 version.json 失败：{exc}")
    pack_version = meta.get("pack_version")
    if isinstance(pack_version, int):  # 1.15.2 / 1.16.5 时代
        return pack_version
    if not isinstance(pack_version, dict):
        sys.exit(f"{jar_path} 的 version.json 里 pack_version 形状未知：{pack_version!r}")
    if "resource" in pack_version:  # 1.17.1 – 1.21.8
        return int(pack_version["resource"])
    # 1.21.10+ 改成 resource_major / resource_minor；本数据集 minor 恒可忽略
    # （26.3 是 97 + minor 1，2pyr 的格式号仍按 97 记，与版本矩阵一致）。
    return int(pack_version["resource_major"])


# ── jar 内容扫描 ──────────────────────────────────────────────────────────────


def scan_paths(jar_path: str) -> tuple[set[str], set[str]]:
    """只收 assets/minecraft/{models,blockstates}/**.json 的包根相对路径。"""
    models: set[str] = set()
    blockstates: set[str] = set()
    with zipfile.ZipFile(jar_path) as z:
        for name in z.namelist():
            if not name.endswith(".json"):
                continue
            if name.startswith(MODELS_PREFIX):
                models.add(name[len("assets/minecraft/") :])
            elif name.startswith(BLOCKSTATES_PREFIX):
                blockstates.add(name[len("assets/minecraft/") :])
    return models, blockstates


def scan_hashes(jar_path: str) -> tuple[dict[str, set[str]], dict[str, set[str]]]:
    """旧版 jar：逐文件读 → 规范化 → sha256（同一路径在多个版本里可能不同）。"""
    models: dict[str, set[str]] = {}
    blockstates: dict[str, set[str]] = {}
    with zipfile.ZipFile(jar_path) as z:
        for name in z.namelist():
            if not name.endswith(".json"):
                continue
            if name.startswith(MODELS_PREFIX):
                bucket = models
            elif name.startswith(BLOCKSTATES_PREFIX):
                bucket = blockstates
            else:
                continue
            rel = name[len("assets/minecraft/") :]
            try:
                digest = canonical_sha(z.read(name))
            except (json.JSONDecodeError, ValueError, UnicodeDecodeError) as exc:
                sys.exit(f"{jar_path} 里的 {name} 不是合法 JSON：{exc}")
            bucket.setdefault(rel, set()).add(digest)
    return models, blockstates


# ── 构建 payload ──────────────────────────────────────────────────────────────


def build_legacy(jar_info: dict[str, dict]) -> dict:
    """legacy[format][kind][path] = 去重排序的 sha256 列表。"""
    legacy: dict[str, dict[str, dict[str, list[str]]]] = {}
    for fmt, versions in LEGACY_VERSIONS.items():
        merged: dict[str, dict[str, set[str]]] = {kind: {} for kind in KINDS}
        for version in versions:
            hashed = jar_info[version]["hashes"]
            for kind in KINDS:
                for path, digests in hashed[kind].items():
                    merged[kind].setdefault(path, set()).update(digests)
        legacy[str(fmt)] = {
            kind: {path: sorted(digests) for path, digests in sorted(merged[kind].items())}
            for kind in KINDS
        }
    return legacy


def build_payload() -> dict:
    manifest = load_manifest(OFFLINE)
    manifest_by_id = {entry["id"]: entry for entry in manifest["versions"]}

    versions_in_order = [v for fmt in sorted(LEGACY_VERSIONS) for v in LEGACY_VERSIONS[fmt]]
    versions_in_order += [v for fmt in sorted(TARGET_VERSIONS) for v in [TARGET_VERSIONS[fmt]]]

    jars_dir = JAR_CACHE
    os.makedirs(jars_dir, exist_ok=True)

    # 先确保所有 jar 都在（下载可并发；只缺的才下载）。
    missing = [
        v
        for v in versions_in_order
        if not os.path.isfile(cache_jar_path(v)) and not os.path.isfile(local_jar_path(v))
    ]
    if missing:
        print(f"缺少 {len(missing)} 个客户端 jar，开始下载（并发 3）：{', '.join(missing)}")
        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
            futures = {pool.submit(ensure_jar, v, manifest_by_id, OFFLINE): v for v in missing}
            for future in concurrent.futures.as_completed(futures):
                version = futures[future]
                path, downloaded = future.result()
                print(f"  {version}: {path}" + ("（新下载）" if downloaded else ""), flush=True)

    # 所有 jar 都要有 id 校验的 version json（旧 jar 无 version.json，格式号只能靠矩阵）。
    for version in versions_in_order:
        ensure_version_json(version, manifest_by_id, OFFLINE)

    jar_info: dict[str, dict] = {}
    sources: list[dict] = []
    print(
        f"\njar 清单（{len(versions_in_order)} 个：legacy "
        f"{sum(len(v) for v in LEGACY_VERSIONS.values())} + target {len(TARGET_VERSIONS)}）："
    )
    for version in versions_in_order:
        jar_path, _downloaded = ensure_jar(version, manifest_by_id, OFFLINE)
        digest, size = sha256_file(jar_path)
        expected = EXPECTED_FORMAT[version]
        self_reported = jar_self_format(jar_path)
        if self_reported is None:
            format_source = "mapping"
        else:
            if self_reported != expected:
                sys.exit(
                    f"{version} 的自报 pack_format={self_reported} 与矩阵 {expected} 不符"
                    "——先核对版本矩阵，修好再生成，绝不静默写错格式"
                )
            format_source = "jar"
        role = "legacy" if version in LEGACY_VERSION_SET else "target"
        sources.append(
            {
                "version": version,
                "role": role,
                "pack_format": expected,
                "format_source": format_source,
                "sha256": digest,
                "bytes": size,
            }
        )
        jar_info[version] = {"path": jar_path, "sha256": digest, "bytes": size}
        print(
            f"  {version:<7} format={expected:<2} source={format_source:<7} "
            f"sha256={digest[:16]}… {size:>10} 字节  {os.path.basename(jar_path)}"
        )

    # legacy：逐文件指纹；target：只需要路径集合。
    legacy_hashes: dict[str, dict] = {}
    for version in versions_in_order:
        if version in LEGACY_VERSION_SET:
            print(f"  提取 legacy 指纹 {version} …", flush=True)
            models, blockstates = scan_hashes(jar_info[version]["path"])
            legacy_hashes[version] = {"models": models, "blockstates": blockstates}
    legacy = build_legacy(
        {version: {"hashes": legacy_hashes[version]} for version in legacy_hashes}
    )

    legacy_paths = {kind: set() for kind in KINDS}
    for fmt in legacy.values():
        for kind in KINDS:
            legacy_paths[kind].update(fmt[kind])

    # target_missing：所有 legacy 路径中，目标 jar 里不存在的（路径存在性，不做 JSON 解析）。
    target_missing: dict[str, dict[str, list[str]]] = {}
    for fmt in sorted(TARGET_VERSIONS):
        version = TARGET_VERSIONS[fmt]
        present_models, present_blockstates = scan_paths(jar_info[version]["path"])
        target_missing[str(fmt)] = {
            "models": sorted(legacy_paths["models"] - present_models),
            "blockstates": sorted(legacy_paths["blockstates"] - present_blockstates),
        }

    payload = {
        "source": {
            "generator": GENERATOR,
            "canonicalization": CANONICALIZATION,
            "note": NOTE,
            "jars": sources,
        },
        "legacy": legacy,
        "target_missing": target_missing,
    }
    return payload


# ── fixtures（跨语言 canonicalization 夹具；全部 synthetic）───────────────────

# 每条 = (name, 原始 JSON 文本)。文本故意包含乱序键/嵌套/数组/int vs float/负数/
# Unicode/转义/空容器/BOM/`\uXXXX` 转义/非常规空白。**不得**放入任何 Mojang 内容。
FIXTURES: list[tuple[str, str]] = [
    ("object_keys_reordered", '{"b":2,"a":1}'),
    ("nested_objects", '{"outer":{"z":[1,2],"a":{"k":true}},"top":null}'),
    ("array_order_preserved", "[3,1,2]"),
    ("int_vs_float_and_negative", '{"int":1,"float":1.0,"neg":-2,"zero":0,"negfloat":-0.5}'),
    ("unicode_kept", '{"中文":"你好","é":"café","emoji":"😀"}'),
    (
        "escapes_control_chars",
        '{"quote":"he said \\"hi\\"","slash":"a\\\\b","newline":"line1\\nline2","tab":"a\\tb"}',
    ),
    ("empty_containers", '{"obj":{},"arr":[],"nested":[[],{}]}'),
    ("bom_prefixed", "\ufeff" + '{"bom":true,"n":1}'),
    ("unicode_escape_input", '{"emoji":"\\ud83d\\ude00","cn":"\\u4e2d\\u6587"}'),
    ("odd_whitespace", ' { "z" : 1 , "a" : [ true , false ] } '),
    ("root_scalar_float", "1.25"),
]


def build_fixtures() -> dict:
    fixtures = []
    for name, raw_text in FIXTURES:
        raw = raw_text.encode("utf-8")
        fixtures.append(
            {
                "name": name,
                "json": raw_text,
                "canonical": canonical_text(raw),
                "sha256": hashlib.sha256(canonical_text(raw).encode("utf-8")).hexdigest(),
            }
        )
    return {"canonicalization": CANONICALIZATION, "fixtures": fixtures}


# ── 输出 / 校验 ───────────────────────────────────────────────────────────────


def dumps_compact(payload: dict, ensure_ascii: bool) -> str:
    return json.dumps(payload, ensure_ascii=ensure_ascii, sort_keys=True, separators=(",", ":"))


def write_outputs(data_payload: dict, fixtures_payload: dict) -> tuple[str, str]:
    data_text = dumps_compact(data_payload, ensure_ascii=False)
    fixtures_text = dumps_compact(fixtures_payload, ensure_ascii=True)
    for path, text in ((OUT_DATA, data_text), (OUT_FIXTURES, fixtures_text)):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(text)
    return data_text, fixtures_text


def print_summary(data: dict) -> None:
    print("\n===== 摘要 =====")
    source = data["source"]
    print(f"jar 数：{len(source['jars'])}（legacy 4 / target {len(TARGET_VERSIONS)}）")
    for fmt in sorted(LEGACY_VERSIONS):
        group = data["legacy"][str(fmt)]
        models = group["models"]
        blockstates = group["blockstates"]
        hashes = sum(len(v) for v in models.values()) + sum(len(v) for v in blockstates.values())
        print(
            f"legacy[{fmt}]：models {len(models)} 路径 / {sum(len(v) for v in models.values())} hash；"
            f"blockstates {len(blockstates)} 路径 / {sum(len(v) for v in blockstates.values())} hash；"
            f"合计 {hashes} hash"
        )
    for fmt in sorted(TARGET_VERSIONS):
        missing = data["target_missing"][str(fmt)]
        print(
            f"target_missing[{fmt:>2}]（{TARGET_VERSIONS[fmt]:<7}）："
            f"models 缺 {len(missing['models'])}，blockstates 缺 {len(missing['blockstates'])}"
        )
    data_size = os.path.getsize(OUT_DATA)
    fixtures_size = os.path.getsize(OUT_FIXTURES)
    print(f"输出：{OUT_DATA}  {data_size} 字节")
    print(f"输出：{OUT_FIXTURES}  {fixtures_size} 字节")
    for fmt in sorted(LEGACY_VERSIONS):
        assert "models/block/cube_all.json" in data["legacy"][str(fmt)]["models"], "缺 cube_all"
        assert "blockstates/stone.json" in data["legacy"][str(fmt)]["blockstates"], "缺 stone"
    print("抽查：legacy 各格式均含 models/block/cube_all.json 与 blockstates/stone.json ✓")


def run_check(data_payload: dict, fixtures_payload: dict) -> int:
    expected = {
        OUT_DATA: dumps_compact(data_payload, ensure_ascii=False),
        OUT_FIXTURES: dumps_compact(fixtures_payload, ensure_ascii=True),
    }
    ok = True
    for path, text in expected.items():
        if not os.path.isfile(path):
            print(f"CHECK FAIL：缺少 {path}")
            ok = False
            continue
        with open(path, "r", encoding="utf-8", newline="") as fh:
            actual = fh.read()
        if actual != text:
            print(f"CHECK FAIL：{path} 与 jar 重新生成的结果不一致")
            ok = False
        else:
            print(f"CHECK OK：{path}（{os.path.getsize(path)} 字节，sha256 "
                  f"{hashlib.sha256(actual.encode('utf-8')).hexdigest()[:16]}…）")
    return 0 if ok else 1


def main() -> None:
    global OFFLINE
    parser = argparse.ArgumentParser(description="从 Mojang 官方 jar 生成 legacy-defaults 参考数据")
    parser.add_argument("--check", action="store_true", help="只比较已提交文件，不写盘")
    parser.add_argument("--offline", action="store_true", help="只用缓存/本地 jar，不联网")
    args = parser.parse_args()
    OFFLINE = args.offline
    os.makedirs(JAR_CACHE, exist_ok=True)
    os.makedirs(VERSION_JSON_CACHE, exist_ok=True)

    data_payload = build_payload()
    fixtures_payload = build_fixtures()

    if args.check:
        sys.exit(run_check(data_payload, fixtures_payload))

    data_text, fixtures_text = write_outputs(data_payload, fixtures_payload)
    print_summary(data_payload)
    print("\n夹具：11 条 synthetic canonicalization fixtures")
    print(f"数据文件 sha256：{hashlib.sha256(data_text.encode('utf-8')).hexdigest()}")
    print(f"夹具文件 sha256：{hashlib.sha256(fixtures_text.encode('utf-8')).hexdigest()}")


OFFLINE = False

if __name__ == "__main__":
    main()
