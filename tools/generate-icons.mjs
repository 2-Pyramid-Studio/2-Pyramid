// tools/generate-icons.mjs — 从 brand/ 品牌 SVG 生成全量 OS 图标（可复现、带自检）
// 用法: node tools/generate-icons.mjs
//
// 产物（全部覆盖同名旧文件，不新增/不删除文件名）：
//   src-tauri/icons/icon.ico            自实现 ICO（PNG 帧，尺寸恰好 16/24/32/48/64/256）
//   src-tauri/icons/icon.icns           自实现 ICNS（PNG 帧 icp4/icp5/icp6/ic07/ic08/ic09）
//   src-tauri/icons/{32x32,64x64,128x128,128x128@2x(256),icon(512)}.png
//   src-tauri/icons/{StoreLogo,Square*Logo}.png、Wide310x150Logo.png、SplashScreen.png（MSIX 资产）
//   installer-app/src-tauri/icons/{icon.ico,icon.png,32x32.png,128x128.png,128x128@2x.png}
//
// 设计契约（OS 图标最终设计，见任务 task-5）：
//   * 白底 #FFFFFF 不透明圆角方板铺满画布，圆角外 alpha=0；圆角半径 = min(边)×0.22。
//   * artwork 居中，margin（画板内缩）: 16→1 / 24→1.5 / 32→2.5 / 48→4 / 64→6 /
//     128→12 / 256→24 / 512→48（≥512 按 9%），中间尺寸在相邻档间线性插值。
//   * artwork 分档：size ≤ 24 用 2-pyramid-micro.svg；24 < size < 64 用 2-pyramid-favicon.svg
//     （契约点名的 32/48 档；MSIX StoreLogo=50 也落此档）；size ≥ 64 用 2-pyramid-mark.svg。
//
// 为什么 mark 用「裁剪 viewBox 40 40 430 430」而不是它自带的 0 0 512 512：
//   mark.svg 画布自带 ~15% 留白（墨迹 71.7%），favicon/micro 已是同一几何的 40..470 裁剪版
//   （墨迹 87%）。若把 mark 整幅 512 画布映射进画板内缩框，≥64 档墨迹只有画板的 ~59%，
//   而 48 档（favicon）有 ~73%，任务栏在 48/64 之间缩放时会出现肉眼可见的「图标变小」跳变。
//   用与 favicon 相同的 40 40 430 430 裁剪框渲染 mark，墨迹占画板 ~70%，与 48 档衔接连续，
//   且完全保留 mark 的几何与 12/9 细线宽（brand/ 源文件不改）。
//
// 关键实测数字（本脚本生成后自检会打印）：
//   旧 ICO（HEAD 42f0621）16px 只有 2 个深色像素（luma<128）、24px 15、32px 49 → 近空白；
//   新 ICO 16px ≈35、24px ≈76、32px ≈90、48px ≈193。

import sharp from 'sharp';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, '..');
const BRAND = resolve(ROOT, 'brand');
const ICONS = resolve(ROOT, 'src-tauri', 'icons');
const INSTALLER_ICONS = resolve(ROOT, 'installer-app', 'src-tauri', 'icons');

const PLATE_FILL = '#FFFFFF';
// favicon / micro 的裁剪框；mark 也用它（原因见文件头注释）
const ART_VIEWBOX = { x: 40, y: 40, w: 430, h: 430 };
// mark 在上述 430 裁剪框内的可见墨迹高度占比：
// 几何点 x ∈ [76,430]、y ∈ [76,430]，stroke-width 12 的圆帽向外扩 6 ⇒ 墨迹 [70,436] → 366/430。
const MARK_INK_RATIO = 366 / 430;

// 画板内缩（artwork margin）断点；中间尺寸线性插值，>512 按 9%
const MARGIN_STOPS = [
  [16, 1],
  [24, 1.5],
  [32, 2.5],
  [48, 4],
  [64, 6],
  [128, 12],
  [256, 24],
  [512, 48],
];

// 深色像素（alpha>0 且 luma<128）下限。luma 阈值沿用 Lead 评审旧图标时的口径
// （旧 16px=2 / 24px=15 / 32px=49 即用该口径测得）。
const DARK_MIN = { 16: 14, 24: 28, 32: 50, 48: 110 };

// MSIX 资产：名字与尺寸必须与仓库现状完全一致（tools/build_release.py 直接按名拷贝）
const MSIX_SQUARES = [
  ['StoreLogo.png', 50],
  ['Square30x30Logo.png', 30],
  ['Square44x44Logo.png', 44],
  ['Square71x71Logo.png', 71],
  ['Square89x89Logo.png', 89],
  ['Square107x107Logo.png', 107],
  ['Square142x142Logo.png', 142],
  ['Square150x150Logo.png', 150],
  ['Square284x284Logo.png', 284],
  ['Square310x310Logo.png', 310],
];
// Wide/Splash 的「mark 高」按可见墨迹高度理解（mark.svg 自带留白，若按 SVG 画布计会偏小）：
//   Wide 墨迹 ~92px（画布 ~108px），Splash 墨迹 ~120px（画布 ~141px）。
const MSIX_WIDE = { name: 'Wide310x150Logo.png', w: 310, h: 150, inkH: 92 };
const MSIX_SPLASH = { name: 'SplashScreen.png', w: 620, h: 300, inkH: 120 };

// ---------------------------------------------------------------------------
// 基础工具
// ---------------------------------------------------------------------------

const files = []; // 清单（打印 + 断言用）
let assertCount = 0;

function assert(cond, msg) {
  assertCount++;
  if (!cond) throw new Error(`自检失败: ${msg}`);
}

function fail(msg) {
  throw new Error(msg);
}

function fmtBytes(n) {
  return n.toLocaleString('en-US');
}

// 读源 SVG，取出里面的 <g>…</g>（几何组），断言存在
async function loadArt(name) {
  const file = resolve(BRAND, `2-pyramid-${name}.svg`);
  const text = await readFile(file, 'utf8');
  const m = text.match(/<g[\s\S]*?<\/g>/);
  if (!m) fail(`brand/2-pyramid-${name}.svg 里找不到 <g> 几何组`);
  if (!/<g[\s\S]*stroke-width=/.test(m[0])) fail(`brand/2-pyramid-${name}.svg 的 <g> 里没有 stroke-width`);
  return { inner: m[0] };
}

function marginFor(size) {
  if (size <= MARGIN_STOPS[0][0]) return MARGIN_STOPS[0][1];
  const last = MARGIN_STOPS[MARGIN_STOPS.length - 1];
  if (size >= last[0]) return size === last[0] ? last[1] : Math.round(size * 0.09);
  for (let i = 0; i < MARGIN_STOPS.length - 1; i++) {
    const [s0, m0] = MARGIN_STOPS[i];
    const [s1, m1] = MARGIN_STOPS[i + 1];
    if (size >= s0 && size <= s1) return m0 + ((size - s0) / (s1 - s0)) * (m1 - m0);
  }
  return Math.round(size * 0.09);
}

function radiusFor(size) {
  const r = size * 0.22;
  if (size <= 16) return Math.max(r, 3); // 契约：16px 圆角至少 3px
  if (size <= 24) return Math.max(r, 4); // 契约：24px 圆角至少 4px
  return r;
}

function artFor(size) {
  if (size <= 24) return 'micro';
  if (size < 64) return 'favicon'; // 契约点名的 32/48 档；MSIX StoreLogo(50) 也落此档
  return 'mark';
}

// 把 artwork 几何组映射进「画板内缩框」：内缩框 side = size - 2*margin
function artTransform(size, margin) {
  const content = size - margin * 2;
  const scale = content / ART_VIEWBOX.w;
  const tx = margin - ART_VIEWBOX.x * scale;
  const ty = margin - ART_VIEWBOX.y * scale;
  return { content, scale, tx, ty };
}

// ---------------------------------------------------------------------------
// SVG 组装 + sharp 光栅化
// ---------------------------------------------------------------------------

function plateRect(w, h, radius) {
  return `<rect x="0" y="0" width="${w}" height="${h}" rx="${radius.toFixed(3)}" ry="${radius.toFixed(3)}" fill="${PLATE_FILL}"/>`;
}

function squareSvg(size, art) {
  const margin = marginFor(size);
  const radius = radiusFor(size);
  const { scale, tx, ty } = artTransform(size, margin);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">
  ${plateRect(size, size, radius)}
  <g transform="translate(${tx.toFixed(4)} ${ty.toFixed(4)}) scale(${scale.toFixed(6)})">${art.inner}</g>
</svg>`;
}

// 宽/高不相等的 MSIX 资产（Wide / Splash）：整张圆角白板 + 居中 mark，mark 可见墨迹高度 = inkH
function wideSvg(w, h, inkH, art) {
  const radius = Math.min(w, h) * 0.22;
  const canvas = inkH / MARK_INK_RATIO; // mark 画布（430 裁剪框）边长
  const scale = canvas / ART_VIEWBOX.w;
  const tx = (w - canvas) / 2 - ART_VIEWBOX.x * scale;
  const ty = (h - canvas) / 2 - ART_VIEWBOX.y * scale;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">
  ${plateRect(w, h, radius)}
  <g transform="translate(${tx.toFixed(4)} ${ty.toFixed(4)}) scale(${scale.toFixed(6)})">${art.inner}</g>
</svg>`;
}

async function rasterize(svg, expectedW, expectedH, label) {
  const png = await sharp(Buffer.from(svg)).png({ compressionLevel: 9, adaptiveFiltering: true }).toBuffer();
  const meta = await sharp(png).metadata();
  if (meta.width !== expectedW || meta.height !== expectedH) {
    fail(`${label}: 光栅化尺寸 ${meta.width}x${meta.height}，期望 ${expectedW}x${expectedH}`);
  }
  return png;
}

// ---------------------------------------------------------------------------
// 自实现 ICO / ICNS 容器（只用 node Buffer，无第三方图标库）
// ---------------------------------------------------------------------------

function buildIco(frames) {
  // ICONDIR(6B) + ICONDIRENTRY(16B)×N + PNG 数据
  const n = frames.length;
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type=icon
  header.writeUInt16LE(n, 4);
  const dir = Buffer.alloc(16 * n);
  let offset = 6 + 16 * n;
  const blobs = [];
  frames.forEach((f, i) => {
    const o = i * 16;
    const dim = f.size >= 256 ? 0 : f.size; // 0 表示 256
    dir[o] = dim;
    dir[o + 1] = dim;
    dir[o + 2] = 0; // color count
    dir[o + 3] = 0; // reserved
    dir.writeUInt16LE(1, o + 4); // planes
    dir.writeUInt16LE(32, o + 6); // bit count（PNG 帧统一标 32bpp）
    dir.writeUInt32LE(f.png.length, o + 8);
    dir.writeUInt32LE(offset, o + 12);
    offset += f.png.length;
    blobs.push(f.png);
  });
  return Buffer.concat([header, dir, ...blobs]);
}

function parseIco(buffer) {
  assert(buffer.length > 6, 'ICO 文件过短');
  assert(buffer.readUInt16LE(0) === 0, 'ICO reserved 字段不是 0');
  assert(buffer.readUInt16LE(2) === 1, 'ICO type 字段不是 1');
  const count = buffer.readUInt16LE(4);
  const frames = [];
  for (let i = 0; i < count; i++) {
    const o = 6 + i * 16;
    const w = buffer[o] === 0 ? 256 : buffer[o];
    const h = buffer[o + 1] === 0 ? 256 : buffer[o + 1];
    const planes = buffer.readUInt16LE(o + 4);
    const bitCount = buffer.readUInt16LE(o + 6);
    const size = buffer.readUInt32LE(o + 8);
    const dataOff = buffer.readUInt32LE(o + 12);
    assert(dataOff + size <= buffer.length, `ICO 第 ${i} 帧数据越界`);
    const data = buffer.subarray(dataOff, dataOff + size);
    assert(data[0] === 0x89 && data[1] === 0x50 && data[2] === 0x4e && data[3] === 0x47, `ICO 第 ${i} 帧不是 PNG`);
    const pngW = data.readUInt32BE(16);
    const pngH = data.readUInt32BE(20);
    frames.push({ w, h, planes, bitCount, size, data, pngW, pngH });
  }
  return frames;
}

const ICNS_TYPES = [
  ['icp4', 16],
  ['icp5', 32],
  ['icp6', 64],
  ['ic07', 128],
  ['ic08', 256],
  ['ic09', 512],
];

function buildIcns(frames) {
  // 'icns' + 总长度(BE) + [4B type + 4B 长度(BE，含头) + PNG]×N
  const elements = frames.map((f) => {
    const head = Buffer.alloc(8);
    head.write(f.type, 0, 4, 'ascii');
    head.writeUInt32BE(f.png.length + 8, 4);
    return Buffer.concat([head, f.png]);
  });
  const body = Buffer.concat(elements);
  const head = Buffer.alloc(8);
  head.write('icns', 0, 4, 'ascii');
  head.writeUInt32BE(body.length + 8, 4);
  return Buffer.concat([head, body]);
}

function parseIcns(buffer) {
  assert(buffer.subarray(0, 4).toString('ascii') === 'icns', 'ICNS magic 错误');
  assert(buffer.readUInt32BE(4) === buffer.length, 'ICNS 总长度字段与文件大小不一致');
  const frames = [];
  let off = 8;
  while (off + 8 <= buffer.length) {
    const type = buffer.subarray(off, off + 4).toString('ascii');
    const len = buffer.readUInt32BE(off + 4);
    assert(len >= 8 && off + len <= buffer.length, `ICNS 元素 ${type} 长度越界`);
    const data = buffer.subarray(off + 8, off + len);
    assert(data[0] === 0x89 && data[1] === 0x50 && data[2] === 0x4e && data[3] === 0x47, `ICNS ${type} 不是 PNG`);
    frames.push({ type, data, pngW: data.readUInt32BE(16), pngH: data.readUInt32BE(20), bytes: len });
    off += len;
  }
  return frames;
}

// ---------------------------------------------------------------------------
// 像素级自检（sharp 解码后逐像素）
// ---------------------------------------------------------------------------

async function inspectPng(png, label) {
  const { data, info } = await sharp(png).ensureAlpha().raw().toBuffer({ resolveWithObject: true });
  const { width: w, height: h } = info;
  const alphaAt = (x, y) => data[(y * w + x) * 4 + 3];
  const corners = [
    alphaAt(0, 0),
    alphaAt(w - 1, 0),
    alphaAt(0, h - 1),
    alphaAt(w - 1, h - 1),
  ];
  const center = alphaAt(w >> 1, h >> 1);
  let dark = 0;
  for (let i = 0; i < data.length; i += 4) {
    if (data[i + 3] === 0) continue;
    const luma = 0.2126 * data[i] + 0.7152 * data[i + 1] + 0.0722 * data[i + 2];
    if (luma < 128) dark++;
  }
  assert(corners.every((a) => a === 0), `${label}: 四角 alpha 应为 0，实测 [${corners.join(',')}]`);
  assert(center === 255, `${label}: 中心 alpha 应为 255，实测 ${center}`);
  return { w, h, corners, center, dark };
}

// ---------------------------------------------------------------------------
// 主流程
// ---------------------------------------------------------------------------

async function writeFileChecked(relPath, absPath, png, w, h, label) {
  await writeFile(absPath, png);
  const back = await readFile(absPath);
  assert(back.length === png.length, `${relPath}: 回读字节数不一致`);
  const meta = await sharp(back).metadata();
  assert(meta.width === w && meta.height === h, `${relPath}: 回读尺寸 ${meta.width}x${meta.height}，期望 ${w}x${h}`);
  files.push({ path: relPath, w, h, bytes: back.length, label });
}

// 非 PNG 容器（ICO/ICNS）：只做字节回读一致性校验，尺寸在各自的 parser 里断言
async function writeContainerChecked(relPath, absPath, buffer, label) {
  await writeFile(absPath, buffer);
  const back = await readFile(absPath);
  assert(back.equals(buffer), `${relPath}: 回读内容与写入不一致`);
  files.push({ path: relPath, w: null, h: null, bytes: back.length, label });
}

async function main() {
  const t0 = Date.now();
  console.log('2-Pyramid OS 图标生成器（白底圆角板 + 分档 artwork）');
  console.log(`  仓库根: ${ROOT}`);

  // 1) 读取品牌 SVG（只读输入）
  const art = {
    mark: await loadArt('mark'),
    favicon: await loadArt('favicon'),
    micro: await loadArt('micro'),
  };
  for (const k of Object.keys(art)) {
    console.log(`  输入 brand/2-pyramid-${k}.svg  ✓`);
  }

  await mkdir(ICONS, { recursive: true });
  await mkdir(INSTALLER_ICONS, { recursive: true });

  // 2) 主图标 PNG 集合
  const png32 = await rasterize(squareSvg(32, art.favicon), 32, 32, '32x32');
  const png64 = await rasterize(squareSvg(64, art.mark), 64, 64, '64x64');
  const png128 = await rasterize(squareSvg(128, art.mark), 128, 128, '128x128');
  const png256 = await rasterize(squareSvg(256, art.mark), 256, 256, '128x128@2x');
  const png512 = await rasterize(squareSvg(512, art.mark), 512, 512, 'icon.png');

  await writeFileChecked('src-tauri/icons/32x32.png', resolve(ICONS, '32x32.png'), png32, 32, 32, '');
  await writeFileChecked('src-tauri/icons/64x64.png', resolve(ICONS, '64x64.png'), png64, 64, 64, '');
  await writeFileChecked('src-tauri/icons/128x128.png', resolve(ICONS, '128x128.png'), png128, 128, 128, '');
  await writeFileChecked('src-tauri/icons/128x128@2x.png', resolve(ICONS, '128x128@2x.png'), png256, 256, 256, '= 256');
  await writeFileChecked('src-tauri/icons/icon.png', resolve(ICONS, 'icon.png'), png512, 512, 512, '');

  // 3) ICO：6 帧，尺寸恰好 [16,24,32,48,64,256]，每档用对应 artwork
  const icoSizes = [16, 24, 32, 48, 64, 256];
  const icoFrames = [];
  for (const size of icoSizes) {
    const png = await rasterize(squareSvg(size, art[artFor(size)]), size, size, `ico ${size}px`);
    icoFrames.push({ size, png, art: artFor(size) });
  }
  const icoBuf = buildIco(icoFrames.map((f) => ({ size: f.size, png: f.png })));
  await writeContainerChecked('src-tauri/icons/icon.ico', resolve(ICONS, 'icon.ico'), icoBuf, '6 帧 PNG，尺寸恰好 [16,24,32,48,64,256]');

  // ICO 断言：尺寸集合、每帧 PNG 尺寸、planes/bitCount、深色像素下限
  const icoBack = await readFile(resolve(ICONS, 'icon.ico'));
  assert(icoBack.equals(icoBuf), 'icon.ico 回读内容与写入内容不一致');
  const parsedIco = parseIco(icoBack);
  assert(parsedIco.length === 6, `ICO 帧数应为 6，实测 ${parsedIco.length}`);
  const gotSizes = parsedIco.map((f) => f.w);
  assert(
    JSON.stringify(gotSizes) === JSON.stringify(icoSizes),
    `ICO 尺寸集合应为 [${icoSizes}]，实测 [${gotSizes}]`
  );
  const darkCounts = {};
  for (let i = 0; i < parsedIco.length; i++) {
    const f = parsedIco[i];
    assert(f.w === f.h, `ICO 第 ${i} 帧不是正方形 (${f.w}x${f.h})`);
    assert(f.pngW === f.w && f.pngH === f.h, `ICO ${f.w}px 帧内 PNG 尺寸 ${f.pngW}x${f.pngH} 不符`);
    assert(f.planes === 1, `ICO ${f.w}px planes 应为 1`);
    assert(f.bitCount === 32, `ICO ${f.w}px bitCount 应为 32`);
    const px = await inspectPng(f.data, `ICO ${f.w}px`);
    assert(px.w === f.w && px.h === f.h, `ICO ${f.w}px 解码尺寸不符`);
    darkCounts[f.w] = px.dark;
  }
  for (const [size, min] of Object.entries(DARK_MIN)) {
    assert(
      darkCounts[size] >= min,
      `ICO ${size}px 深色像素 ${darkCounts[size]} < 下限 ${min}（小尺寸会接近空白）`
    );
  }

  // 4) ICNS（Windows-only 项目，但 tauri.conf 引用 icon.icns，必须保持有效）
  const icnsFrames = [];
  for (const [type, size] of ICNS_TYPES) {
    const png = await rasterize(squareSvg(size, art[artFor(size)]), size, size, `icns ${type}`);
    icnsFrames.push({ type, size, png });
  }
  const icnsBuf = buildIcns(icnsFrames);
  await writeContainerChecked('src-tauri/icons/icon.icns', resolve(ICONS, 'icon.icns'), icnsBuf, '6 帧 PNG：icp4/icp5/icp6/ic07/ic08/ic09');
  const parsedIcns = parseIcns(await readFile(resolve(ICONS, 'icon.icns')));
  assert(parsedIcns.length === ICNS_TYPES.length, `ICNS 元素数应为 6，实测 ${parsedIcns.length}`);
  parsedIcns.forEach((f, i) => {
    const [type, expect] = ICNS_TYPES[i];
    assert(f.type === type, `ICNS 第 ${i} 项类型应为 ${type}，实测 ${f.type}`);
    assert(f.pngW === expect && f.pngH === expect, `ICNS ${type} 内 PNG 应为 ${expect}px，实测 ${f.pngW}x${f.pngH}`);
  });

  // 5) MSIX 正方形资产
  for (const [name, size] of MSIX_SQUARES) {
    const png = await rasterize(squareSvg(size, art[artFor(size)]), size, size, name);
    await writeFileChecked(`src-tauri/icons/${name}`, resolve(ICONS, name), png, size, size, `MSIX, art=${artFor(size)}`);
  }

  // 6) MSIX Wide / Splash（白底圆角板铺满 + 居中 mark，不依赖横向文字/转曲资产）
  for (const spec of [MSIX_WIDE, MSIX_SPLASH]) {
    const png = await rasterize(wideSvg(spec.w, spec.h, spec.inkH, art.mark), spec.w, spec.h, spec.name);
    await writeFileChecked(`src-tauri/icons/${spec.name}`, resolve(ICONS, spec.name), png, spec.w, spec.h, `MSIX, mark 墨迹高 ≈${spec.inkH}px`);
  }

  // 7) 安装器图标（与主集合同字节/同尺寸拷贝）
  await writeContainerChecked(
    'installer-app/src-tauri/icons/icon.ico',
    resolve(INSTALLER_ICONS, 'icon.ico'),
    icoBuf,
    '= 主 icon.ico 同字节'
  );
  assert(
    (await readFile(resolve(INSTALLER_ICONS, 'icon.ico'))).equals(icoBack),
    'installer icon.ico 与主 icon.ico 不同字节'
  );
  const installerPngs = [
    ['icon.png', png512, 512],
    ['32x32.png', png32, 32],
    ['128x128.png', png128, 128],
    ['128x128@2x.png', png256, 256],
  ];
  for (const [name, png, size] of installerPngs) {
    await writeFileChecked(`installer-app/src-tauri/icons/${name}`, resolve(INSTALLER_ICONS, name), png, size, size, '');
  }

  // 8) 全部 PNG 的像素级自检（四角透明 / 中心不透明）。ICO/ICNS 帧已在上面检查过。
  for (const f of files) {
    if (!f.w) continue;
    const buf = await readFile(resolve(ROOT, f.path));
    const px = await inspectPng(buf, f.path);
    assert(px.w === f.w && px.h === f.h, `${f.path}: 尺寸 ${px.w}x${px.h}，期望 ${f.w}x${f.h}`);
  }

  // 9) 打印清单
  console.log('\n生成清单:');
  for (const f of files) {
    const dims = f.w ? `${f.w}×${f.h}` : '（容器）';
    console.log(
      `  ✓ ${f.path.padEnd(46)} ${dims.padEnd(9)} ${fmtBytes(f.bytes).padStart(9)} B  ${f.label || ''}`
    );
  }
  console.log('\nICO 深色像素（alpha>0 且 luma<128）:');
  for (const [size, min] of Object.entries(DARK_MIN)) {
    console.log(`  ${String(size).padStart(2)}px: ${String(darkCounts[size]).padStart(3)}  (下限 ${min})`);
  }
  console.log(
    `\n自检 ${assertCount} 项全部通过 · 文件 ${files.length} 个 · 用时 ${((Date.now() - t0) / 1000).toFixed(1)}s`
  );
  console.log('完成。生成器可重复执行（覆盖同名文件，不新增/不删除文件名）。');
}

main().catch((e) => {
  console.error('\n[generate-icons] 失败:');
  console.error(e);
  process.exit(1);
});
