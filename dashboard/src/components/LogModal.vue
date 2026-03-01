<script setup lang="ts">
defineProps<{
  show: boolean;
  title: string;
  content: string;
}>();

const emit = defineEmits<{ close: [] }>();

function onOverlayClick(e: MouseEvent) {
  if ((e.target as HTMLElement).classList.contains("modal-overlay")) {
    emit("close");
  }
}
</script>

<template>
  <Teleport to="#modal-root">
    <div v-if="show" class="modal-overlay" @click="onOverlayClick">
      <div class="modal">
        <div class="modal-header">
          <h2 class="card-title">{{ title }}</h2>
          <button class="btn btn-ghost btn-sm btn-icon" @click="emit('close')">&#x2715;</button>
        </div>
        <div class="modal-body">
          <slot>
            <div class="log-output">{{ content || "No logs available." }}</div>
          </slot>
        </div>
      </div>
    </div>
  </Teleport>
</template>
