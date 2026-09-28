import { describe, it, expect, beforeEach, vi } from "vitest";
import { nextTick } from "vue";
import {
  useRightSidebar,
  STORAGE_KEY_RIGHT_SIDEBAR_OPEN,
  STORAGE_KEY_RIGHT_SIDEBAR_MODE,
  STORAGE_KEY_RIGHT_SIDEBAR_WIDTH,
  DEFAULT_RIGHT_SIDEBAR_WIDTH,
  MIN_RIGHT_SIDEBAR_WIDTH,
  MAX_RIGHT_SIDEBAR_WIDTH,
} from "./useRightSidebar";

describe("useRightSidebar", () => {
  let store: Record<string, string> = {};

  beforeEach(() => {
    store = {};
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => store[key] ?? null,
      setItem: (key: string, val: string) => {
        store[key] = String(val);
      },
      removeItem: (key: string) => {
        delete store[key];
      },
      clear: () => {
        store = {};
      },
    });
  });

  it("默认状态：收起(open=false)，项目模式(mode=project)，宽度 280px", () => {
    const { rightSidebarOpen, rightSidebarMode, rightSidebarWidth } = useRightSidebar();
    expect(rightSidebarOpen.value).toBe(false);
    expect(rightSidebarMode.value).toBe("project");
    expect(rightSidebarWidth.value).toBe(DEFAULT_RIGHT_SIDEBAR_WIDTH);
  });

  it("从 localStorage 恢复已保存的状态", () => {
    store[STORAGE_KEY_RIGHT_SIDEBAR_OPEN] = "1";
    store[STORAGE_KEY_RIGHT_SIDEBAR_MODE] = "git";
    store[STORAGE_KEY_RIGHT_SIDEBAR_WIDTH] = "350";

    const { rightSidebarOpen, rightSidebarMode, rightSidebarWidth } = useRightSidebar();
    expect(rightSidebarOpen.value).toBe(true);
    expect(rightSidebarMode.value).toBe("git");
    expect(rightSidebarWidth.value).toBe(350);
  });

  it("从 localStorage 恢复非法或超界宽度时自动做 clamp", () => {
    store[STORAGE_KEY_RIGHT_SIDEBAR_WIDTH] = "50"; // below min
    const sidebarSmall = useRightSidebar();
    expect(sidebarSmall.rightSidebarWidth.value).toBe(MIN_RIGHT_SIDEBAR_WIDTH);

    store[STORAGE_KEY_RIGHT_SIDEBAR_WIDTH] = "1000"; // above max
    const sidebarLarge = useRightSidebar();
    expect(sidebarLarge.rightSidebarWidth.value).toBe(MAX_RIGHT_SIDEBAR_WIDTH);

    store[STORAGE_KEY_RIGHT_SIDEBAR_WIDTH] = "invalid";
    const sidebarInvalid = useRightSidebar();
    expect(sidebarInvalid.rightSidebarWidth.value).toBe(DEFAULT_RIGHT_SIDEBAR_WIDTH);
  });

  it("从 localStorage 恢复非法模式时回退到 project", () => {
    store[STORAGE_KEY_RIGHT_SIDEBAR_MODE] = "unknown-mode";
    const { rightSidebarMode } = useRightSidebar();
    expect(rightSidebarMode.value).toBe("project");
  });

  it("toggleRightSidebar 无参数时切换开闭状态，并持久化到 localStorage", async () => {
    const { rightSidebarOpen, toggleRightSidebar } = useRightSidebar();
    expect(rightSidebarOpen.value).toBe(false);

    toggleRightSidebar();
    expect(rightSidebarOpen.value).toBe(true);
    await nextTick();
    expect(store[STORAGE_KEY_RIGHT_SIDEBAR_OPEN]).toBe("1");

    toggleRightSidebar();
    expect(rightSidebarOpen.value).toBe(false);
    await nextTick();
    expect(store[STORAGE_KEY_RIGHT_SIDEBAR_OPEN]).toBe("0");
  });

  it("toggleRightSidebar 带模式参数时的状态流转", async () => {
    const { rightSidebarOpen, rightSidebarMode, toggleRightSidebar } = useRightSidebar();

    // 初始关闭：传入 "timeline" 应打开并切换到 timeline 模式
    toggleRightSidebar("timeline");
    expect(rightSidebarOpen.value).toBe(true);
    expect(rightSidebarMode.value).toBe("timeline");
    await nextTick();
    expect(store[STORAGE_KEY_RIGHT_SIDEBAR_OPEN]).toBe("1");
    expect(store[STORAGE_KEY_RIGHT_SIDEBAR_MODE]).toBe("timeline");

    // 已展开且模式相同：再次传入 "timeline" 应收起
    toggleRightSidebar("timeline");
    expect(rightSidebarOpen.value).toBe(false);
    await nextTick();
    expect(store[STORAGE_KEY_RIGHT_SIDEBAR_OPEN]).toBe("0");

    // 重新打开到 git 模式
    toggleRightSidebar("git");
    expect(rightSidebarOpen.value).toBe(true);
    expect(rightSidebarMode.value).toBe("git");

    // 已展开但模式不同：传入 "timeline" 应切换模式但不收起
    toggleRightSidebar("timeline");
    expect(rightSidebarOpen.value).toBe(true);
    expect(rightSidebarMode.value).toBe("timeline");
  });

  it("openRightSidebar / closeRightSidebar 正确控制开合", async () => {
    const { rightSidebarOpen, rightSidebarMode, openRightSidebar, closeRightSidebar } = useRightSidebar();

    openRightSidebar("git");
    expect(rightSidebarOpen.value).toBe(true);
    expect(rightSidebarMode.value).toBe("git");

    closeRightSidebar();
    expect(rightSidebarOpen.value).toBe(false);

    openRightSidebar();
    expect(rightSidebarOpen.value).toBe(true);
    expect(rightSidebarMode.value).toBe("git"); // 保持之前模式
  });

  it("setRightSidebarMode 切换模式并持久化", async () => {
    const { rightSidebarMode, setRightSidebarMode } = useRightSidebar();

    setRightSidebarMode("timeline");
    expect(rightSidebarMode.value).toBe("timeline");
    await nextTick();
    expect(store[STORAGE_KEY_RIGHT_SIDEBAR_MODE]).toBe("timeline");

    setRightSidebarMode("git");
    expect(rightSidebarMode.value).toBe("git");
    await nextTick();
    expect(store[STORAGE_KEY_RIGHT_SIDEBAR_MODE]).toBe("git");
  });

  it("startRightResize 拖拽调整宽度并持久化", async () => {
    const { rightSidebarWidth, isRightResizing, startRightResize } = useRightSidebar();
    expect(rightSidebarWidth.value).toBe(280);

    // 模拟 mousedown (startX = 500)
    startRightResize(new MouseEvent("mousedown", { clientX: 500 }));
    expect(isRightResizing.value).toBe(true);

    // 向左拖拽 100px (clientX = 400)，宽度应增加 100px -> 380px
    document.dispatchEvent(new MouseEvent("mousemove", { clientX: 400 }));
    expect(rightSidebarWidth.value).toBe(380);

    // 拖拽超出最大限制 (clientX = 0 -> 280 + 500 = 780 -> clamped to 600)
    document.dispatchEvent(new MouseEvent("mousemove", { clientX: 0 }));
    expect(rightSidebarWidth.value).toBe(MAX_RIGHT_SIDEBAR_WIDTH);

    // 拖拽超出最小限制 (clientX = 800 -> 280 - 300 = -20 -> clamped to 200)
    document.dispatchEvent(new MouseEvent("mousemove", { clientX: 800 }));
    expect(rightSidebarWidth.value).toBe(MIN_RIGHT_SIDEBAR_WIDTH);

    // 模拟 mouseup 结束拖拽
    document.dispatchEvent(new MouseEvent("mouseup"));
    expect(isRightResizing.value).toBe(false);
    await nextTick();
    expect(store[STORAGE_KEY_RIGHT_SIDEBAR_WIDTH]).toBe(String(MIN_RIGHT_SIDEBAR_WIDTH));
  });
});
