import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  PROXY_CRATE,
  artifactPath,
  planStaging,
  resolveProfile,
  resolveTriple,
  sidecarName,
} from "./stage-sidecar.mjs";

const DARWIN_ARM = "aarch64-apple-darwin";
const DARWIN_X64 = "x86_64-apple-darwin";
const WINDOWS = "x86_64-pc-windows-msvc";

// 相对测试文件定位，避免测试结果随调用时的 cwd 变化
const PROJECT_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

describe("crate 名一致性", () => {
  it("tauri.conf.json 的 externalBin 条目去掉 binaries/ 前缀后等于 PROXY_CRATE", () => {
    const config = JSON.parse(
      readFileSync(join(PROJECT_ROOT, "src-tauri", "tauri.conf.json"), "utf8")
    );
    const stems = config.bundle.externalBin.map((entry) => entry.replace(/^binaries\//, ""));

    expect(stems).toEqual([PROXY_CRATE]);
  });
});

describe("resolveTriple", () => {
  it("hook 注入的 TAURI_ENV_TARGET_TRIPLE 优先于 host", () => {
    expect(
      resolveTriple({
        env: { TAURI_ENV_TARGET_TRIPLE: "universal-apple-darwin" },
        hostTriple: DARWIN_ARM,
      })
    ).toBe("universal-apple-darwin");
  });

  it("无 hook 变量时回退到 host triple", () => {
    expect(resolveTriple({ env: {}, hostTriple: DARWIN_ARM })).toBe(DARWIN_ARM);
  });
});

describe("resolveProfile", () => {
  it("--profile 显式指定时以其为准", () => {
    expect(resolveProfile({ argv: ["--profile", "release"], env: {} })).toBe("release");
    expect(resolveProfile({ argv: ["--profile", "dev"], env: {} })).toBe("dev");
  });

  it("--profile 取值非法时报错", () => {
    expect(() => resolveProfile({ argv: ["--profile", "prod"], env: {} })).toThrow(
      /只接受 dev 或 release/
    );
  });

  it("在 tauri hook 里且无 TAURI_ENV_DEBUG 时是 release（CI 的 tauri build 场景）", () => {
    expect(
      resolveProfile({ argv: [], env: { TAURI_ENV_TARGET_TRIPLE: DARWIN_ARM } })
    ).toBe("release");
  });

  it("在 tauri hook 里且有 TAURI_ENV_DEBUG 时是 dev（tauri dev 场景）", () => {
    expect(
      resolveProfile({
        argv: [],
        env: { TAURI_ENV_TARGET_TRIPLE: DARWIN_ARM, TAURI_ENV_DEBUG: "true" },
      })
    ).toBe("dev");
  });

  it("不在 hook 环境里（手工调用）默认 dev", () => {
    expect(resolveProfile({ argv: [], env: {} })).toBe("dev");
  });
});

describe("artifactPath", () => {
  it("未传 --target 时产物在 target/<profile>/ 下", () => {
    expect(
      artifactPath({ triple: DARWIN_ARM, profile: "dev", hostTriple: DARWIN_ARM, platform: "darwin" })
    ).toBe("target/debug/seshbuddy-proxy");
  });

  it("传 --target 时产物多一层 triple 目录", () => {
    expect(
      artifactPath({ triple: DARWIN_X64, profile: "release", hostTriple: DARWIN_ARM, platform: "darwin" })
    ).toBe("target/x86_64-apple-darwin/release/seshbuddy-proxy");
  });

  it("Windows 产物带 .exe", () => {
    expect(
      artifactPath({ triple: WINDOWS, profile: "dev", hostTriple: WINDOWS, platform: "win32" })
    ).toBe("target/debug/seshbuddy-proxy.exe");
  });
});

describe("sidecarName", () => {
  it("按 triple 后缀命名", () => {
    expect(sidecarName(DARWIN_ARM, "darwin")).toBe("seshbuddy-proxy-aarch64-apple-darwin");
  });

  it("Windows 带 .exe 后缀", () => {
    expect(sidecarName(WINDOWS, "win32")).toBe(
      "seshbuddy-proxy-x86_64-pc-windows-msvc.exe"
    );
  });
});

describe("planStaging", () => {
  it("普通 host 构建：一次构建、一次拷贝、无 lipo", () => {
    const plan = planStaging({
      triple: DARWIN_ARM,
      profile: "dev",
      hostTriple: DARWIN_ARM,
      platform: "darwin",
    });
    expect(plan.builds).toEqual([{ triple: DARWIN_ARM, useTargetFlag: false, profile: "dev" }]);
    expect(plan.lipos).toEqual([]);
    expect(plan.copies).toEqual([
      {
        from: "target/debug/seshbuddy-proxy",
        to: "src-tauri/binaries/seshbuddy-proxy-aarch64-apple-darwin",
      },
    ]);
  });

  it("交叉目标：构建带 --target，产物路径含 triple 层", () => {
    const plan = planStaging({
      triple: WINDOWS,
      profile: "release",
      hostTriple: DARWIN_ARM,
      platform: "darwin",
    });
    expect(plan.builds).toEqual([{ triple: WINDOWS, useTargetFlag: true, profile: "release" }]);
    expect(plan.copies[0].from).toBe("target/x86_64-pc-windows-msvc/release/seshbuddy-proxy");
  });

  it("universal：两次构建 + 一次 lipo + 三份 sidecar", () => {
    const plan = planStaging({
      triple: "universal-apple-darwin",
      profile: "release",
      hostTriple: DARWIN_ARM,
      platform: "darwin",
    });
    expect(plan.builds).toEqual([
      { triple: DARWIN_ARM, useTargetFlag: false, profile: "release" },
      { triple: DARWIN_X64, useTargetFlag: true, profile: "release" },
    ]);
    expect(plan.lipos).toEqual([
      {
        inputs: [
          "target/release/seshbuddy-proxy",
          "target/x86_64-apple-darwin/release/seshbuddy-proxy",
        ],
        output: "src-tauri/binaries/seshbuddy-proxy-universal-apple-darwin",
      },
    ]);
    expect(plan.copies.map((copy) => copy.to)).toEqual([
      "src-tauri/binaries/seshbuddy-proxy-aarch64-apple-darwin",
      "src-tauri/binaries/seshbuddy-proxy-x86_64-apple-darwin",
    ]);
  });

  it("非 macOS 上的 universal 目标直接报错", () => {
    expect(() =>
      planStaging({
        triple: "universal-apple-darwin",
        profile: "release",
        hostTriple: WINDOWS,
        platform: "win32",
      })
    ).toThrow(/只能在 macOS 上构建/);
  });
});
