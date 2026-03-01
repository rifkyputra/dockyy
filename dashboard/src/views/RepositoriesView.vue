<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useRouter } from "vue-router";
import { api, type Repository } from "../api";
import LogModal from "../components/LogModal.vue";

const router = useRouter();
const repos = ref<Repository[]>([]);
const loading = ref(true);
const error = ref("");

// Add modal state
const showAddModal = ref(false);
const addForm = ref({
  name: "",
  owner: "",
  url: "",
  default_branch: "main",
  ssh_password: "",
  is_private: false,
  domain: "",
  proxy_port: "3000",
});
const addError = ref("");

async function load() {
  loading.value = true;
  error.value = "";
  try {
    repos.value = await api.listRepositories();
  } catch (err: unknown) {
    error.value = err instanceof Error ? err.message : "Unknown error";
  }
  loading.value = false;
}

async function deleteRepo(id: number) {
  if (confirm("Delete this repository?")) {
    await api.deleteRepository(id);
    load();
  }
}

function openDetail(id: number) {
  router.push(`/repository/${id}`);
}

async function submitAdd() {
  addError.value = "";
  const proxyPort = addForm.value.proxy_port ? parseInt(addForm.value.proxy_port) : null;
  try {
    await api.createRepository({
      name: addForm.value.name,
      owner: addForm.value.owner,
      url: addForm.value.url,
      default_branch: addForm.value.default_branch,
      ssh_password: addForm.value.ssh_password || null,
      is_private: addForm.value.is_private,
      domain: addForm.value.domain || null,
      proxy_port: proxyPort && !isNaN(proxyPort) ? proxyPort : null,
    });
    showAddModal.value = false;
    addForm.value = { name: "", owner: "", url: "", default_branch: "main", ssh_password: "", is_private: false, domain: "", proxy_port: "3000" };
    load();
  } catch (err: unknown) {
    addError.value = err instanceof Error ? err.message : "Failed to add repository";
  }
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
        <div class="stat-icon purple">&#x1F4C2;</div>
        <div class="stat-value">{{ repos.length }}</div>
        <div class="stat-label">Repositories</div>
      </div>
      <div class="stat-card" style="display: flex; align-items: center; justify-content: center">
        <button class="btn btn-primary" @click="showAddModal = true">+ Add Repository</button>
      </div>
    </div>

    <div v-if="repos.length === 0" class="empty-state">
      <div class="empty-icon">&#x1F4C2;</div>
      <h3>No repositories</h3>
      <p>Add a repository to enable push-to-deploy.</p>
    </div>
    <div v-else class="container-grid">
      <div v-for="r in repos" :key="r.id" class="container-row">
        <div class="container-status" :class="r.is_private ? 'exited' : 'running'"></div>
        <div>
          <div class="container-name">
            <a href="#" style="color: var(--primary); text-decoration: none" @click.prevent="openDetail(r.id)">{{ r.owner }}/{{ r.name }}</a>
          </div>
          <div class="container-image">{{ r.url }}</div>
        </div>
        <div>
          <span class="badge" :class="r.is_private ? 'badge-warning' : 'badge-success'">{{ r.is_private ? "Private" : "Public" }}</span>
        </div>
        <div>
          <span v-if="r.domain" class="badge badge-info" title="Proxy domain">&#x1F500; {{ r.domain }}</span>
          <span v-else style="font-size: 12px; color: var(--text-muted)">No proxy</span>
        </div>
        <div class="container-actions">
          <button class="btn btn-danger btn-sm" @click="deleteRepo(r.id)">Delete</button>
        </div>
      </div>
    </div>
  </div>

  <!-- Add Repository Modal -->
  <LogModal :show="showAddModal" title="Add Repository" content="" @close="showAddModal = false">
    <template #default>
      <form @submit.prevent="submitAdd" style="display: flex; flex-direction: column; gap: 10px">
        <div class="form-group">
          <label class="form-label">Name</label>
          <input v-model="addForm.name" class="form-input" required placeholder="my-repo" />
        </div>
        <div class="form-group">
          <label class="form-label">Owner</label>
          <input v-model="addForm.owner" class="form-input" required placeholder="username" />
        </div>
        <div class="form-group">
          <label class="form-label">URL</label>
          <input v-model="addForm.url" class="form-input" type="url" required placeholder="https://github.com/user/repo" />
        </div>
        <div class="form-group">
          <label class="form-label">Default Branch</label>
          <input v-model="addForm.default_branch" class="form-input" />
        </div>
        <div class="form-group">
          <label class="form-label">SSH Key / Password (Optional)</label>
          <textarea v-model="addForm.ssh_password" class="form-input" rows="3" placeholder="Paste your private SSH key here (e.g. for git@...)"></textarea>
        </div>
        <div class="form-group" style="display: flex; align-items: center; gap: 8px">
          <input v-model="addForm.is_private" type="checkbox" id="repo-private" />
          <label class="form-label" style="margin: 0" for="repo-private">Private Repository</label>
        </div>
        <div style="border-top: 1px solid var(--border-color); padding-top: 10px; margin-top: 4px">
          <div style="font-size: 12px; color: var(--text-muted); margin-bottom: 8px">Optional — reverse proxy settings</div>
          <div class="form-group">
            <label class="form-label">Domain</label>
            <input v-model="addForm.domain" class="form-input" placeholder="app.example.com" />
          </div>
          <div class="form-group">
            <label class="form-label">Container Port</label>
            <input v-model="addForm.proxy_port" class="form-input" type="number" min="1" max="65535" style="max-width: 120px" />
          </div>
        </div>
        <div v-if="addError" class="form-error" style="color: red">{{ addError }}</div>
        <button class="btn btn-primary" type="submit">Create</button>
      </form>
    </template>
  </LogModal>
</template>
