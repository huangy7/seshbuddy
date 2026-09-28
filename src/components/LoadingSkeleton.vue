<template>
  <div class="loading-skeleton">
    <div v-for="i in 3" :key="i" class="skeleton-message">
      <div class="skeleton-header">
        <div class="skeleton-bar" style="width: 60px;"></div>
        <div class="skeleton-bar" style="width: 100px;"></div>
      </div>
      <div class="skeleton-body">
        <div class="skeleton-bar" style="width: 100%;"></div>
        <div class="skeleton-bar" style="width: 85%;"></div>
        <div class="skeleton-bar" style="width: 70%;"></div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.loading-skeleton {
  padding: var(--space-4);
}
.skeleton-message {
  margin-bottom: var(--space-6);
}
.skeleton-header {
  display: flex;
  gap: var(--space-2);
  margin-bottom: var(--space-2);
}
.skeleton-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.skeleton-bar {
  height: 14px;
  border-radius: var(--radius-sm);
  background: linear-gradient(
    90deg,
    var(--color-bg-hover) 25%,
    var(--color-border-light) 50%,
    var(--color-bg-hover) 75%
  );
  background-size: 200% 100%;
  animation: shimmer 1.5s ease-in-out infinite;
}
@keyframes shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}
</style>
