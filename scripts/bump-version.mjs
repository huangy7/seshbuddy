// ==============================================================================
// SeshBuddy — 跨生态版本号同步脚本 (Node package.json ↔ Cargo workspace)
// ==============================================================================
// 为什么必须存在：
// SeshBuddy 是前端 (Vite/Vue) 与后端 (Tauri/Rust Cargo Workspace) 双生态架构。
// 客户端安装包分发与 release 闸门以 package.json 为基准，而 Rust 编译产物以
// 根目录 Cargo.toml ([workspace.package]) 为基准。如果版本号只改一处，CI 闸门
// 或客户端检查逻辑将因版本分裂而阻断。
//
// 本脚本将版本号更新收敛为单一入口：
// 1. 校验目标版本符合 SemVer 格式
// 2. 调用 npm 同步 package.json 与 package-lock.json
// 3. 更新根目录 Cargo.toml 的 [workspace.package].version
// 4. 调用 cargo check --workspace 自动刷新 Cargo.lock
//
// 用法：
//   node scripts/bump-version.mjs <new-version>
//   例如：npm run bump 0.1.1
// ==============================================================================

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const PROJECT_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/**
 * 校验并规范化版本号字符串。
 * 允许传入带 'v' 前缀或纯数字版本，如 'v0.1.1' 或 '0.1.1'。
 */
export function validateVersion(version) {
  if (!version || typeof version !== "string") {
    throw new Error("版本号不能为空");
  }
  const clean = version.startsWith("v") ? version.slice(1) : version;
  if (!/^\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?$/.test(clean)) {
    throw new Error(`非法版本号格式: '${version}'，必须符合 SemVer 规范（例如 0.1.1 或 0.1.1-beta.1）`);
  }
  return clean;
}

/**
 * 更新根目录 Cargo.toml 内容中的 [workspace.package].version 字段。
 */
export function updateCargoTomlContent(content, newVersion) {
  const regex = /(\[workspace\.package\][\s\S]*?version\s*=\s*")[^"]+(")/;
  if (!regex.test(content)) {
    throw new Error("根目录 Cargo.toml 中未找到 [workspace.package] 的 version 字段定义");
  }
  return content.replace(regex, `$1${newVersion}$2`);
}

function run() {
  const rawTarget = process.argv[2];
  if (!rawTarget) {
    console.error("错误: 未指定目标版本号。");
    console.error("用法: node scripts/bump-version.mjs <new-version>");
    console.error("示例: npm run bump 0.1.1");
    process.exit(1);
  }

  const version = validateVersion(rawTarget);
  console.log(`\n正在将 SeshBuddy 全局版本号同步为 v${version}...`);

  // 1. 同步 package.json 与 package-lock.json
  console.log("-> 更新 package.json 与 package-lock.json");
  execFileSync("npm", ["version", version, "--no-git-tag-version", "--allow-same-version"], {
    cwd: PROJECT_ROOT,
    stdio: "inherit",
  });

  // 2. 更新根目录 Cargo.toml
  console.log("-> 更新根目录 Cargo.toml [workspace.package].version");
  const cargoPath = resolve(PROJECT_ROOT, "Cargo.toml");
  const cargoContent = readFileSync(cargoPath, "utf-8");
  const updatedCargo = updateCargoTomlContent(cargoContent, version);
  writeFileSync(cargoPath, updatedCargo, "utf-8");

  // 3. 刷新 Cargo.lock
  console.log("-> 触发 cargo check --workspace 刷新 Cargo.lock");
  execFileSync("cargo", ["check", "--workspace"], {
    cwd: PROJECT_ROOT,
    stdio: "inherit",
  });

  console.log(`\n✓ 版本号已全量成功同步至 ${version}！\n`);
}

// 仅在作为脚本直接执行时运行
const isDirectRun =
  process.argv[1] &&
  resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url));

if (isDirectRun) {
  run();
}
