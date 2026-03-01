<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api, type Container } from "../api";
import LogModal from "../components/LogModal.vue";

const containers = ref<Container[]>([]);
const loading = ref(true);
const error = ref("");

const logModal = ref(false);
const logTitle = ref("");
const logContent = ref("");

const running = () => containers.value.filter((c) => c.state === "running").length;
const stopped = () => containers.value.filter((c) => c.state !== "running").length;

async function load() {
  loading.value = true;
  error.value = "";
  try {
    containers.value = await api.listContainers(true);
  } catch (err: unknown) {
    error.value = err instanceof Error ? err.message : "Unknown error";
  }
  loading.value = false;
}

async function start(id: string) {
  await api.startContainer(id);
  load();
}
async function stop(id: string) {
  await api.stopContainer(id);
  load();
}
async function restart(id: string) {
  await api.restartContainer(id);
  load();
}
async function remove(id: string) {
  if (confirm("Remove this container?")) {
    await api.removeContainer(id);
    load();
  }
}
async function showLogs(c: Container) {
  const { logs } = await api.containerLogs(c.id, 200);
  logTitle.value = `Logs — ${c.name}`;
  logContent.value = logs;
  logModal.value = true;
}

function formatPorts(c: Container): string[] {
  return c.ports.filter((p) => p.public_port).map((p) => `${p.public_port}\u2192${p.private_port}`);
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
        <div class="stat-icon blue">&#x1F4E6;</div>
        <div class="stat-value">{{ containers.length }}</div>
        <div class="stat-label">Total Containers</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon green">&#x2713;</div>
        <div class="stat-value">{{ running() }}</div>
        <div class="stat-label">Running</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon yellow">&#x23F8;</div>
        <div class="stat-value">{{ stopped() }}</div>
        <div class="stat-label">Stopped</div>
      </div>
    </div>

    <div v-if="containers.length === 0" class="empty-state">
      <div class="empty-icon">&#x1F4E6;</div>
      <h3>No containers found</h3>
      <p>Start a container with Docker to see it here.</p>
    </div>
    <div v-else class="container-grid">
      <div v-for="c in containers" :key="c.id" class="container-row">
        <div class="container-status" :class="c.state"></div>
        <div>
          <div class="container-name">{{ c.name }}</div>
          <div class="container-image">{{ c.image }}</div>
        </div>
        <div>
          <template v-if="formatPorts(c).length">
            <span v-for="p in formatPorts(c)" :key="p" class="badge badge-info">{{ p }}</span>
          </template>
          <span v-else class="badge badge-warning">No ports</span>
        </div>
        <div><span class="container-state-badge" :class="c.state">{{ c.state }}</span></div>
        <div class="container-actions">
          <template v-if="c.state === 'running'">
            <button class="btn btn-ghost btn-sm btn-icon" title="Stop" @click="stop(c.id)">&#x23F9;</button>
            <button class="btn btn-ghost btn-sm btn-icon" title="Restart" @click="restart(c.id)">&#x21bb;</button>
          </template>
          <button v-else class="btn btn-success btn-sm btn-icon" title="Start" @click="start(c.id)">&#x25B6;</button>
          <button class="btn btn-ghost btn-sm btn-icon" title="Logs" @click="showLogs(c)">&#x1F4CB;</button>
          <button class="btn btn-danger btn-sm btn-icon" title="Remove" @click="remove(c.id)">&#x2715;</button>
        </div>
      </div>
    </div>
  </div>

  <LogModal :show="logModal" :title="logTitle" :content="logContent" @close="logModal = false" />
</template>
