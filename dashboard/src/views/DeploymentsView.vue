<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api, type Deployment } from "../api";

const deps = ref<Deployment[]>([]);
const loading = ref(true);
const error = ref("");

const successCount = () => deps.value.filter((d) => d.status === "success").length;
const inProgressCount = () => deps.value.filter((d) => d.status === "pending" || d.status === "building").length;

function statusClass(status: string): string {
  if (status === "success") return "running";
  if (status === "failed") return "exited";
  return "created";
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    deps.value = await api.listDeployments();
  } catch (err: unknown) {
    error.value = err instanceof Error ? err.message : "Unknown error";
  }
  loading.value = false;
}

async function redeploy(id: number) {
  await api.redeploy(id);
  load();
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
        <div class="stat-icon green">&#x1F680;</div>
        <div class="stat-value">{{ deps.length }}</div>
        <div class="stat-label">Total Deployments</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon blue">&#x2713;</div>
        <div class="stat-value">{{ successCount() }}</div>
        <div class="stat-label">Successful</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon yellow">&#x23F3;</div>
        <div class="stat-value">{{ inProgressCount() }}</div>
        <div class="stat-label">In Progress</div>
      </div>
    </div>

    <div v-if="deps.length === 0" class="empty-state">
      <div class="empty-icon">&#x1F680;</div>
      <h3>No deployments yet</h3>
      <p>Push to a connected repository to trigger a deployment.</p>
    </div>
    <div v-else class="container-grid">
      <div v-for="d in deps" :key="d.id" class="container-row">
        <div class="container-status" :class="statusClass(d.status)"></div>
        <div>
          <div class="container-name">Deployment #{{ d.id }}</div>
          <div class="container-image">{{ d.commit_sha ? d.commit_sha.slice(0, 7) : "N/A" }} — {{ d.created_at }}</div>
        </div>
        <div><span class="container-state-badge" :class="statusClass(d.status)">{{ d.status }}</span></div>
        <div>
          <span v-if="d.domain" class="badge badge-info">{{ d.domain }}</span>
        </div>
        <div class="container-actions">
          <button class="btn btn-ghost btn-sm" @click="redeploy(d.id)">&#x21bb; Redeploy</button>
        </div>
      </div>
    </div>
  </div>
</template>
