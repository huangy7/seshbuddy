import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  useActivityBar,
  resetActivityBarStateForTest,
  DEFAULT_EXPANDED_WIDTH,
  COLLAPSED_WIDTH,
  MAX_EXPANDED_WIDTH,
} from "./useActivityBar";

describe("useActivityBar", () => {
  let store: Record<string, string> = {};

  beforeEach(() => {
    store = {};
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => store[key] ?? null,
      setItem: (key: string, val: string) => { store[key] = val; },
      removeItem: (key: string) => { delete store[key]; },
      clear: () => { store = {}; },
    });
    resetActivityBarStateForTest();
  });

  it("默认初始化为收起态 (48px) 与默认展开宽度 (160px)", () => {
    const { isExpanded, railWidth } = useActivityBar();
    expect(isExpanded.value).toBe(false);
    expect(railWidth.value).toBe(COLLAPSED_WIDTH);
  });

  it("支持从 localStorage 恢复已展开状态和自定义宽度", () => {
    store["seshbuddy-activity-bar-expanded"] = "true";
    store["seshbuddy-activity-bar-width"] = "190";

    const { isExpanded, railWidth } = useActivityBar();
    expect(isExpanded.value).toBe(true);
    expect(railWidth.value).toBe(190);
  });

  it("toggleExpanded 在收起态和展开态之间切换并持久化", () => {
    const { isExpanded, railWidth, toggleExpanded } = useActivityBar();
    
    toggleExpanded();
    expect(isExpanded.value).toBe(true);
    expect(railWidth.value).toBe(DEFAULT_EXPANDED_WIDTH);
    expect(store["seshbuddy-activity-bar-expanded"]).toBe("true");

    toggleExpanded();
    expect(isExpanded.value).toBe(false);
    expect(railWidth.value).toBe(COLLAPSED_WIDTH);
    expect(store["seshbuddy-activity-bar-expanded"]).toBe("false");
  });

  it("setRailWidth 限制在合法范围 (48px ~ 260px)，小于 100px 自动吸附收起", () => {
    const { isExpanded, railWidth, setRailWidth } = useActivityBar();
    
    // 拉伸到 180px
    setRailWidth(180);
    expect(isExpanded.value).toBe(true);
    expect(railWidth.value).toBe(180);

    // 拉伸小于 100px 自动收缩为 48px
    setRailWidth(90);
    expect(isExpanded.value).toBe(false);
    expect(railWidth.value).toBe(COLLAPSED_WIDTH);

    // 拉伸超过最大值 260px 时 clamp 到 260px
    setRailWidth(300);
    expect(railWidth.value).toBe(MAX_EXPANDED_WIDTH);
  });
});
