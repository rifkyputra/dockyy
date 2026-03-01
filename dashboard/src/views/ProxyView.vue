<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api, type ProxyRoute } from "../api";

const traefikRunning = ref(false);
const network = ref("");
const container = ref("");
const routes = ref<ProxyRoute[]>([]);
const loading = ref(true);
const error = ref("");
const starting = ref(false);

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const [status, routesList] = await Promise.all([api.proxyStatus(), api.proxyRoutes()]);
    traefikRunning.value = status.traefik_running;
    network.value = status.network;
    container.value = status.container;
    routes.value = routesList;
  } catch (err: unknown) {
    error.value = err instanceof Error ? err.message : "Unknown error";
  }
  loading.value = false;
}

async function ensureTraefik() {
  starting.value = true;
  try {
    await api.ensureTraefik();
    load();
  } catch (err: unknown) {
    alert(err instanceof Error ? err.message : "Failed to start Traefik");
  }
  starting.value = false;
}

onMounted(load);
</script>

<template>
  <div v-if="loading" class="spinner"></div>
  <div v-else-if="error" class="empty-state">
    <div class="empty-icon">&#x26A0;&#xFE0F;</div>
    <h3>Error loading data</h3>
    <p>{{ error }}</p>
  </div>
  <div v-else>
    <div class="stats-grid">
      <div class="stat-card">
        <div class="stat-icon" :class="traefikRunning ? 'green' : 'yellow'">&#x1F500;</div>
        <div class="stat-value" style="font-size: 1.1rem">{{ traefikRunning ? "Running" : "Stopped" }}</div>
        <div class="stat-label">Traefik</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon blue">&#x1F310;</div>
        <div class="stat-value">{{ routes.length }}</div>
        <div class="stat-label">Active Routes</div>
      </div>
      <div class="stat-card" style="display: flex; flex-direction: column; align-items: flex-start; gap: 6px; padding: 16px">
        <div style="font-size: 13px; color: var(--text-muted)">Container</div>
        <code style="font-size: 12px">{{ container }}</code>
        <div style="font-size: 13px; color: var(--text-muted); margin-top: 4px">Network</div>
        <code style="font-size: 12px">{{ network }}</code>
      </div>
      <div class="stat-card" style="display: flex; align-items: center; justify-content: center">
        <button class="btn" :class="traefikRunning ? 'btn-ghost' : 'btn-primary'" :disabled="starting" @click="ensureTraefik">
          {{ starting ? "Starting..." : traefikRunning ? "\u21bb Restart" : "\u25B6 Start Traefik" }}
        </button>
      </div>
    </div>

    <div class="card">
      <div class="card-header"><h2 class="card-title">Active Routes</h2></div>
      <div class="card-body" style="padding: 0">
        <div v-if="routes.length === 0" style="padding: 24px; text-align: center; color: var(--text-muted)">
          No active routes. Set a <strong>domain</strong> on a repository and deploy to create one.
        </div>
        <table v-else style="width: 100%; border-collapse: collapse; font-size: 14px">
          <thead>
            <tr style="border-bottom: 1px solid var(--border-color); text-align: left">
              <th style="padding: 10px 16px; color: var(--text-muted); font-weight: 500">Container</th>
              <th style="padding: 10px 16px; color: var(--text-muted); font-weight: 500">Domain</th>
              <th style="padding: 10px 16px; color: var(--text-muted); font-weight: 500">Port</th>
              <th style="padding: 10px 16px; color: var(--text-muted); font-weight: 500">Status</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="r in routes" :key="r.container_id" style="border-bottom: 1px solid var(--border-color)">
              <td style="padding: 10px 16px"><code>{{ r.container_name }}</code></td>
              <td style="padding: 10px 16px">
                <a :href="'http://' + r.domain" target="_blank" style="color: var(--primary); text-decoration: none">{{ r.domain }}</a>
              </td>
              <td style="padding: 10px 16px"><span class="badge badge-info">{{ r.port }}</span></td>
              <td style="padding: 10px 16px"><span class="container-state-badge running">{{ r.status }}</span></td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </div>
</template>
