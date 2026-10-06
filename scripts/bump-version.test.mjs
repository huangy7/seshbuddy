import { describe, expect, it } from "vitest";
import { updateCargoTomlContent, validateVersion } from "./bump-version.mjs";

describe("bump-version 脚本测试", () => {
  describe("validateVersion", () => {
    it("接受标准三段式 SemVer 版本", () => {
      expect(validateVersion("0.1.1")).toBe("0.1.1");
      expect(validateVersion("1.0.0")).toBe("1.0.0");
    });

    it("自动剔除 v 前缀", () => {
      expect(validateVersion("v0.1.1")).toBe("0.1.1");
      expect(validateVersion("v2.3.4")).toBe("2.3.4");
    });

    it("支持预发布版本后缀", () => {
      expect(validateVersion("0.1.1-beta.1")).toBe("0.1.1-beta.1");
    });

    it("非法格式抛出异常", () => {
      expect(() => validateVersion("")).toThrow("版本号不能为空");
      expect(() => validateVersion("invalid")).toThrow("非法版本号格式");
      expect(() => validateVersion("0.1")).toThrow("非法版本号格式");
      expect(() => validateVersion("v")).toThrow("非法版本号格式");
    });
  });

  describe("updateCargoTomlContent", () => {
    it("正确替换 [workspace.package] 下的 version", () => {
      const sample = `[workspace]
members = ["src-tauri", "src-tauri-proxy", "transcript-store"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
authors = ["huangy"]

[profile.release]
opt-level = "z"
`;

      const updated = updateCargoTomlContent(sample, "0.1.2");
      expect(updated).toContain('version = "0.1.2"');
      expect(updated).toContain('[workspace.package]');
      expect(updated).not.toContain('version = "0.1.0"');
    });

    it("缺失 [workspace.package] version 时抛出明确异常", () => {
      const invalid = `[workspace]
members = ["src-tauri"]
`;
      expect(() => updateCargoTomlContent(invalid, "0.1.2")).toThrow(
        "未找到 [workspace.package] 的 version 字段定义"
      );
    });
  });
});
