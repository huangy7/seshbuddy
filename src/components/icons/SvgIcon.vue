<script setup lang="ts">
import { computed } from "vue";
import { iconPaths } from "./iconPaths";

const props = withDefaults(
  defineProps<{
    name: string;
    size?: number;
  }>(),
  { size: 16 }
);

const pathData = computed(() => iconPaths[props.name] ?? "");
</script>

<template>
  <svg
    class="svg-icon"
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
    v-html="pathData"
  />
</template>

<style scoped>
.svg-icon {
  display: inline-block;
  vertical-align: middle;
  flex-shrink: 0;
}
</style>
