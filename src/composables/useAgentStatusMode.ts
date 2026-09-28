import { ref, watch } from "vue";
import type { AgentStatusMode } from "../types/pty";

const STORAGE_KEY = "seshbuddy:agent-status-mode:v1";

function loadMode(): AgentStatusMode {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === "osc" || raw === "hook-relay") {
      return raw;
    }
  } catch {
    // ignore
  }
  return "osc";
}

const agentStatusMode = ref<AgentStatusMode>(loadMode());

watch(agentStatusMode, (value) => {
  try {
    localStorage.setItem(STORAGE_KEY, value);
  } catch {
    // ignore
  }
});

export function useAgentStatusMode() {
  return { agentStatusMode };
}
