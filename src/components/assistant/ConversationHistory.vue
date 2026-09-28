<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { invokeApp } from "../../utils/invokeApp";
import { ask } from "@tauri-apps/plugin-dialog";
import { t } from "../../i18n";

interface Conversation {
  id: string;
  title: string;
  profile: string;
  updatedAt: string;
}

const props = defineProps<{ currentId: string | null }>();
const emit = defineEmits<{
  select: [id: string, profile: string];
  close: [];
  delete: [id: string];
}>();
const conversations = ref<Conversation[]>([]);

interface Group {
  label: string;
  items: Conversation[];
}

/** 按更新时间分组：今天 / 昨天 / 过去 7 天 / 更早（本地时区） */
const groups = computed<Group[]>(() => {
  const now = new Date();
  const todayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const dayMs = 86400000;
  const buckets: Group[] = [
    { label: t("assistant.history.today"), items: [] },
    { label: t("assistant.history.yesterday"), items: [] },
    { label: t("assistant.history.last7Days"), items: [] },
    { label: t("assistant.history.earlier"), items: [] },
  ];
  for (const c of conversations.value) {
    const t = new Date(c.updatedAt).getTime();
    if (t >= todayStart) buckets[0].items.push(c);
    else if (t >= todayStart - dayMs) buckets[1].items.push(c);
    else if (t >= todayStart - 7 * dayMs) buckets[2].items.push(c);
    else buckets[3].items.push(c);
  }
  return buckets.filter((b) => b.items.length > 0);
});

onMounted(async () => {
  try {
    conversations.value = await invokeApp<Conversation[]>("assistant_list_conversations");
  } catch (err) {
    console.error("加载历史对话失败:", err);
  }
});

async function remove(e: MouseEvent, c: Conversation) {
  e.stopPropagation();
  // 用 Tauri 异步对话框：WebView 的 window.confirm 不阻塞
  const ok = await ask(
    t("assistant.history.deleteConfirm", { title: c.title || t("assistant.history.untitled") }),
    {
      title: t("assistant.history.deleteTitle"),
      kind: "warning",
      okLabel: t("app.batch.delete"),
      cancelLabel: t("dialogs.common.cancel"),
    },
  );
  if (!ok) return;
  try {
    await invokeApp("assistant_delete_conversation", { conversationId: c.id });
    conversations.value = conversations.value.filter((x) => x.id !== c.id);
    emit("delete", c.id);
  } catch (err) {
    console.error("删除对话失败:", err);
  }
}

function relativeTime(iso: string): string {
  const t = new Date(iso);
  return `${String(t.getHours()).padStart(2, "0")}:${String(t.getMinutes()).padStart(2, "0")}`;
}
</script>

<template>
  <div class="history-backdrop" @click="emit('close')" />
  <aside class="history-drawer">
    <header class="history-header">
      <span>{{ t("assistant.history.title") }}</span>
      <button class="icon-btn" :title="t('dialogs.common.close')" @click="emit('close')">×</button>
    </header>
    <div v-if="conversations.length === 0" class="history-empty">{{ t("assistant.history.empty") }}</div>
    <div v-else class="history-scroll">
      <div v-for="g in groups" :key="g.label" class="history-group">
        <div class="group-label">{{ g.label }}</div>
        <button
          v-for="c in g.items"
          :key="c.id"
          class="conv-item"
          :class="{ active: c.id === props.currentId }"
          @click="emit('select', c.id, c.profile)"
        >
          <span class="conv-title">{{ c.title || t("assistant.history.untitled") }}</span>
          <span class="conv-time">{{ relativeTime(c.updatedAt) }}</span>
          <span
            class="conv-delete"
            :title="t('assistant.history.deleteConversation')"
            @click="remove($event, c)"
          >×</span>
        </button>
      </div>
    </div>
  </aside>
</template>

<style scoped>
.history-backdrop {
  position: absolute;
  inset: 0;
  z-index: 9;
  background: rgba(0, 0, 0, 0.18);
  animation: backdrop-in 0.15s ease-out;
}
@keyframes backdrop-in {
  from { opacity: 0; }
  to { opacity: 1; }
}
.history-drawer {
  position: absolute;
  top: 0;
  left: 0;
  bottom: 0;
  width: 240px;
  z-index: 10;
  display: flex;
  flex-direction: column;
  background: var(--color-bg);
  border-right: 1px solid var(--color-border);
  box-shadow: var(--shadow-lg);
}
.history-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-3);
  border-bottom: 1px solid var(--color-border);
  font-size: var(--text-sm);
  font-weight: 600;
}
.history-empty {
  padding: var(--space-3);
  color: var(--color-text-muted);
  font-size: var(--text-sm);
}
.history-scroll {
  overflow-y: auto;
  padding: var(--space-1);
}
.group-label {
  padding: var(--space-2) var(--space-2) var(--space-1);
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
  letter-spacing: 0.05em;
}
.conv-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  width: 100%;
  padding: 7px 8px;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--color-text);
  cursor: pointer;
  text-align: left;
}
.conv-item:hover {
  background: var(--color-bg-hover);
}
.conv-item.active {
  background: var(--color-primary-light);
}
.conv-title {
  flex: 1;
  font-size: var(--text-sm);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.conv-time {
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
  font-family: var(--font-mono);
  flex-shrink: 0;
}
.conv-delete {
  position: absolute;
  right: 6px;
  top: 50%;
  transform: translateY(-50%);
  display: none;
  width: 20px;
  height: 20px;
  line-height: 20px;
  text-align: center;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  font-size: 14px;
}
.conv-item:hover .conv-delete {
  display: block;
}
.conv-delete:hover {
  background: var(--color-danger);
  color: var(--color-text-inverse);
}
.conv-item:hover .conv-time {
  visibility: hidden;
}
</style>
