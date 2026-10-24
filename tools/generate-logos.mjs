// 从 brand/ 下的品牌 SVG 源生成全部 logo PNG(可复现)。
// 用法: node tools/generate-logos.mjs  (或 npm run logos)
//
// ⚠️ 严禁把 logo PNG 写到 src-tauri/UImage/ 下的任何路径!
// UImage/ 是打包到最终安装目录的资源目录,会被 overlay_icons /
// generate_crossbow / overlay_icons 等转换器当作资源包覆盖层读
// 取。任何把 logo 误写到这里的行为都会让转换后的资源包被 logo
// 污染(实测: 2-Pyramid logo 出现在 icons.png 上)。
// 本脚本只允许写以下白名单位置,新增目标请先核对:
import sharp from 'sharp';
import { mkdir, copyFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { dirname, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, '..');
const BRAND = resolve(ROOT, 'brand');

// 品牌源(与 brand/README.md 中的资产用途表一一对应)
const SRC = {
  mark: resolve(BRAND, '2-pyramid-mark.svg'),                  // 512 画布,主标识(大尺寸)
  markOnDark: resolve(BRAND, '2-pyramid-mark-on-dark.svg'),    // 白色版,深色背景
  favicon: resolve(BRAND, '2-pyramid-favicon.svg'),            // 加粗版,≤48px
  faviconWhite: resolve(BRAND, '2-pyramid-favicon-white.svg'), // 白色加粗版,深色背景小尺寸
};

const ALLOWED_DIRS = new Set([
  resolve(ROOT, 'public'),         // favicon / 开屏 / 兼容名
  resolve(ROOT, 'src/assets'),     // HomePage / SettingsPage / toast logo
]);

// src 的 SVG 在浏览器中的固有尺寸(见各文件 width/height),用于换算 sharp 的 density。
// density 决定光栅化清晰度: 先按目标尺寸的 SUPERSAMPLE 倍渲染,再缩放到目标尺寸,
// 保证 1x 下边缘干净(旧版按 1.5x 太保守,512 输出时边缘仍偏软)。
const SUPERSAMPLE = 2;

const ASSETS = resolve(ROOT, 'src/assets');
const PUBLIC = resolve(ROOT, 'public');

// 输出清单是契约(文件名/用途勿改): 应用内用 src/assets, 运行时/网页用 public。
const TASKS = [
  // HomePage 大图(mask 用,需要 alpha)
  { src: SRC.mark, out: 'logo-256.png', size: 256, dir: ASSETS, intrinsic: 512 },
  // 通用高清 logo(可给 SettingsPage / toast 等)
  { src: SRC.mark, out: 'logo-512.png', size: 512, dir: ASSETS, intrinsic: 512 },
  // README <picture> 的深色主题版本 / 深色背景用
  { src: SRC.markOnDark, out: 'logo-256-white.png', size: 256, dir: ASSETS, intrinsic: 512 },
  // 浏览器 tab favicon(小尺寸加粗版)
  { src: SRC.favicon, out: 'favicon.png', size: 32, dir: PUBLIC, intrinsic: 32 },
  { src: SRC.favicon, out: 'favicon-192.png', size: 192, dir: PUBLIC, intrinsic: 32 },
  // 开屏 / 通用 mark(与 src/assets/logo-512.png 同图)
  { src: SRC.mark, out: 'logo-mark.png', size: 512, dir: PUBLIC, intrinsic: 512 },
  // 旧兼容名(index.html 引导 / 外部引用),与 logo-mark.png 同图
  { src: SRC.mark, out: 'logo-transparent.png', size: 512, dir: PUBLIC, intrinsic: 512 },
  // 深色背景用白色线条 + 透明底(旧文件曾是白底不透明,必须修)
  { src: SRC.markOnDark, out: 'logo-white.png', size: 512, dir: PUBLIC, intrinsic: 512 },
  // VersionInfoDialog 用图(新)
  { src: SRC.mark, out: 'logo-192.png', size: 192, dir: PUBLIC, intrinsic: 512 },
];

function assertSafeOutputDir(targetDir) {
  // 1) 必须落在 ALLOWED_DIRS 之一
  let allowed = false;
  for (const root of ALLOWED_DIRS) {
    if (targetDir === root || targetDir.startsWith(root + sep)) {
      allowed = true;
      break;
    }
  }
  if (!allowed) {
    throw new Error(
      `Refusing to write logo PNG outside the allow-list.\n` +
      `  target = ${targetDir}\n` +
      `  allowed = ${[...ALLOWED_DIRS].join(', ')}\n` +
      `  If you really need a new location, add it to ALLOWED_DIRS in ` +
      `tools/generate-logos.mjs.`
    );
  }
  // 2) 严禁写到 src-tauri/UImage/ — 这是打包资源目录,会被覆盖到最终资源包
  const sepN = sep === '/' ? /[/\\]/ : new RegExp(`\\${sep}`);
  const pathParts = targetDir.split(sepN);
  if (pathParts.includes('UImage')) {
    throw new Error(
      `Refusing to write logo PNG into UImage/ — that directory is bundled ` +
      `into installed packs and will pollute converted resource packs. ` +
      `Target was: ${targetDir}`
    );
  }
}

function rel(p) {
  return p.replace(ROOT + sep, '');
}

async function renderOne(t) {
  assertSafeOutputDir(t.dir);
  if (!existsSync(t.src)) {
    throw new Error(`Brand source SVG missing: ${rel(t.src)} (expected in brand/)`);
  }
  if (!existsSync(t.dir)) await mkdir(t.dir, { recursive: true });
  const out = resolve(t.dir, t.out);
  const density = Math.max(72, Math.ceil((t.size * SUPERSAMPLE * 72) / t.intrinsic));
  await sharp(t.src, { density })
    .resize(t.size, t.size, {
      fit: 'contain',
      background: { r: 0, g: 0, b: 0, alpha: 0 },
    })
    .png()
    .toFile(out);
  const meta = await sharp(out).metadata();
  console.log(
    `  ✓ ${rel(out).padEnd(30)} ${meta.width}×${meta.height}  alpha=${meta.hasAlpha ? 'yes' : 'NO'}`
  );
}

async function main() {
  console.log(`brand source : ${rel(BRAND)}`);
  console.log(`outputs      : ${TASKS.length} PNG + app-icon copy (= contract names)`);
  for (const t of TASKS) await renderOne(t);

  // app-icon.png 必须与 logo-256.png 同图(HomePage 旧引用名保留)
  const logo256 = resolve(ASSETS, 'logo-256.png');
  const appIcon = resolve(ASSETS, 'app-icon.png');
  await copyFile(logo256, appIcon);
  const iconMeta = await sharp(appIcon).metadata();
  console.log(
    `  ✓ ${rel(appIcon).padEnd(30)} ${iconMeta.width}×${iconMeta.height}  ` +
    `(copy of logo-256.png)`
  );

  console.log('done.');
}
main().catch((e) => { console.error(e); process.exit(1); });
