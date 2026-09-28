// 「重建全部索引」的共享队列状态。
// IndexSettings 的全局卡与 CLI 卡是两个独立组件实例，
// 重建进度、当前 CLI、等待队列都收在这里，两张卡各自订阅。
import { computed, reactive } from "vue";
import { invokeApp } from "../utils/invokeApp";
import { useSessions } from "./useSessions";
import { indexProgressPercent } from "../utils/indexProgress";
import type { CliId } from "../types/cli";

export interface RebuildQueueItem {
  id: CliId;
  name: string;
}

const state = reactive({
  running: false,
  queue: [] as RebuildQueueItem[],
  currentIndex: 0,
  // 高水位：done 帧会把 searchIndexProgress 置 null，
  // 避免加权百分比在 CLI 切换间隙回落。
  highWaterMark: 0,
});

export function useIndexRebuildQueue() {
  const { searchIndexProgress } = useSessions();

  const running = computed(() => state.running);
  const total = computed(() => state.queue.length);
  const currentIndex = computed(() => state.currentIndex);
  const currentCliId = computed<CliId | undefined>(() =>
    state.running ? state.queue[state.currentIndex]?.id : undefined,
  );
  const currentCliName = computed(() =>
    state.running ? (state.queue[state.currentIndex]?.name ?? "") : "",
  );

  const overallPercent = computed(() => {
    if (!state.running || state.queue.length === 0) return 0;
    const progress = searchIndexProgress.value;
    const currentRatio =
      progress && progress.cliId === currentCliId.value
        ? indexProgressPercent(progress) / 100
        : 0;
    const raw = Math.round(
      ((state.currentIndex + currentRatio) / state.queue.length) * 100,
    );
    state.highWaterMark = Math.max(state.highWaterMark, raw);
    return state.highWaterMark;
  });

  function isQueued(cliId: CliId): boolean {
    if (!state.running) return false;
    const idx = state.queue.findIndex((item) => item.id === cliId);
    return idx > state.currentIndex;
  }

  async function startRebuildAll(clis: RebuildQueueItem[]): Promise<void> {
    if (state.running || clis.length === 0) return;
    state.running = true;
    state.queue = clis;
    state.currentIndex = 0;
    state.highWaterMark = 0;
    try {
      for (let i = 0; i < state.queue.length; i++) {
        state.currentIndex = i;
        const cliId = state.queue[i].id;
        await invokeApp("clear_session_index", { cliId });
        await invokeApp("refresh_session_list_index", {
          cliId,
          notify: false,
          force: true,
        });
        await invokeApp("ensure_search_index_ready", { cliId });
      }
    } finally {
      state.running = false;
      state.queue = [];
      state.currentIndex = 0;
      state.highWaterMark = 0;
    }
  }

  return {
    running,
    total,
    currentIndex,
    currentCliId,
    currentCliName,
    overallPercent,
    isQueued,
    startRebuildAll,
  };
}
