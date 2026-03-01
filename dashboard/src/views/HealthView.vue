<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api, type ServerMetrics } from "../api";

const metrics = ref<ServerMetrics | null>(null);
const loading = ref(true);
const error = ref("");

function fmtBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`;
}

function pct(used: number, total: number): number {
  return total > 0 ? Math.round((used / total) * 100) : 0;
}

function meterColor(p: number): string {
  if (p > 95) return "#ef4444";
  if (p > 85) return "#f59e0b";
  return "var(--primary)";
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    metrics.value = await api.metrics();
  } catch (err: unknown) {
    error.value = err instanceof Error ? err.message : "Unknown error";
  }
  loading.value = false;
}

onMounted(load);
</script>

<template>
  <div v-if="loading" class="spinner"></div>
  <div v-else-if="error" class="empty-state">
    <div class="empty-icon">&#x26A0;&#xFE0F;</div>
    <h3>Could not load metrics</h3>
    <p>{{ error }}</p>
  </div>
  <div v-else-if="metrics">
    <div class="stats-grid">
      <div class="stat-card">
        <div class="stat-icon" :class="Math.round(metrics.cpu_usage_pct) > 85 ? 'yellow' : 'blue'">&#x1F5A5;</div>
        <div class="stat-value">{{ Math.round(metrics.cpu_usage_pct) }}%</div>
        <div class="stat-label">CPU Usage</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon" :class="pct(metrics.mem_used_bytes, metrics.mem_total_bytes) > 85 ? 'yellow' : 'green'">&#x1F9E0;</div>
        <div class="stat-value">{{ fmtBytes(metrics.mem_used_bytes) }}</div>
        <div class="stat-label">RAM Used / {{ fmtBytes(metrics.mem_total_bytes) }}</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon" :class="pct(metrics.swap_used_bytes, metrics.swap_total_bytes) > 85 ? 'yellow' : 'blue'">&#x1F504;</div>
        <div class="stat-value">{{ metrics.swap_total_bytes > 0 ? fmtBytes(metrics.swap_used_bytes) : "N/A" }}</div>
        <div class="stat-label">Swap Used{{ metrics.swap_total_bytes > 0 ? ` / ${fmtBytes(metrics.swap_total_bytes)}` : "" }}</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon" :class="pct(metrics.disk_used_bytes, metrics.disk_total_bytes) > 85 ? 'yellow' : 'purple'">&#x1F4BE;</div>
        <div class="stat-value">{{ fmtBytes(metrics.disk_used_bytes) }}</div>
        <div class="stat-label">Disk Used / {{ fmtBytes(metrics.disk_total_bytes) }}</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon" :class="metrics.docker_ok ? 'green' : 'yellow'">&#x1F40B;</div>
        <div class="stat-value" style="font-size: 1.1rem">{{ metrics.docker_ok ? "Connected" : "Disconnected" }}</div>
        <div class="stat-label">Docker</div>
      </div>
    </div>

    <div class="card">
      <div class="card-header"><h2 class="card-title">Resource Usage</h2></div>
      <div class="card-body" style="display: flex; flex-direction: column; gap: 20px; max-width: 600px">
        <!-- CPU -->
        <div>
          <div style="display: flex; justify-content: space-between; margin-bottom: 6px">
            <span style="font-size: 14px; font-weight: 500">CPU</span>
            <span style="font-size: 13px; color: var(--text-muted)">{{ Math.round(metrics.cpu_usage_pct) }}%</span>
          </div>
          <div style="background: var(--border-color); border-radius: 6px; height: 10px; overflow: hidden">
            <div :style="{ width: Math.round(metrics.cpu_usage_pct) + '%', height: '100%', background: meterColor(Math.round(metrics.cpu_usage_pct)), borderRadius: '6px', transition: 'width 0.4s' }"></div>
          </div>
        </div>
        <!-- Memory -->
        <div>
          <div style="display: flex; justify-content: space-between; margin-bottom: 6px">
            <span style="font-size: 14px; font-weight: 500">Memory</span>
            <span style="font-size: 13px; color: var(--text-muted)">{{ fmtBytes(metrics.mem_used_bytes) }} / {{ fmtBytes(metrics.mem_total_bytes) }}</span>
          </div>
          <div style="background: var(--border-color); border-radius: 6px; height: 10px; overflow: hidden">
            <div :style="{ width: pct(metrics.mem_used_bytes, metrics.mem_total_bytes) + '%', height: '100%', background: meterColor(pct(metrics.mem_used_bytes, metrics.mem_total_bytes)), borderRadius: '6px', transition: 'width 0.4s' }"></div>
          </div>
        </div>
        <!-- Swap -->
        <div>
          <div style="display: flex; justify-content: space-between; margin-bottom: 6px">
            <span style="font-size: 14px; font-weight: 500">Swap</span>
            <span style="font-size: 13px; color: var(--text-muted)">{{ metrics.swap_total_bytes > 0 ? `${fmtBytes(metrics.swap_used_bytes)} / ${fmtBytes(metrics.swap_total_bytes)}` : "N/A" }}</span>
          </div>
          <div style="background: var(--border-color); border-radius: 6px; height: 10px; overflow: hidden">
            <div :style="{ width: pct(metrics.swap_used_bytes, metrics.swap_total_bytes) + '%', height: '100%', background: meterColor(pct(metrics.swap_used_bytes, metrics.swap_total_bytes)), borderRadius: '6px', transition: 'width 0.4s' }"></div>
          </div>
        </div>
        <!-- Disk -->
        <div>
          <div style="display: flex; justify-content: space-between; margin-bottom: 6px">
            <span style="font-size: 14px; font-weight: 500">Disk</span>
            <span style="font-size: 13px; color: var(--text-muted)">{{ fmtBytes(metrics.disk_used_bytes) }} / {{ fmtBytes(metrics.disk_total_bytes) }}</span>
          </div>
          <div style="background: var(--border-color); border-radius: 6px; height: 10px; overflow: hidden">
            <div :style="{ width: pct(metrics.disk_used_bytes, metrics.disk_total_bytes) + '%', height: '100%', background: meterColor(pct(metrics.disk_used_bytes, metrics.disk_total_bytes)), borderRadius: '6px', transition: 'width 0.4s' }"></div>
          </div>
        </div>
        <div style="font-size: 12px; color: var(--text-muted); margin-top: 4px">
          Last updated: {{ metrics.checked_at ? new Date(metrics.checked_at).toLocaleString() : "\u2014" }} &nbsp;&middot;&nbsp; refreshes every 30 s
        </div>
      </div>
    </div>
  </div>
</template>
