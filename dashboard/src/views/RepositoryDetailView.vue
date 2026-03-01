<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useRoute, useRouter } from "vue-router";
import { marked } from "marked";
import { api, type Repository, type Deployment } from "../api";
import LogModal from "../components/LogModal.vue";

const route = useRoute();
const router = useRouter();

const repo = ref<Repository | null>(null);
const deps = ref<Deployment[]>([]);
const fsStatus = ref({ has_git_repo: false, has_docker_compose: false, repo_path: "" });
const readme = ref("");
const composeFiles = ref<{ path: string; content: string; override_content: string | null }[]>([]);
const activeComposeFile = ref("");
const composeEditMode = ref(false);
const composeEditContent = ref("");
const loading = ref(true);
const error = ref("");

// Edit modal
const showEditModal = ref(false);
const editForm = ref({ name: "", owner: "", url: "", default_branch: "", ssh_password: "", is_private: false, domain: "", proxy_port: "3000" });
const editError = ref("");

// Proxy form
const proxyDomain = ref("");
const proxyPort = ref("3000");
const proxyMsg = ref("");
const proxyMsgOk = ref(true);

// Action states
const cloning = ref(false);

function showProxyMessage(text: string, ok = true) {
  proxyMsg.value = text;
  proxyMsgOk.value = ok;
  setTimeout(() => { proxyMsg.value = ""; }, 3000);
}

async function load() {
  loading.value = true;
  error.value = "";
  const id = Number(route.params.id);
  if (!id) {
    router.push("/repositories");
    return;
  }
  try {
    repo.value = await api.getRepository(id);
    deps.value = await api.listDeploymentsByRepo(id);

    proxyDomain.value = repo.value.domain || "";
    proxyPort.value = String(repo.value.proxy_port ?? 3000);

    try {
      fsStatus.value = await api.getFilesystemStatus(id);
      if (fsStatus.value.has_git_repo) {
        const [r, cf] = await Promise.all([api.getReadme(id), api.getComposeFiles(id)]);
        readme.value = r.content;
        composeFiles.value = cf;
        if (cf.length > 0 && !activeComposeFile.value) {
          activeComposeFile.value = cf[0].path;
        }
      }
    } catch {
      // rich details not available
    }
  } catch (err: unknown) {
    error.value = err instanceof Error ? err.message : "Unknown error";
  }
  loading.value = false;
}

async function cloneRepo() {
  cloning.value = true;
  try {
    await api.cloneRepository(Number(route.params.id));
    load();
  } catch (err: unknown) {
    alert(err instanceof Error ? err.message : "Clone failed");
  }
  cloning.value = false;
}

async function pullRepo() {
  try {
    const res = await api.gitPull(Number(route.params.id));
    alert(res.message);
    load();
  } catch (err: unknown) {
    alert(err instanceof Error ? err.message : "Pull failed");
  }
}

async function fetchRepo() {
  try {
    const res = await api.gitFetch(Number(route.params.id));
    alert(res.message);
    load();
  } catch (err: unknown) {
    alert(err instanceof Error ? err.message : "Fetch failed");
  }
}

async function composeUp(file: string) {
  try {
    await api.dockerComposeUp(Number(route.params.id), file);
    load();
  } catch (err: unknown) {
    alert(err instanceof Error ? err.message : "Deployment failed");
  }
}

function startEditCompose(cf: { path: string; content: string; override_content: string | null }) {
  composeEditContent.value = cf.override_content ?? cf.content;
  composeEditMode.value = true;
}

function cancelEditCompose() {
  composeEditMode.value = false;
}

async function saveComposeOverride() {
  try {
    await api.saveComposeOverride(Number(route.params.id), activeComposeFile.value, composeEditContent.value);
    composeEditMode.value = false;
    await load();
  } catch (err: unknown) {
    alert(err instanceof Error ? err.message : "Save failed");
  }
}

async function resetComposeOverride(filename: string) {
  try {
    await api.resetComposeOverride(Number(route.params.id), filename);
    composeEditMode.value = false;
    await load();
  } catch (err: unknown) {
    alert(err instanceof Error ? err.message : "Reset failed");
  }
}

async function saveProxy() {
  const domain = proxyDomain.value.trim();
  const port = parseInt(proxyPort.value);
  try {
    await api.updateRepository(repo.value!.id, {
      domain: domain || null,
      proxy_port: isNaN(port) ? null : port,
    });
    showProxyMessage("Saved. Redeploy to apply.");
  } catch (err: unknown) {
    showProxyMessage(err instanceof Error ? err.message : "Save failed", false);
  }
}

async function redeploy(id: number) {
  await api.redeploy(id);
  load();
}

function openEditModal() {
  if (!repo.value) return;
  editForm.value = {
    name: repo.value.name,
    owner: repo.value.owner,
    url: repo.value.url,
    default_branch: repo.value.default_branch,
    ssh_password: repo.value.ssh_password || "",
    is_private: repo.value.is_private,
    domain: repo.value.domain || "",
    proxy_port: String(repo.value.proxy_port ?? 3000),
  };
  editError.value = "";
  showEditModal.value = true;
}

async function submitEdit() {
  editError.value = "";
  const proxyPortVal = editForm.value.proxy_port ? parseInt(editForm.value.proxy_port) : null;
  try {
    await api.updateRepository(repo.value!.id, {
      name: editForm.value.name,
      owner: editForm.value.owner,
      url: editForm.value.url,
      default_branch: editForm.value.default_branch,
      ssh_password: editForm.value.ssh_password || null,
      is_private: editForm.value.is_private,
      domain: editForm.value.domain || null,
      proxy_port: proxyPortVal && !isNaN(proxyPortVal) ? proxyPortVal : null,
    });
    showEditModal.value = false;
    load();
  } catch (err: unknown) {
    editError.value = err instanceof Error ? err.message : "Failed to update repository";
  }
}

function statusClass(status: string): string {
  if (status === "success") return "running";
  if (status === "failed") return "exited";
  return "created";
}

function renderMarkdown(content: string): string {
  return marked.parse(content) as string;
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
  <div v-else-if="repo">
    <div style="margin-bottom: 16px">
      <button class="btn btn-ghost btn-sm" @click="router.push('/repositories')">&larr; Back to Repositories</button>
    </div>
    <div class="repo-detail-grid">
      <!-- Left column -->
      <div style="display: flex; flex-direction: column; gap: 24px; min-width: 0">
        <!-- Repo info card -->
        <div class="card">
          <div class="card-header" style="display: flex; justify-content: space-between; align-items: center">
            <div style="display: flex; align-items: center; gap: 12px">
              <h2 class="card-title" style="margin: 0">{{ repo.owner }}/{{ repo.name }}</h2>
              <button class="btn btn-ghost btn-sm" @click="openEditModal">&#x270F;&#xFE0F; Edit</button>
            </div>
            <span class="badge" :class="repo.is_private ? 'badge-warning' : 'badge-success'">{{ repo.is_private ? "Private" : "Public" }}</span>
          </div>
          <div class="card-body">
            <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 16px">
              <div>
                <span style="color: var(--text-muted); font-size: 12px; text-transform: uppercase; letter-spacing: 0.05em">URL</span><br />
                <a :href="repo.url" target="_blank" style="color: var(--accent); text-decoration: none; font-size: 14px; word-break: break-all">{{ repo.url }}</a>
              </div>
              <div>
                <span style="color: var(--text-muted); font-size: 12px; text-transform: uppercase; letter-spacing: 0.05em">Branch</span><br />
                <strong style="font-size: 14px">{{ repo.default_branch }}</strong>
              </div>
              <div>
                <span style="color: var(--text-muted); font-size: 12px; text-transform: uppercase; letter-spacing: 0.05em">Created</span><br />
                <span style="font-size: 14px">{{ new Date(repo.created_at).toLocaleString() }}</span>
              </div>
            </div>
          </div>
        </div>

        <!-- Docker Compose card -->
        <div class="card">
          <div class="card-header"><h3 class="card-title">Docker Compose</h3></div>
          <div class="card-body" style="padding: 0">
            <template v-if="composeFiles.length > 0">
              <div class="tabs" style="padding: 12px 24px 0">
                <button
                  v-for="cf in composeFiles"
                  :key="cf.path"
                  class="tab-btn"
                  :class="{ active: cf.path === activeComposeFile }"
                  @click="activeComposeFile = cf.path; composeEditMode = false"
                >
                  {{ cf.path }}
                  <span v-if="cf.override_content" class="badge badge-warning" style="margin-left: 6px; font-size: 9px; padding: 1px 5px">Modified</span>
                </button>
              </div>
              <div v-for="cf in composeFiles" :key="cf.path" class="tab-content" :class="{ active: cf.path === activeComposeFile }">
                <div style="padding: 0 24px 24px">
                  <div style="display: flex; justify-content: flex-end; gap: 8px; margin-bottom: 12px; flex-wrap: wrap">
                    <template v-if="composeEditMode">
                      <button class="btn btn-primary btn-sm" @click="saveComposeOverride">Save</button>
                      <button class="btn btn-ghost btn-sm" @click="cancelEditCompose">Cancel</button>
                    </template>
                    <template v-else>
                      <button class="btn btn-ghost btn-sm" @click="startEditCompose(cf)">Edit</button>
                      <button v-if="cf.override_content" class="btn btn-ghost btn-sm" style="color: var(--danger)" @click="resetComposeOverride(cf.path)">Reset</button>
                    </template>
                    <button class="btn btn-success btn-sm" @click="composeUp(cf.path)">&#x25B6; Trigger</button>
                  </div>
                  <textarea
                    v-if="composeEditMode"
                    v-model="composeEditContent"
                    style="width: 100%; min-height: 300px; font-size: 13px; font-family: monospace; background: var(--bg-primary); padding: 16px; border-radius: var(--radius-sm); border: 1px solid var(--accent); color: var(--text-secondary); line-height: 1.5; resize: vertical; tab-size: 2; box-sizing: border-box"
                  ></textarea>
                  <pre v-else style="font-size: 13px; background: var(--bg-primary); padding: 16px; border-radius: var(--radius-sm); overflow-x: auto; max-width: 100%; border: 1px solid var(--border); color: var(--text-secondary); line-height: 1.5">{{ cf.override_content ?? cf.content }}</pre>
                  <div v-if="cf.override_content && !composeEditMode" style="margin-top: 8px; font-size: 11px; color: var(--text-muted)">
                    Showing modified version. <button style="color: var(--accent); background: none; border: none; cursor: pointer; font-size: 11px; padding: 0; text-decoration: underline" @click="resetComposeOverride(cf.path)">Reset to original</button>
                  </div>
                </div>
              </div>
            </template>
            <div v-else style="padding: 40px; text-align: center; color: var(--text-muted)">
              <div style="font-size: 32px; margin-bottom: 12px">&#x1F4C4;</div>
              <p>No docker-compose files detected.</p>
            </div>
          </div>
        </div>

        <!-- README card -->
        <div v-if="readme" class="card">
          <div class="card-header"><h3 class="card-title">README.md</h3></div>
          <div class="card-body">
            <div class="readme-content" v-html="renderMarkdown(readme)"></div>
          </div>
        </div>
      </div>

      <!-- Right column -->
      <div style="display: flex; flex-direction: column; gap: 24px">
        <!-- Repository Actions -->
        <div class="card">
          <div class="card-header"><h3 class="card-title">Repository Actions</h3></div>
          <div class="card-body" style="display: flex; flex-direction: column; gap: 12px">
            <div style="font-size: 13px; color: var(--text-muted); margin-bottom: 4px">
              <strong>Status:</strong>
              <span v-if="fsStatus.has_git_repo" class="badge badge-success">Cloned</span>
              <span v-else class="badge badge-warning">Not Cloned</span>
            </div>
            <button class="btn btn-primary" style="width: 100%; justify-content: center" :disabled="cloning" @click="cloneRepo">
              {{ cloning ? "Cloning..." : fsStatus.has_git_repo ? "Re-Clone" : "Clone Repository" }}
            </button>
            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 8px">
              <button class="btn btn-ghost btn-sm" :disabled="!fsStatus.has_git_repo" style="justify-content: center" @click="pullRepo">Git Pull</button>
              <button class="btn btn-ghost btn-sm" :disabled="!fsStatus.has_git_repo" style="justify-content: center" @click="fetchRepo">Git Fetch</button>
            </div>
            <div v-if="fsStatus.repo_path" style="font-size: 11px; color: var(--text-muted); word-break: break-all; background: var(--bg-primary); padding: 8px; border-radius: 4px; border: 1px solid var(--border)">
              <code>{{ fsStatus.repo_path }}</code>
            </div>
          </div>
        </div>

        <!-- Reverse Proxy -->
        <div class="card">
          <div class="card-header" style="display: flex; justify-content: space-between; align-items: center">
            <h3 class="card-title">Reverse Proxy</h3>
            <span v-if="repo.domain" class="badge badge-success" style="font-size: 10px">Active</span>
            <span v-else class="badge badge-warning" style="font-size: 10px">None</span>
          </div>
          <div class="card-body">
            <form @submit.prevent="saveProxy" style="display: flex; flex-direction: column; gap: 12px">
              <div class="form-group" style="margin: 0">
                <label class="form-label" style="font-size: 11px">Domain</label>
                <input v-model="proxyDomain" class="form-input" placeholder="app.example.com" style="font-size: 13px" />
              </div>
              <div class="form-group" style="margin: 0">
                <label class="form-label" style="font-size: 11px">Internal Port</label>
                <input v-model="proxyPort" class="form-input" type="number" min="1" max="65535" style="font-size: 13px" />
              </div>
              <div v-if="proxyMsg" style="font-size: 12px" :style="{ color: proxyMsgOk ? 'var(--success)' : 'var(--danger)' }">{{ proxyMsg }}</div>
              <button class="btn btn-primary btn-sm" type="submit" style="width: 100%; justify-content: center">Save Proxy</button>
            </form>
          </div>
        </div>

        <!-- Latest Deployments -->
        <div class="card">
          <div class="card-header"><h3 class="card-title">Latest Deployments</h3></div>
          <div class="card-body" style="padding: 0">
            <div v-if="deps.length === 0" style="padding: 16px; text-align: center; color: var(--text-muted); font-size: 13px">No history.</div>
            <div v-else style="display: flex; flex-direction: column">
              <div v-for="d in deps.slice(0, 5)" :key="d.id" style="padding: 12px 16px; border-bottom: 1px solid var(--border); display: flex; justify-content: space-between; align-items: center">
                <div style="display: flex; align-items: center; gap: 10px">
                  <div class="container-status" :class="statusClass(d.status)" style="width: 8px; height: 8px"></div>
                  <div>
                    <div style="font-size: 13px; font-weight: 500">#{{ d.id }}</div>
                    <div style="font-size: 11px; color: var(--text-muted)">{{ d.commit_sha ? d.commit_sha.slice(0, 7) : "N/A" }}</div>
                  </div>
                </div>
                <button class="btn btn-ghost btn-sm" style="padding: 2px 6px; font-size: 10px" @click="redeploy(d.id)">&#x21bb;</button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>

  <!-- Edit Repository Modal -->
  <LogModal :show="showEditModal" title="Edit Repository" content="" @close="showEditModal = false">
    <template #default>
      <form @submit.prevent="submitEdit" style="display: flex; flex-direction: column; gap: 10px">
        <div class="form-group">
          <label class="form-label">Name</label>
          <input v-model="editForm.name" class="form-input" required />
        </div>
        <div class="form-group">
          <label class="form-label">Owner</label>
          <input v-model="editForm.owner" class="form-input" required />
        </div>
        <div class="form-group">
          <label class="form-label">URL</label>
          <input v-model="editForm.url" class="form-input" type="url" required />
        </div>
        <div class="form-group">
          <label class="form-label">Default Branch</label>
          <input v-model="editForm.default_branch" class="form-input" />
        </div>
        <div class="form-group">
          <label class="form-label">SSH Key / Password (Optional)</label>
          <textarea v-model="editForm.ssh_password" class="form-input" rows="3" placeholder="Paste your private SSH key here"></textarea>
        </div>
        <div class="form-group" style="display: flex; align-items: center; gap: 8px">
          <input v-model="editForm.is_private" type="checkbox" id="edit-repo-private" />
          <label class="form-label" style="margin: 0" for="edit-repo-private">Private Repository</label>
        </div>
        <div style="border-top: 1px solid var(--border-color); padding-top: 10px; margin-top: 4px">
          <div style="font-size: 12px; color: var(--text-muted); margin-bottom: 8px">Optional — reverse proxy settings</div>
          <div class="form-group">
            <label class="form-label">Domain</label>
            <input v-model="editForm.domain" class="form-input" placeholder="app.example.com" />
          </div>
          <div class="form-group">
            <label class="form-label">Container Port</label>
            <input v-model="editForm.proxy_port" class="form-input" type="number" min="1" max="65535" style="max-width: 120px" />
          </div>
        </div>
        <div v-if="editError" class="form-error" style="color: red">{{ editError }}</div>
        <button class="btn btn-primary" type="submit">Save Changes</button>
      </form>
    </template>
  </LogModal>
</template>
