<script setup lang="ts">
import { computed } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { usePtySession } from "../composables/usePtySession";
import { t } from "../i18n";
import type { PtySessionInfo, PtyStatus } from "../types/pty";
import { basename } from "../utils/projectPath";
import { formatRelativeTime } from "../utils/format";

withDefaults(defineProps<{
  bootstrapping?: boolean;
  showNewSession?: boolean;
}>(), {
  bootstrapping: false,
  showNewSession: true,
});

const emit = defineEmits<{
  newSession: [];
  selectTerminal: [sessionId: string];
}>();

const { sessions, customLabels, getSessionLabel } = usePtySession();

const groupedSessions = computed(() => {
  const groups = new Map<string, PtySessionInfo[]>();
  for (const s of sessions.value) {
    const list = groups.get(s.projectPath) || [];
    list.push(s);
    groups.set(s.projectPath, list);
  }
  return groups;
});

const activeCount = computed(() => sessions.value.filter(s => s.status === "active").length);
const waitingCount = computed(() => sessions.value.filter(s => s.status === "waiting_input").length);
const idleCount = computed(() => sessions.value.filter(s => s.status === "idle").length);

function projectName(path: string): string {
  return basename(path) || path;
}

function statusColor(status: PtyStatus): string {
  switch (status) {
    case "active": return "var(--color-success)";
    case "idle": return "var(--color-warning)";
    case "waiting_input": return "var(--color-danger)";
    case "exited": return "var(--color-text-muted)";
  }
}

function statusLabel(status: PtyStatus): string {
  switch (status) {
    case "active": return t("session.agentPanel.statusActive");
    case "idle": return t("session.agentPanel.statusIdle");
    case "waiting_input": return t("session.agentPanel.statusWaitingInput");
    case "exited": return t("session.agentPanel.statusExited");
  }
}
</script>

<template>
  <div class="dashboard">
    <div class="dashboard-inner">
      <Transition name="banner-slide">
        <div v-if="bootstrapping" class="startup-banner">
          <span class="spinner" />
          <div class="banner-text">
            <strong>{{ t("session.dashboard.bootstrappingTitle") }}</strong>
            <span>{{ t("session.agentDashboard.bootstrappingHint") }}</span>
          </div>
        </div>
      </Transition>

      <template v-if="sessions.length > 0">
        <div class="stat-row">
          <div class="stat-pill stat-active">
            <span class="stat-dot" style="background: var(--color-success)" />
            {{ t("session.agentDashboard.activeCount", { count: activeCount }) }}
          </div>
          <div v-if="waitingCount > 0" class="stat-pill stat-waiting">
            <span class="stat-dot" style="background: var(--color-danger)" />
            {{ t("session.agentDashboard.waitingCount", { count: waitingCount }) }}
          </div>
          <div v-if="idleCount > 0" class="stat-pill">
            <span class="stat-dot" style="background: var(--color-warning)" />
            {{ t("session.agentDashboard.idleCount", { count: idleCount }) }}
          </div>
          <div class="stat-spacer" />
          <button v-if="showNewSession" class="new-btn" @click="emit('newSession')">
            <SvgIcon name="plus" :size="13" />
            {{ t("app.ctxMenu.newSession") }}
          </button>
        </div>

        <div v-for="[projectPath, projectSessions] in groupedSessions" :key="projectPath" class="project-group">
          <div class="project-label">
            <SvgIcon name="folder" :size="12" />
            {{ projectName(projectPath) }}
          </div>
          <div class="card-grid">
            <button
              v-for="session in projectSessions"
              :key="session.sessionId"
              class="session-card"
              :class="{ 'is-waiting': session.status === 'waiting_input' }"
              @click="emit('selectTerminal', session.sessionId)"
            >
              <div class="card-header">
                <span
                  class="status-dot"
                  :style="{ background: statusColor(session.status) }"
                  :class="{ pulse: session.status === 'waiting_input' || session.status === 'active' }"
                />
                <span class="card-cli">{{ customLabels[session.sessionId] ?? getSessionLabel(session.sessionId, session.projectPath) }}</span>
                <span class="card-time">{{ formatRelativeTime(session.createdAt) }}</span>
              </div>
              <div class="card-status">{{ statusLabel(session.status) }}</div>
            </button>
          </div>
        </div>
      </template>

      <div v-else class="empty-state">
        <div class="empty-icon">
          <SvgIcon name="terminal" :size="32" />
        </div>
        <p class="empty-title">{{ t("session.agentDashboard.emptyTitle") }}</p>
        <p class="empty-sub">{{ t("session.agentDashboard.emptySub") }}</p>
        <button v-if="showNewSession" class="new-btn new-btn-lg" @click="emit('newSession')">
          <SvgIcon name="plus" :size="14" />
          {{ t("app.ctxMenu.newSession") }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dashboard {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  justify-content: center;
  padding: var(--space-8) var(--space-6);
}
.dashboard-inner {
  width: 100%;
  max-width: 680px;
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
}

.banner-slide-enter-active,
.banner-slide-leave-active {
  transition: opacity 180ms ease, transform 200ms ease, max-height 200ms ease;
  overflow: hidden;
}
.banner-slide-enter-from,
.banner-slide-leave-to { opacity: 0; transform: translateY(-6px); max-height: 0; }
.banner-slide-enter-to,
.banner-slide-leave-from { opacity: 1; transform: translateY(0); max-height: 80px; }
.startup-banner {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-4);
  border: 1px solid color-mix(in srgb, var(--color-primary) 18%, transparent);
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--color-primary) 6%, transparent);
}
.spinner {
  width: 15px;
  height: 15px;
  border-radius: var(--radius-full);
  border: 2px solid var(--color-border);
  border-top-color: var(--color-primary);
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}
.banner-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
}
.banner-text strong { color: var(--color-text); font-size: var(--text-sm); }

.stat-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: wrap;
}
.stat-pill {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  padding: 3px var(--space-2);
  border-radius: var(--radius-full);
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  background: var(--color-bg-hover);
  font-weight: 500;
}
.stat-waiting {
  color: var(--color-danger);
  background: rgba(220, 38, 38, 0.08);
}
.stat-dot {
  width: 6px;
  height: 6px;
  border-radius: 99px;
  flex-shrink: 0;
}
.stat-spacer { flex: 1; }

.new-btn {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--color-primary);
  background: var(--color-primary-light);
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.new-btn:hover {
  background: var(--color-primary);
  color: white;
}
.new-btn-lg {
  padding: var(--space-2) var(--space-5);
  font-size: var(--text-sm);
}

.project-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.project-label {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  font-size: var(--text-xs);
  font-weight: 600;
  color: var(--color-text-muted);
  text-transform: uppercase;
  letter-spacing: 0.4px;
}

.card-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
  gap: var(--space-2);
}
.session-card {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  padding: var(--space-3) var(--space-3);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  cursor: pointer;
  text-align: left;
  background: var(--color-bg);
  transition: all var(--transition-fast);
}
.session-card:hover {
  border-color: var(--color-primary);
  background: var(--color-primary-light);
  transform: translateY(-1px);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.06);
}
.session-card.is-waiting {
  border-color: var(--color-danger);
  animation: card-pulse 2s ease-in-out infinite;
}
@keyframes card-pulse {
  0%, 100% { box-shadow: 0 0 0 0 rgba(220, 38, 38, 0.15); }
  50% { box-shadow: 0 0 0 4px rgba(220, 38, 38, 0.08); }
}
.card-header {
  display: flex;
  align-items: center;
  gap: var(--space-1);
}
.status-dot {
  width: 7px;
  height: 7px;
  border-radius: 99px;
  flex-shrink: 0;
}
.status-dot.pulse {
  animation: dot-pulse 2s ease-in-out infinite;
}
@keyframes dot-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}
.card-cli {
  flex: 1;
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--color-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.card-time {
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
  flex-shrink: 0;
}
.card-status {
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-16) 0;
  text-align: center;
}
.empty-icon {
  width: 56px;
  height: 56px;
  border-radius: var(--radius-xl);
  background: var(--color-bg-hover);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted);
}
.empty-title {
  font-size: var(--text-base);
  font-weight: 600;
  color: var(--color-text);
}
.empty-sub {
  font-size: var(--text-sm);
  color: var(--color-text-muted);
}
</style>
