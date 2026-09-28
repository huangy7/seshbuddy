import { ref, computed } from "vue";
import { invokeApp, renderAppError } from "../utils/invokeApp";
import { t } from "../i18n";
import type { UsageRecord } from "../types/session";
import type { CliId } from "../types/cli";

const recordsByCli = ref<Map<CliId, UsageRecord[]>>(new Map());
const loading = ref(false);
const loadError = ref("");
const lastFetchedAt = ref<number | null>(null);

export function resetUsageStatsStore() {
  recordsByCli.value = new Map();
  loading.value = false;
  loadError.value = "";
  lastFetchedAt.value = null;
}

export function useUsageStats() {
  const allRecords = computed(() => {
    const list: UsageRecord[] = [];
    for (const arr of recordsByCli.value.values()) {
      list.push(...arr);
    }
    return list;
  });

  async function fetchUsage(cliIds: CliId[], force = false) {
    if (cliIds.length === 0) {
      recordsByCli.value = new Map();
      return;
    }
    if (lastFetchedAt.value === null || force) {
      loading.value = true;
    }
    loadError.value = "";

    try {
      const results = await Promise.allSettled(
        cliIds.map(async (cliId) => {
          const res = await invokeApp<UsageRecord[]>("get_usage_stats", { cliId });
          return { cliId, records: res };
        }),
      );

      const nextMap = new Map(recordsByCli.value);
      let successCount = 0;
      for (const res of results) {
        if (res.status === "fulfilled") {
          nextMap.set(res.value.cliId, res.value.records);
          successCount++;
        }
      }
      recordsByCli.value = nextMap;
      lastFetchedAt.value = Date.now();
      if (successCount === 0 && results.length > 0) {
        loadError.value = t("session.usageDashboard.loadError");
      }
    } catch (err) {
      loadError.value = renderAppError(err);
    } finally {
      loading.value = false;
    }
  }

  return {
    recordsByCli,
    allRecords,
    loading: computed(() => loading.value),
    loadError: computed(() => loadError.value),
    lastFetchedAt: computed(() => lastFetchedAt.value),
    fetchUsage,
  };
}
