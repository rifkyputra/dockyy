<script setup lang="ts">
import { ref, onMounted, computed } from "vue";
import { api } from "../api";

interface ServerInfo {
  version: string;
  docker: string;
  status: string;
  hostname: string;
  os: string;
  arch: string;
  cpu_cores: number;
  uptime_secs: number;
}

const info = ref<ServerInfo | null>(null);
const loading = ref(true);

const uptime = computed(() => {
  if (!info.value) return "";
  const s = info.value.uptime_secs;
  const days = Math.floor(s / 86400);
  const hours = Math.floor((s % 86400) / 3600);
  const mins = Math.floor((s % 3600) / 60);
  const parts: string[] = [];
  if (days > 0) parts.push(`${days}d`);
  if (hours > 0) parts.push(`${hours}h`);
  parts.push(`${mins}m`);
  return parts.join(" ");
});

async function load() {
  loading.value = true;
  try {
    info.value = await api.health() as ServerInfo;
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
          <span style="color: var(--text-muted); font-size: 13px">Hostname</span><br />
          <strong>{{ info.hostname }}</strong>
        </div>
        <div>
          <span style="color: var(--text-muted); font-size: 13px">OS</span><br />
          <strong>{{ info.os }} ({{ info.arch }})</strong>
        </div>
        <div>
          <span style="color: var(--text-muted); font-size: 13px">CPU Cores</span><br />
          <strong>{{ info.cpu_cores }}</strong>
        </div>
        <div>
          <span style="color: var(--text-muted); font-size: 13px">Uptime</span><br />
          <strong>{{ uptime }}</strong>
        </div>
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
