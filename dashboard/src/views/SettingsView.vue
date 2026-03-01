<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api } from "../api";

const info = ref<{ version: string; docker: string; status: string } | null>(null);
const loading = ref(true);

async function load() {
  loading.value = true;
  try {
    info.value = await api.health();
  } catch {
    // ignore
  }
  loading.value = false;
}

onMounted(load);
</script>

<template>
  <div class="card">
    <div class="card-header"><h2 class="card-title">Server Info</h2></div>
    <div class="card-body">
      <div v-if="loading" class="spinner"></div>
      <div v-else-if="info" style="display: grid; gap: 12px; max-width: 400px">
        <div>
          <span style="color: var(--text-muted); font-size: 13px">Version</span><br />
          <strong>{{ info.version }}</strong>
        </div>
        <div>
          <span style="color: var(--text-muted); font-size: 13px">Docker</span><br />
          <span class="badge" :class="info.docker === 'connected' ? 'badge-success' : 'badge-danger'">{{ info.docker }}</span>
        </div>
        <div>
          <span style="color: var(--text-muted); font-size: 13px">Status</span><br />
          <span class="badge badge-success">{{ info.status }}</span>
        </div>
      </div>
    </div>
  </div>
</template>
