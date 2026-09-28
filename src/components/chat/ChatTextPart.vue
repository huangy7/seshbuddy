<script setup lang="ts">
import { vMermaid } from "../../directives/vMermaid";

defineProps<{
  html: string;
}>();

defineEmits<{
  markdownClick: [event: MouseEvent];
}>();
</script>

<template>
  <div
    v-mermaid
    class="text-content markdown-body"
    v-html="html"
    @click="$emit('markdownClick', $event)"
  ></div>
</template>

<style scoped>
.text-content {
  word-break: break-word;
}
</style>
