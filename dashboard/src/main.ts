import "./style.css";
import { api, type Container, type ServerMetrics } from "./api";

// ── State ──
let currentPage = "containers";
let currentRepoId: number | null = null;
let containers: Container[] = [];

// ── Auth check ──
function isLoggedIn(): boolean {
  return !!localStorage.getItem("dockyy_token");
}

// ── Render ──
function render() {
  const app = document.getElementById("app")!;
  if (!isLoggedIn()) {
    app.innerHTML = renderLogin();
    bindLogin();
    return;
  }
  app.innerHTML = renderLayout();
  bindNav();
  loadPage();
}

// ── Login page ──
function renderLogin(): string {
  return `
    <div class="login-wrapper">
      <div class="login-card">
        <div class="login-header">
          <div class="login-logo">D</div>
          <h1>Welcome to Dockyy</h1>
          <p>Sign in to manage your containers</p>
        </div>
        <form id="login-form">
          <div class="form-group">
            <label class="form-label" for="username">Username</label>
            <input class="form-input" id="username" type="text" placeholder="admin" autocomplete="username" />
          </div>
          <div class="form-group">
            <label class="form-label" for="password">Password</label>
            <input class="form-input" id="password" type="password" placeholder="••••••••" autocomplete="current-password" />
          </div>
          <div id="login-error" class="form-error" style="display:none"></div>
          <button class="btn btn-primary btn-login" type="submit">Sign In</button>
        </form>
      </div>
    </div>`;
}

function bindLogin() {
  document
    .getElementById("login-form")
    ?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const username = (document.getElementById("username") as HTMLInputElement)
        .value;
      const password = (document.getElementById("password") as HTMLInputElement)
        .value;
      const errorEl = document.getElementById("login-error")!;
      try {
        const { token } = await api.login(username, password);
        localStorage.setItem("dockyy_token", token);
        updateHash();
        render();
      } catch (err: unknown) {
        errorEl.style.display = "block";
        errorEl.textContent =
          err instanceof Error ? err.message : "Login failed";
      }
    });
}

// ── Layout ──
function renderLayout(): string {
  return `
    <div class="layout">
      <aside class="sidebar">
        <div class="sidebar-header">
          <div class="sidebar-logo">D</div>
          <span class="sidebar-title">Dockyy</span>
          <span class="sidebar-version">v0.1</span>
        </div>
        <nav class="sidebar-nav">
          <div class="nav-section">Management</div>
          <button class="nav-item ${currentPage === "containers" ? "active" : ""}" data-page="containers">
            <span class="icon">📦</span> Containers
          </button>
          <button class="nav-item ${currentPage === "repositories" ? "active" : ""}" data-page="repositories">
            <span class="icon">📂</span> Repositories
          </button>
          <button class="nav-item ${currentPage === "deployments" ? "active" : ""}" data-page="deployments">
            <span class="icon">🚀</span> Deployments
          </button>
          <div class="nav-section">System</div>
          <button class="nav-item ${currentPage === "health" ? "active" : ""}" data-page="health">
            <span class="icon">🏥</span> Health
          </button>
          <button class="nav-item ${currentPage === "settings" ? "active" : ""}" data-page="settings">
            <span class="icon">⚙️</span> Settings
          </button>
        </nav>
        <div class="sidebar-footer">
          <button class="btn btn-ghost btn-sm" id="logout-btn" style="width:100%">Sign Out</button>
        </div>
      </aside>
      <div class="main-content">
        <header class="topbar">
          <h1 class="topbar-title" id="page-title">${pageTitle()}</h1>
          <div class="topbar-actions">
            <button class="btn btn-ghost btn-sm" id="refresh-btn">↻ Refresh</button>
          </div>
        </header>
        <main class="page-content" id="page-content">
          <div class="spinner"></div>
        </main>
      </div>
    </div>
    <div id="modal-root"></div>`;
}

function pageTitle(): string {
  const titles: Record<string, string> = {
    containers: "Containers",
    repositories: "Repositories",
    repository_detail: "Repository Details",
    deployments: "Deployments",
    health: "Server Health",
    settings: "Settings",
  };
  return titles[currentPage] || "Dashboard";
}

function bindNav() {
  document.querySelectorAll(".nav-item[data-page]").forEach((btn) => {
    btn.addEventListener("click", () => {
      currentPage = (btn as HTMLElement).dataset.page!;
      currentRepoId = null;
      updateHash();
      render();
    });
  });
  document.getElementById("logout-btn")?.addEventListener("click", () => {
    localStorage.removeItem("dockyy_token");
    render();
  });
  document
    .getElementById("refresh-btn")
    ?.addEventListener("click", () => loadPage());
}

// ── Pages ──
async function loadPage() {
  const content = document.getElementById("page-content")!;
  content.innerHTML = '<div class="spinner"></div>';
  try {
    switch (currentPage) {
      case "containers":
        await loadContainers(content);
        break;
      case "repositories":
        await loadRepositories(content);
        break;
      case "repository_detail":
        await loadRepositoryDetail(content);
        break;
      case "deployments":
        await loadDeployments(content);
        break;
      case "health":
        await loadHealth(content);
        break;
      case "settings":
        loadSettings(content);
        break;
    }
  } catch (err) {
    content.innerHTML = `<div class="empty-state"><div class="empty-icon">⚠️</div><h3>Error loading data</h3><p>${err instanceof Error ? err.message : "Unknown error"}</p></div>`;
  }
}

// ── Containers page ──
async function loadContainers(el: HTMLElement) {
  containers = await api.listContainers(true);
  const running = containers.filter((c) => c.state === "running").length;
  const stopped = containers.filter((c) => c.state !== "running").length;

  el.innerHTML = `
    <div class="stats-grid">
      <div class="stat-card">
        <div class="stat-icon blue">📦</div>
        <div class="stat-value">${containers.length}</div>
        <div class="stat-label">Total Containers</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon green">✓</div>
        <div class="stat-value">${running}</div>
        <div class="stat-label">Running</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon yellow">⏸</div>
        <div class="stat-value">${stopped}</div>
        <div class="stat-label">Stopped</div>
      </div>
    </div>
    ${
      containers.length === 0
        ? `
      <div class="empty-state">
        <div class="empty-icon">📦</div>
        <h3>No containers found</h3>
        <p>Start a container with Docker to see it here.</p>
      </div>`
        : `
      <div class="container-grid">
        ${containers
          .map(
            (c) => `
          <div class="container-row" data-id="${c.id}">
            <div class="container-status ${c.state}"></div>
            <div>
              <div class="container-name">${esc(c.name)}</div>
              <div class="container-image">${esc(c.image)}</div>
            </div>
            <div>${
              c.ports
                .filter((p) => p.public_port)
                .map(
                  (p) =>
                    `<span class="badge badge-info">${p.public_port}→${p.private_port}</span>`,
                )
                .join(" ") ||
              '<span class="badge badge-warning">No ports</span>'
            }</div>
            <div><span class="container-state-badge ${c.state}">${c.state}</span></div>
            <div class="container-actions">
              ${
                c.state === "running"
                  ? `
                <button class="btn btn-ghost btn-sm btn-icon act-stop" title="Stop">⏹</button>
                <button class="btn btn-ghost btn-sm btn-icon act-restart" title="Restart">↻</button>
              `
                  : `
                <button class="btn btn-success btn-sm btn-icon act-start" title="Start">▶</button>
              `
              }
              <button class="btn btn-ghost btn-sm btn-icon act-logs" title="Logs">📋</button>
              <button class="btn btn-danger btn-sm btn-icon act-remove" title="Remove">✕</button>
            </div>
          </div>
        `,
          )
          .join("")}
      </div>`
    }`;

  // Bind container actions
  el.querySelectorAll(".container-row").forEach((row) => {
    const id = (row as HTMLElement).dataset.id!;
    row.querySelector(".act-start")?.addEventListener("click", async (e) => {
      e.stopPropagation();
      await api.startContainer(id);
      loadPage();
    });
    row.querySelector(".act-stop")?.addEventListener("click", async (e) => {
      e.stopPropagation();
      await api.stopContainer(id);
      loadPage();
    });
    row.querySelector(".act-restart")?.addEventListener("click", async (e) => {
      e.stopPropagation();
      await api.restartContainer(id);
      loadPage();
    });
    row.querySelector(".act-remove")?.addEventListener("click", async (e) => {
      e.stopPropagation();
      if (confirm("Remove this container?")) {
        await api.removeContainer(id);
        loadPage();
      }
    });
    row.querySelector(".act-logs")?.addEventListener("click", async (e) => {
      e.stopPropagation();
      const c = containers.find((c) => c.id === id);
      const { logs } = await api.containerLogs(id, 200);
      showLogModal(c?.name || id, logs);
    });
  });
}

// ── Repositories page ──
async function loadRepositories(el: HTMLElement) {
  const repos = await api.listRepositories();
  el.innerHTML = `
    <div class="stats-grid">
      <div class="stat-card">
        <div class="stat-icon purple">📂</div>
        <div class="stat-value">${repos.length}</div>
        <div class="stat-label">Repositories</div>
      </div>
      <div class="stat-card" style="display:flex; align-items:center; justify-content:center;">
        <button id="btn-add-repo" class="btn btn-primary">+ Add Repository</button>
      </div>
    </div>
    ${
      repos.length === 0
        ? `
      <div class="empty-state">
        <div class="empty-icon">📂</div>
        <h3>No repositories</h3>
        <p>Add a repository to enable push-to-deploy.</p>
      </div>`
        : `
      <div class="container-grid">
        ${repos
          .map(
            (r) => `
          <div class="container-row">
            <div class="container-status ${r.is_private ? "exited" : "running"}"></div>
            <div>
              <div class="container-name"><a href="#" class="repo-link" data-id="${r.id}" style="color:var(--primary); text-decoration:none;">${esc(r.owner)}/${esc(r.name)}</a></div>
              <div class="container-image">${esc(r.url)}</div>
            </div>
            <div><span class="badge ${r.is_private ? "badge-warning" : "badge-success"}">${r.is_private ? "Private" : "Public"}</span></div>
            <div><span class="badge badge-info">${esc(r.default_branch)}</span></div>
            <div class="container-actions">
              <button class="btn btn-danger btn-sm act-delete-repo" data-id="${r.id}">Delete</button>
            </div>
          </div>
        `,
          )
          .join("")}
      </div>`
    }`;

  el.querySelectorAll(".act-delete-repo").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const id = Number((btn as HTMLElement).dataset.id);
      if (confirm("Delete this repository?")) {
        await api.deleteRepository(id);
        loadPage();
      }
    });
  });

  el.querySelectorAll(".repo-link").forEach((link) => {
    link.addEventListener("click", (e) => {
      e.preventDefault();
      currentRepoId = Number((link as HTMLElement).dataset.id);
      currentPage = "repository_detail";
      updateHash();
      render();
    });
  });

  document.getElementById("btn-add-repo")?.addEventListener("click", () => {
    showAddRepoModal();
  });
}

function showAddRepoModal() {
  const root = document.getElementById("modal-root")!;
  root.innerHTML = `
    <div class="modal-overlay" id="modal-overlay">
      <div class="modal">
        <div class="modal-header">
          <h2 class="card-title">Add Repository</h2>
          <button class="btn btn-ghost btn-sm btn-icon" id="modal-close">✕</button>
        </div>
        <div class="modal-body">
          <form id="add-repo-form" style="display:flex; flex-direction:column; gap:10px;">
            <div class="form-group">
              <label class="form-label">Name</label>
              <input class="form-input" id="repo-name" required placeholder="my-repo">
            </div>
            <div class="form-group">
              <label class="form-label">Owner</label>
              <input class="form-input" id="repo-owner" required placeholder="username">
            </div>
            <div class="form-group">
              <label class="form-label">URL</label>
              <input class="form-input" type="url" id="repo-url" required placeholder="https://github.com/user/repo">
            </div>
            <div class="form-group">
              <label class="form-label">Default Branch</label>
              <input class="form-input" id="repo-branch" value="main">
            </div>
            <div class="form-group">
              <label class="form-label">SSH Key / Password (Optional)</label>
              <textarea class="form-input" id="repo-ssh" rows="3" placeholder="Paste your private SSH key here (e.g. for git@...)"></textarea>
            </div>
            <div class="form-group" style="display:flex; align-items:center; gap:8px;">
              <input type="checkbox" id="repo-private">
              <label class="form-label" style="margin:0;">Private Repository</label>
            </div>
            <div id="add-repo-error" class="form-error" style="display:none; color:red;"></div>
            <button class="btn btn-primary" type="submit">Create</button>
          </form>
        </div>
      </div>
    </div>`;

  document.getElementById("modal-close")?.addEventListener("click", () => {
    root.innerHTML = "";
  });
  document.getElementById("modal-overlay")?.addEventListener("click", (e) => {
    if ((e.target as HTMLElement).id === "modal-overlay") root.innerHTML = "";
  });

  document
    .getElementById("add-repo-form")
    ?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const name = (document.getElementById("repo-name") as HTMLInputElement)
        .value;
      const owner = (document.getElementById("repo-owner") as HTMLInputElement)
        .value;
      const url = (document.getElementById("repo-url") as HTMLInputElement)
        .value;
      const branch = (
        document.getElementById("repo-branch") as HTMLInputElement
      ).value;
      const sshPassword = (
        document.getElementById("repo-ssh") as HTMLTextAreaElement
      ).value;
      const isPrivate = (
        document.getElementById("repo-private") as HTMLInputElement
      ).checked;

      const errorEl = document.getElementById("add-repo-error")!;
      try {
        await api.createRepository({
          name,
          owner,
          url,
          default_branch: branch,
          ssh_password: sshPassword || null,
          is_private: isPrivate,
        });
        root.innerHTML = "";
        loadPage();
      } catch (err) {
        errorEl.style.display = "block";
        errorEl.textContent =
          err instanceof Error ? err.message : "Failed to add repository";
      }
    });
}

// ── Repository Detail page ──
async function loadRepositoryDetail(el: HTMLElement) {
  if (!currentRepoId) {
    currentPage = "repositories";
    updateHash();
    render();
    return;
  }
  const repo = await api.getRepository(currentRepoId);
  const deps = await api.listDeploymentsByRepo(currentRepoId);

  let fsStatus = {
    has_git_repo: false,
    has_docker_compose: false,
    repo_path: "",
  };
  let readme = { content: "" };
  let composeFiles: { path: string; content: string }[] = [];

  try {
    fsStatus = await api.getFilesystemStatus(currentRepoId);
    if (fsStatus.has_git_repo) {
      readme = await api.getReadme(currentRepoId);
      composeFiles = await api.getComposeFiles(currentRepoId);
    }
  } catch (e) {
    console.error("Failed to load rich repo details", e);
  }

  el.innerHTML = `
    <div style="margin-bottom: 16px;">
      <button class="btn btn-ghost btn-sm" id="btn-back-repos">← Back to Repositories</button>
    </div>
    <div class="card" style="margin-bottom: 16px;">
      <div class="card-header" style="display:flex; justify-content:space-between; align-items:center;">
        <div style="display:flex; align-items:center; gap:12px;">
          <h2 class="card-title" style="margin:0">${esc(repo.owner)}/${esc(repo.name)}</h2>
          <button class="btn btn-ghost btn-sm" id="btn-edit-repo">✏️ Edit</button>
        </div>
        <span class="badge ${repo.is_private ? "badge-warning" : "badge-success"}">${repo.is_private ? "Private" : "Public"}</span>
      </div>
      <div class="card-body">
        <div style="display:grid; gap:12px; max-width:600px;">
          <div><span style="color:var(--text-muted);font-size:13px;">URL</span><br><a href="${esc(repo.url)}" target="_blank" style="color:var(--primary);text-decoration:none;">${esc(repo.url)}</a></div>
          <div><span style="color:var(--text-muted);font-size:13px;">Branch</span><br><strong>${esc(repo.default_branch)}</strong></div>
          <div><span style="color:var(--text-muted);font-size:13px;">Created</span><br>${new Date(repo.created_at).toLocaleString()}</div>
        </div>
      </div>
    </div>
    
    <div class="card" style="margin-bottom: 16px;">
      <div class="card-header"><h3 class="card-title">Repository Settings</h3></div>
      <div class="card-body">
         <div style="display:flex; flex-direction:column; gap:8px; margin-bottom: 16px;">
           <div style="display:flex; gap:12px; align-items: center;">
              <strong>Filesystem Status: </strong> 
              ${fsStatus.has_git_repo ? '<span class="badge badge-success">Cloned</span>' : '<span class="badge badge-warning">Not Cloned</span>'}
              ${fsStatus.has_docker_compose ? '<span class="badge badge-info">Docker Compose Detected</span>' : ""}
           </div>
           ${fsStatus.repo_path ? `<div style="font-size:13px; color:var(--text-muted);"><strong>Path:</strong> <code>${esc(fsStatus.repo_path)}</code></div>` : ""}
         </div>
         <div style="display:flex; gap:12px;">
           <button class="btn btn-primary" id="btn-clone-repo">${fsStatus.has_git_repo ? "Re-Clone" : "Clone Repository"}</button>
           ${fsStatus.has_git_repo ? '<button class="btn btn-ghost" style="border:1px solid var(--border-color);" id="btn-pull-repo">Git Pull</button>' : ""}
           ${fsStatus.has_git_repo ? '<button class="btn btn-ghost" style="border:1px solid var(--border-color);" id="btn-fetch-repo">Git Fetch</button>' : ""}
           ${fsStatus.has_docker_compose ? '<button class="btn btn-success" id="btn-compose-up">▶ Trigger Docker Compose</button>' : ""}
         </div>
      </div>
    </div>
    
    ${
      readme.content
        ? `
    <div class="card" style="margin-bottom: 16px;">
       <div class="card-header"><h3 class="card-title">README</h3></div>
       <div class="card-body"><pre style="white-space: pre-wrap; word-wrap: break-word; font-family: inherit; font-size: 14px; background: var(--bg-color); padding: 12px; border-radius: 4px; border: 1px solid var(--border-color);">${esc(readme.content)}</pre></div>
    </div>
    `
        : ""
    }
    
    ${composeFiles
      .map(
        (cf) => `
      <div class="card" style="margin-bottom: 16px;">
        <div class="card-header"><h3 class="card-title">${esc(cf.path)}</h3></div>
        <div class="card-body"><pre style="font-size: 13px; background: var(--bg-color); padding: 12px; border-radius: 4px; overflow-x: auto; border: 1px solid var(--border-color);">${esc(cf.content)}</pre></div>
      </div>
    `,
      )
      .join("")}

    <div class="card">
      <div class="card-header"><h3 class="card-title">Deployments</h3></div>
      <div class="card-body" style="padding:0;">
        ${
          deps.length === 0
            ? `<div style="padding:16px; text-align:center; color:var(--text-muted);">No deployments found for this repository.</div>`
            : `
          <div class="container-grid" style="padding:16px;">
            ${deps
              .map(
                (d) => `
              <div class="container-row">
                <div class="container-status ${d.status === "success" ? "running" : d.status === "failed" ? "exited" : "created"}"></div>
                <div>
                  <div class="container-name">Deployment #${d.id}</div>
                  <div class="container-image">${d.commit_sha ? esc(d.commit_sha.slice(0, 7)) : "N/A"} — ${esc(d.created_at)}</div>
                </div>
                <div><span class="container-state-badge ${d.status === "success" ? "running" : d.status === "failed" ? "exited" : "created"}">${esc(d.status)}</span></div>
                <div class="container-actions">
                  <button class="btn btn-ghost btn-sm act-redeploy" data-id="${d.id}">↻ Redeploy</button>
                </div>
              </div>`,
              )
              .join("")}
          </div>`
        }
      </div>
    </div>
  `;

  document.getElementById("btn-back-repos")?.addEventListener("click", () => {
    currentRepoId = null;
    currentPage = "repositories";
    updateHash();
    render();
  });

  document.getElementById("btn-edit-repo")?.addEventListener("click", () => {
    showEditRepoModal(repo);
  });

  document
    .getElementById("btn-clone-repo")
    ?.addEventListener("click", async (e) => {
      const btn = e.target as HTMLButtonElement;
      btn.textContent = "Cloning...";
      btn.disabled = true;
      try {
        await api.cloneRepository(currentRepoId!);
        loadPage();
      } catch (err) {
        alert(err instanceof Error ? err.message : "Clone failed");
        btn.textContent = "Clone Repository";
        btn.disabled = false;
      }
    });

  document
    .getElementById("btn-pull-repo")
    ?.addEventListener("click", async (e) => {
      const btn = e.target as HTMLButtonElement;
      btn.textContent = "Pulling...";
      btn.disabled = true;
      try {
        const res = await api.gitPull(currentRepoId!);
        alert(res.message);
        loadPage();
      } catch (err) {
        alert(err instanceof Error ? err.message : "Pull failed");
        btn.textContent = "Git Pull";
        btn.disabled = false;
      }
    });

  document
    .getElementById("btn-fetch-repo")
    ?.addEventListener("click", async (e) => {
      const btn = e.target as HTMLButtonElement;
      btn.textContent = "Fetching...";
      btn.disabled = true;
      try {
        const res = await api.gitFetch(currentRepoId!);
        alert(res.message);
        loadPage();
      } catch (err) {
        alert(err instanceof Error ? err.message : "Fetch failed");
        btn.textContent = "Git Fetch";
        btn.disabled = false;
      }
    });

  document
    .getElementById("btn-compose-up")
    ?.addEventListener("click", async (e) => {
      const btn = e.target as HTMLButtonElement;
      btn.textContent = "Starting...";
      btn.disabled = true;
      try {
        await api.dockerComposeUp(currentRepoId!);
        loadPage();
      } catch (err) {
        alert(err instanceof Error ? err.message : "Deployment failed");
        btn.textContent = "▶ Trigger Docker Compose";
        btn.disabled = false;
      }
    });

  el.querySelectorAll(".act-redeploy").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const id = Number((btn as HTMLElement).dataset.id);
      await api.redeploy(id);
      loadPage();
    });
  });
}

// ── Deployments page ──
async function loadDeployments(el: HTMLElement) {
  const deps = await api.listDeployments();
  el.innerHTML = `
    <div class="stats-grid">
      <div class="stat-card">
        <div class="stat-icon green">🚀</div>
        <div class="stat-value">${deps.length}</div>
        <div class="stat-label">Total Deployments</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon blue">✓</div>
        <div class="stat-value">${deps.filter((d) => d.status === "success").length}</div>
        <div class="stat-label">Successful</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon yellow">⏳</div>
        <div class="stat-value">${deps.filter((d) => d.status === "pending" || d.status === "building").length}</div>
        <div class="stat-label">In Progress</div>
      </div>
    </div>
    ${
      deps.length === 0
        ? `
      <div class="empty-state">
        <div class="empty-icon">🚀</div>
        <h3>No deployments yet</h3>
        <p>Push to a connected repository to trigger a deployment.</p>
      </div>`
        : `
      <div class="container-grid">
        ${deps
          .map(
            (d) => `
          <div class="container-row">
            <div class="container-status ${d.status === "success" ? "running" : d.status === "failed" ? "exited" : "created"}"></div>
            <div>
              <div class="container-name">Deployment #${d.id}</div>
              <div class="container-image">${d.commit_sha ? esc(d.commit_sha.slice(0, 7)) : "N/A"} — ${esc(d.created_at)}</div>
            </div>
            <div><span class="container-state-badge ${d.status === "success" ? "running" : d.status === "failed" ? "exited" : "created"}">${esc(d.status)}</span></div>
            <div>${d.domain ? `<span class="badge badge-info">${esc(d.domain)}</span>` : ""}</div>
            <div class="container-actions">
              <button class="btn btn-ghost btn-sm act-redeploy" data-id="${d.id}">↻ Redeploy</button>
            </div>
          </div>
        `,
          )
          .join("")}
      </div>`
    }`;

  el.querySelectorAll(".act-redeploy").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const id = Number((btn as HTMLElement).dataset.id);
      await api.redeploy(id);
      loadPage();
    });
  });
}

// ── Health page ──
async function loadHealth(el: HTMLElement) {
  el.innerHTML = '<div class="spinner"></div>';

  let m: ServerMetrics;
  try {
    m = await api.metrics();
  } catch (err) {
    el.innerHTML = `<div class="empty-state"><div class="empty-icon">⚠️</div><h3>Could not load metrics</h3><p>${err instanceof Error ? err.message : "Unknown error"}</p></div>`;
    return;
  }

  const cpuPct = Math.round(m.cpu_usage_pct);
  const memPct =
    m.mem_total_bytes > 0
      ? Math.round((m.mem_used_bytes / m.mem_total_bytes) * 100)
      : 0;
  const diskPct =
    m.disk_total_bytes > 0
      ? Math.round((m.disk_used_bytes / m.disk_total_bytes) * 100)
      : 0;

  const checkedAt = m.checked_at
    ? new Date(m.checked_at).toLocaleString()
    : "—";

  el.innerHTML = `
    <div class="stats-grid">
      <div class="stat-card">
        <div class="stat-icon ${cpuPct > 85 ? "yellow" : "blue"}">🖥</div>
        <div class="stat-value">${cpuPct}%</div>
        <div class="stat-label">CPU Usage</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon ${memPct > 85 ? "yellow" : "green"}">🧠</div>
        <div class="stat-value">${fmtBytes(m.mem_used_bytes)}</div>
        <div class="stat-label">RAM Used / ${fmtBytes(m.mem_total_bytes)}</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon ${diskPct > 85 ? "yellow" : "purple"}">💾</div>
        <div class="stat-value">${fmtBytes(m.disk_used_bytes)}</div>
        <div class="stat-label">Disk Used / ${fmtBytes(m.disk_total_bytes)}</div>
      </div>
      <div class="stat-card">
        <div class="stat-icon ${m.docker_ok ? "green" : "yellow"}">🐋</div>
        <div class="stat-value" style="font-size:1.1rem;">${m.docker_ok ? "Connected" : "Disconnected"}</div>
        <div class="stat-label">Docker</div>
      </div>
    </div>

    <div class="card">
      <div class="card-header"><h2 class="card-title">Resource Usage</h2></div>
      <div class="card-body" style="display:flex; flex-direction:column; gap:20px; max-width:600px;">
        ${meter("CPU", cpuPct, `${cpuPct}%`)}
        ${meter("Memory", memPct, `${fmtBytes(m.mem_used_bytes)} / ${fmtBytes(m.mem_total_bytes)}`)}
        ${meter("Disk", diskPct, `${fmtBytes(m.disk_used_bytes)} / ${fmtBytes(m.disk_total_bytes)}`)}
        <div style="font-size:12px; color:var(--text-muted); margin-top:4px;">
          Last updated: ${checkedAt} &nbsp;·&nbsp; refreshes every 30 s
        </div>
      </div>
    </div>`;
}

function meter(label: string, pct: number, detail: string): string {
  const color = pct > 85 ? "#f59e0b" : pct > 95 ? "#ef4444" : "var(--primary)";
  return `
    <div>
      <div style="display:flex; justify-content:space-between; margin-bottom:6px;">
        <span style="font-size:14px; font-weight:500;">${label}</span>
        <span style="font-size:13px; color:var(--text-muted);">${detail}</span>
      </div>
      <div style="background:var(--border-color); border-radius:6px; height:10px; overflow:hidden;">
        <div style="width:${pct}%; height:100%; background:${color}; border-radius:6px; transition:width 0.4s;"></div>
      </div>
    </div>`;
}

function fmtBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`;
}

// ── Settings page ──
function loadSettings(el: HTMLElement) {
  el.innerHTML = `
    <div class="card">
      <div class="card-header"><h2 class="card-title">Server Info</h2></div>
      <div class="card-body" id="settings-body"><div class="spinner"></div></div>
    </div>`;

  api.health().then((info) => {
    document.getElementById("settings-body")!.innerHTML = `
      <div style="display:grid; gap:12px; max-width:400px;">
        <div><span style="color:var(--text-muted);font-size:13px;">Version</span><br><strong>${esc(info.version)}</strong></div>
        <div><span style="color:var(--text-muted);font-size:13px;">Docker</span><br><span class="badge ${info.docker === "connected" ? "badge-success" : "badge-danger"}">${esc(info.docker)}</span></div>
        <div><span style="color:var(--text-muted);font-size:13px;">Status</span><br><span class="badge badge-success">${esc(info.status)}</span></div>
      </div>`;
  });
}

// ── Log modal ──
function showLogModal(name: string, logs: string) {
  const root = document.getElementById("modal-root")!;
  root.innerHTML = `
    <div class="modal-overlay" id="modal-overlay">
      <div class="modal">
        <div class="modal-header">
          <h2 class="card-title">Logs — ${esc(name)}</h2>
          <button class="btn btn-ghost btn-sm btn-icon" id="modal-close">✕</button>
        </div>
        <div class="modal-body">
          <div class="log-output">${esc(logs) || "No logs available."}</div>
        </div>
      </div>
    </div>`;

  document.getElementById("modal-close")?.addEventListener("click", () => {
    root.innerHTML = "";
  });
  document.getElementById("modal-overlay")?.addEventListener("click", (e) => {
    if ((e.target as HTMLElement).id === "modal-overlay") root.innerHTML = "";
  });
}

function showEditRepoModal(repo: any) {
  const root = document.getElementById("modal-root")!;
  root.innerHTML = `
    <div class="modal-overlay" id="modal-overlay">
      <div class="modal">
        <div class="modal-header">
          <h2 class="card-title">Edit Repository</h2>
          <button class="btn btn-ghost btn-sm btn-icon" id="modal-close">✕</button>
        </div>
        <div class="modal-body">
          <form id="edit-repo-form" style="display:flex; flex-direction:column; gap:10px;">
            <div class="form-group">
              <label class="form-label">Name</label>
              <input class="form-input" id="edit-repo-name" required value="${esc(repo.name)}">
            </div>
            <div class="form-group">
              <label class="form-label">Owner</label>
              <input class="form-input" id="edit-repo-owner" required value="${esc(repo.owner)}">
            </div>
            <div class="form-group">
              <label class="form-label">URL</label>
              <input class="form-input" type="url" id="edit-repo-url" required value="${esc(repo.url)}">
            </div>
            <div class="form-group">
              <label class="form-label">Default Branch</label>
              <input class="form-input" id="edit-repo-branch" value="${esc(repo.default_branch)}">
            </div>
            <div class="form-group">
              <label class="form-label">SSH Key / Password (Optional)</label>
              <textarea class="form-input" id="edit-repo-ssh" rows="3" placeholder="Paste your private SSH key here">${esc(repo.ssh_password || "")}</textarea>
            </div>
            <div class="form-group" style="display:flex; align-items:center; gap:8px;">
              <input type="checkbox" id="edit-repo-private" ${repo.is_private ? "checked" : ""}>
              <label class="form-label" style="margin:0;">Private Repository</label>
            </div>
            <div id="edit-repo-error" class="form-error" style="display:none; color:red;"></div>
            <button class="btn btn-primary" type="submit">Save Changes</button>
          </form>
        </div>
      </div>
    </div>`;

  document.getElementById("modal-close")?.addEventListener("click", () => {
    root.innerHTML = "";
  });
  document.getElementById("modal-overlay")?.addEventListener("click", (e) => {
    if ((e.target as HTMLElement).id === "modal-overlay") root.innerHTML = "";
  });

  document
    .getElementById("edit-repo-form")
    ?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const name = (
        document.getElementById("edit-repo-name") as HTMLInputElement
      ).value;
      const owner = (
        document.getElementById("edit-repo-owner") as HTMLInputElement
      ).value;
      const url = (document.getElementById("edit-repo-url") as HTMLInputElement)
        .value;
      const branch = (
        document.getElementById("edit-repo-branch") as HTMLInputElement
      ).value;
      const sshPassword = (
        document.getElementById("edit-repo-ssh") as HTMLTextAreaElement
      ).value;
      const isPrivate = (
        document.getElementById("edit-repo-private") as HTMLInputElement
      ).checked;

      const errorEl = document.getElementById("edit-repo-error")!;
      try {
        await api.updateRepository(repo.id, {
          name,
          owner,
          url,
          default_branch: branch,
          ssh_password: sshPassword || null,
          is_private: isPrivate,
        });
        root.innerHTML = "";
        loadPage();
      } catch (err) {
        errorEl.style.display = "block";
        errorEl.textContent =
          err instanceof Error ? err.message : "Failed to update repository";
      }
    });
}

// ── Utils ──
function esc(str: string): string {
  const el = document.createElement("span");
  el.textContent = str;
  return el.innerHTML;
}

// ── Routing ──
function parseHash() {
  const hash = window.location.hash.slice(1);
  if (hash.startsWith("repository/")) {
    const id = parseInt(hash.split("/")[1]);
    if (!isNaN(id)) {
      currentPage = "repository_detail";
      currentRepoId = id;
    }
  } else if (
    [
      "containers",
      "repositories",
      "deployments",
      "health",
      "settings",
    ].includes(hash)
  ) {
    currentPage = hash;
    currentRepoId = null;
  }
}

function updateHash() {
  const targetHash =
    currentPage === "repository_detail" && currentRepoId
      ? `#repository/${currentRepoId}`
      : `#${currentPage}`;
  if (window.location.hash !== targetHash) {
    window.location.hash = targetHash;
  }
}

window.addEventListener("hashchange", () => {
  parseHash();
  render();
});

// ── Boot ──
parseHash();
updateHash();
render();
