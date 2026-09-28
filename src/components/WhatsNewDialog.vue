<script setup lang="ts">
import type { ChangelogEntry } from "../changelog";
import { t } from "../i18n";

defineProps<{
  entry: ChangelogEntry;
}>();

const emit = defineEmits<{
  close: [];
}>();
</script>

<template>
  <Transition name="fade">
    <div class="modal-backdrop" @click.self="emit('close')">
      <div class="whatsnew-window">
        <div class="whatsnew-header">
          <div class="whatsnew-badge">{{ t("dialogs.whatsNew.badge") }}</div>
          <h2 class="whatsnew-title">SeshBuddy v{{ entry.version }}</h2>
          <span class="whatsnew-date">{{ entry.date }}</span>
        </div>
        <div class="whatsnew-body">
          <ul class="whatsnew-list">
            <li v-for="(changeKey, i) in entry.changeKeys" :key="i">{{ t(changeKey) }}</li>
          </ul>
        </div>
        <div class="whatsnew-footer">
          <button class="whatsnew-btn" @click="emit('close')">{{ t("dialogs.whatsNew.confirm") }}</button>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.whatsnew-window {
  width: 420px;
  max-width: 90vw;
  background: var(--color-bg);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
}
.whatsnew-header {
  padding: var(--space-5) var(--space-5) var(--space-3);
  text-align: center;
}
.whatsnew-badge {
  display: inline-block;
  padding: 2px 10px;
  font-size: var(--text-2xs);
  font-weight: 700;
  letter-spacing: 1px;
  color: white;
  background: var(--color-primary);
  border-radius: var(--radius-full);
  margin-bottom: var(--space-2);
}
.whatsnew-title {
  font-size: var(--text-lg);
  font-weight: 700;
  margin: 0 0 var(--space-1);
  color: var(--color-text);
}
.whatsnew-date {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}
.whatsnew-body {
  padding: 0 var(--space-5) var(--space-3);
  max-height: 300px;
  overflow-y: auto;
}
.whatsnew-list {
  margin: 0;
  padding: 0;
  list-style: none;
}
.whatsnew-list li {
  position: relative;
  padding: var(--space-1) 0 var(--space-1) var(--space-4);
  font-size: var(--text-sm);
  color: var(--color-text-secondary);
  line-height: var(--leading-relaxed);
}
.whatsnew-list li::before {
  content: "•";
  position: absolute;
  left: var(--space-1);
  color: var(--color-primary);
  font-weight: 700;
}
.whatsnew-footer {
  padding: var(--space-3) var(--space-5) var(--space-5);
  text-align: center;
}
.whatsnew-btn {
  padding: var(--space-2) var(--space-6);
  font-size: var(--text-sm);
  font-weight: 600;
  color: white;
  background: var(--color-primary);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.whatsnew-btn:hover {
  opacity: 0.9;
  transform: translateY(-1px);
}
</style>
