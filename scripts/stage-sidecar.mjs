// ==============================================================================
// SeshBuddy — 把 seshbuddy-proxy 按 sidecar 约定摆到 src-tauri/binaries/
// ==============================================================================
// 为什么必须存在：tauri.conf.json 的 bundle.externalBin 声明为
// "binaries/seshbuddy-proxy"，tauri-build 在构建脚本阶段把它展开成
// binaries/seshbuddy-proxy-<TARGET> 并校验文件存在。缺失时任何 cargo check /
// cargo build 都会失败，且没有任何配置项可以关掉这个校验。
//
// 为什么 universal 要三份：tauri CLI 会先对 aarch64 与 x86_64 各跑一次构建
// （每次 build.rs 校验的是各自的 triple），再 lipo 出主程序，最后 bundler 找的是
// universal 那一份。sidecar 不参与 lipo，必须自己合成。
//
// 用法：
//   node scripts/stage-sidecar.mjs [--profile dev|release]
// ==============================================================================

import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const PROJECT_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// 导出给测试用：crate 名同时出现在 Cargo.toml、tauri.conf.json 的 externalBin、
// src-tauri/src/proxy.rs 的运行时查找里，改名时几处必须一起改。
export const PROXY_CRATE = "seshbuddy-proxy";
const UNIVERSAL_TRIPLE = "universal-apple-darwin";
const UNIVERSAL_ARCHES = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

// lipo -archs 打印的是 CPU 架构名而非 triple。本脚本只在 universal-apple-darwin
// 目标下合成 lipo 产物（planStaging 对其他平台直接抛错），所以这里可以硬编码
// 期望值，不必从 triple 反推。
const UNIVERSAL_EXPECTED_LIPO_ARCHS = ["arm64", "x86_64"];

export function binaryFileName(platform = process.platform) {
  return platform === "win32" ? `${PROXY_CRATE}.exe` : PROXY_CRATE;
}

export function sidecarName(triple, platform = process.platform) {
  return `${PROXY_CRATE}-${triple}${platform === "win32" ? ".exe" : ""}`;
}

// cargo 只在传了 --target 时才多出一层 triple 子目录，没传则落在 target/<profile>/。
// 本机构建走后者，以复用开发者已有的构建缓存。
export function artifactPath({ triple, profile, hostTriple, platform = process.platform }) {
  const parts = ["target"];
  if (triple !== hostTriple) {
    parts.push(triple);
  }
  parts.push(profile === "release" ? "release" : "debug", binaryFileName(platform));
  return parts.join("/");
}

export function resolveTriple({ env = process.env, hostTriple }) {
  return env.TAURI_ENV_TARGET_TRIPLE ?? hostTriple;
}

export function resolveProfile({ argv = [], env = process.env }) {
  const index = argv.indexOf("--profile");
  if (index !== -1) {
    const value = argv[index + 1];
    if (value !== "dev" && value !== "release") {
      throw new Error(`--profile 只接受 dev 或 release，收到：${value}`);
    }
    return value;
  }

  // tauri CLI 只在 debug 构建时注入 TAURI_ENV_DEBUG，release 构建下该变量不存在。
  // 所以「在 hook 里但没有 TAURI_ENV_DEBUG」必须判定为 release —— CI 的
  // tauri build 正是这个场景，判错就会给发布包配一个 debug 版 proxy。
  if (env.TAURI_ENV_TARGET_TRIPLE !== undefined) {
    return env.TAURI_ENV_DEBUG !== undefined ? "dev" : "release";
  }

  // 不在 hook 环境里（手工调用），按开发用途默认 dev
  return "dev";
}

export function planStaging({ triple, profile, hostTriple, platform = process.platform }) {
  const toSidecar = (name) => `src-tauri/binaries/${name}`;

  if (triple === UNIVERSAL_TRIPLE) {
    if (platform !== "darwin") {
      throw new Error(
        `目标 ${UNIVERSAL_TRIPLE} 只能在 macOS 上构建（需要 lipo），当前平台为 ${platform}`
      );
    }

    const archArtifacts = UNIVERSAL_ARCHES.map((arch) => ({
      triple: arch,
      path: artifactPath({ triple: arch, profile, hostTriple, platform }),
    }));

    return {
      builds: UNIVERSAL_ARCHES.map((arch) => ({
        triple: arch,
        useTargetFlag: arch !== hostTriple,
        profile,
      })),
      lipos: [
        {
          inputs: archArtifacts.map((artifact) => artifact.path),
          output: toSidecar(sidecarName(UNIVERSAL_TRIPLE, platform)),
        },
      ],
      copies: archArtifacts.map((artifact) => ({
        from: artifact.path,
        to: toSidecar(sidecarName(artifact.triple, platform)),
      })),
    };
  }

  return {
    builds: [{ triple, useTargetFlag: triple !== hostTriple, profile }],
    lipos: [],
    copies: [
      {
        from: artifactPath({ triple, profile, hostTriple, platform }),
        to: toSidecar(sidecarName(triple, platform)),
      },
    ],
  };
}

function detectHostTriple() {
  const output = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const match = output.match(/^host: (.+)$/m);
  if (!match) {
    throw new Error("无法从 rustc -vV 解析 host triple");
  }
  return match[1];
}

// 子进程的 stdio 直接继承、不捕获：cargo 在等 target 目录锁时会打印
// "Blocking waiting for file lock"，一旦把输出吞掉，这类挂死就变成完全静默的卡死。
function run(command, args) {
  const result = spawnSync(command, args, { stdio: "inherit", cwd: PROJECT_ROOT });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} 退出码为 ${result.status}`);
  }
}

// 需要拿输出做判断时用这个变体：输出要参与断言，就不能继承到终端里。
function capture(command, args) {
  const result = spawnSync(command, args, { encoding: "utf8", cwd: PROJECT_ROOT });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} 退出码为 ${result.status}`);
  }
  return result.stdout.trim();
}

function main() {
  const argv = process.argv.slice(2);
  const hostTriple = detectHostTriple();
  const triple = resolveTriple({ hostTriple });
  const profile = resolveProfile({ argv });
  const plan = planStaging({ triple, profile, hostTriple });

  const cargo = process.env.CARGO || "cargo";
  for (const build of plan.builds) {
    const args = ["build", "-p", PROXY_CRATE];
    if (build.profile === "release") {
      args.push("--release");
    }
    if (build.useTargetFlag) {
      args.push("--target", build.triple);
    }
    run(cargo, args);
  }

  const lipoOutputs = [];
  for (const lipo of plan.lipos) {
    const output = join(PROJECT_ROOT, lipo.output);
    mkdirSync(dirname(output), { recursive: true });
    run("lipo", [
      "-create",
      "-output",
      output,
      ...lipo.inputs.map((input) => join(PROJECT_ROOT, input)),
    ]);

    // lipo -create 退出码 0 只说明它没报错：输入给成同一架构的两份，它一样成功，
    // 产出的「universal」包在另一架构上根本跑不起来。读回真实架构再断言一次，
    // 否则问题要等到用户机器上才暴露，而那时已经发布出去了。
    const arches = capture("lipo", ["-archs", output]).split(/\s+/);
    const missing = UNIVERSAL_EXPECTED_LIPO_ARCHS.filter((arch) => !arches.includes(arch));
    if (missing.length > 0) {
      throw new Error(
        `${lipo.output} 缺少架构 ${missing.join(", ")}，实际只含 [${arches.join(", ")}]`
      );
    }
    lipoOutputs.push({ path: lipo.output, arches });
  }

  for (const copy of plan.copies) {
    const to = join(PROJECT_ROOT, copy.to);
    mkdirSync(dirname(to), { recursive: true });
    copyFileSync(join(PROJECT_ROOT, copy.from), to);
    // sidecar 会被 Tauri 直接当可执行文件拉起，缺执行位时只在打包阶段才报错，信息很隐晦
    chmodSync(to, 0o755);
  }

  console.log(`[INFO] sidecar 就位（target=${triple}, profile=${profile}）：`);
  for (const copy of plan.copies) {
    console.log(`  ${copy.to}`);
  }
  for (const lipo of lipoOutputs) {
    console.log(`  ${lipo.path}（lipo -archs: ${lipo.arches.join(", ")}）`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
