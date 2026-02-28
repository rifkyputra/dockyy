const API_BASE = "/api";

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const token = localStorage.getItem("dockyy_token");
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...((options.headers as Record<string, string>) || {}),
  };
  if (token) headers["Authorization"] = `Bearer ${token}`;

  const res = await fetch(`${API_BASE}${path}`, { ...options, headers });

  if (res.status === 401) {
    localStorage.removeItem("dockyy_token");
    window.location.reload();
    throw new Error("Unauthorized");
  }

  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.error || `HTTP ${res.status}`);
  }

  return res.json();
}

export const api = {
  // Auth
  login: (username: string, password: string) =>
    request<{ token: string; username: string }>("/auth/login", {
      method: "POST",
      body: JSON.stringify({ username, password }),
    }),

  verify: (token: string) =>
    request<{ valid: boolean; username: string }>("/auth/verify", {
      method: "POST",
      body: JSON.stringify({ token }),
    }),

  // Health
  health: () =>
    request<{ status: string; docker: string; version: string }>("/health"),

  metrics: () => request<ServerMetrics>("/metrics"),

  // Containers
  listContainers: (all = true) =>
    request<Container[]>(`/containers?all=${all}`),

  startContainer: (id: string) =>
    request<{ status: string }>(`/containers/${id}/start`, { method: "POST" }),

  stopContainer: (id: string) =>
    request<{ status: string }>(`/containers/${id}/stop`, { method: "POST" }),

  restartContainer: (id: string) =>
    request<{ status: string }>(`/containers/${id}/restart`, {
      method: "POST",
    }),

  removeContainer: (id: string) =>
    request<{ status: string }>(`/containers/${id}`, { method: "DELETE" }),

  containerLogs: (id: string, tail = 100) =>
    request<{ logs: string }>(`/containers/${id}/logs?tail=${tail}`),

  // Repositories
  listRepositories: () => request<Repository[]>("/repositories"),
  getRepository: (id: number) => request<Repository>(`/repositories/${id}`),
  createRepository: (data: Partial<Repository>) =>
    request<{ id: number }>("/repositories", {
      method: "POST",
      body: JSON.stringify(data),
    }),
  updateRepository: (id: number, data: Partial<Repository>) =>
    request<{ message: string }>(`/repositories/${id}`, {
      method: "PUT",
      body: JSON.stringify(data),
    }),
  deleteRepository: (id: number) =>
    request<{ message: string }>(`/repositories/${id}`, { method: "DELETE" }),

  getFilesystemStatus: (id: number) =>
    request<{
      has_git_repo: boolean;
      has_docker_compose: boolean;
      repo_path: string;
    }>(`/repositories/${id}/filesystem-status`),
  getReadme: (id: number) =>
    request<{ content: string }>(`/repositories/${id}/readme`),
  getComposeFiles: (id: number) =>
    request<{ path: string; content: string }[]>(
      `/repositories/${id}/compose-files`,
    ),
  cloneRepository: (id: number) =>
    request<{ message: string }>(`/repositories/${id}/clone`, {
      method: "POST",
    }),
  gitPull: (id: number) =>
    request<{ message: string }>(`/repositories/${id}/pull`, {
      method: "POST",
    }),
  gitFetch: (id: number) =>
    request<{ message: string }>(`/repositories/${id}/fetch`, {
      method: "POST",
    }),
  dockerComposeUp: (id: number) =>
    request<{ message: string }>(`/repositories/${id}/docker-compose-up`, {
      method: "POST",
    }),

  // Env vars
  listEnvVars: (id: number) =>
    request<EnvVar[]>(`/repositories/${id}/env-vars`),
  upsertEnvVar: (id: number, key: string, value: string) =>
    request<{ id: number; message: string }>(`/repositories/${id}/env-vars`, {
      method: "POST",
      body: JSON.stringify({ key, value }),
    }),
  updateEnvVar: (repoId: number, varId: number, value: string) =>
    request<{ message: string }>(`/repositories/${repoId}/env-vars/${varId}`, {
      method: "PUT",
      body: JSON.stringify({ value }),
    }),
  deleteEnvVar: (repoId: number, varId: number) =>
    request<{ message: string }>(`/repositories/${repoId}/env-vars/${varId}`, {
      method: "DELETE",
    }),
  importEnvVarsFromCompose: (id: number, compose_file: string) =>
    request<{ message: string; keys: string[] }>(
      `/repositories/${id}/env-vars/import-from-compose`,
      {
        method: "POST",
        body: JSON.stringify({ compose_file }),
      },
    ),

  // Deployments
  listDeployments: () => request<Deployment[]>("/deployments"),
  listDeploymentsByRepo: (repoId: number) =>
    request<Deployment[]>(`/deployments/repo/${repoId}`),
  redeploy: (id: number) =>
    request<{ message: string; job_id: number }>(
      `/deployments/${id}/redeploy`,
      { method: "POST" },
    ),

  // Proxy
  proxyStatus: () =>
    request<{ traefik_running: boolean; network: string; container: string }>(
      "/proxy/status",
    ),
  proxyRoutes: () => request<ProxyRoute[]>("/proxy/routes"),
  ensureTraefik: () =>
    request<{ message: string }>("/proxy/ensure", { method: "POST" }),
};

// Types
export interface Container {
  id: string;
  name: string;
  image: string;
  status: string;
  state: string;
  ports: {
    private_port: number;
    public_port: number | null;
    port_type: string;
  }[];
  created: number;
}

export interface ProxyRoute {
  container_id: string;
  container_name: string;
  domain: string;
  port: number;
  status: string;
}

export interface EnvVar {
  id: number;
  repo_id: number;
  key: string;
  value: string;
  created_at: string;
  updated_at: string;
}

export interface Repository {
  id: number;
  name: string;
  owner: string;
  url: string;
  description: string | null;
  webhook_url: string | null;
  filesystem_path: string | null;
  ssh_password: string | null;
  is_private: boolean;
  default_branch: string;
  domain: string | null;
  proxy_port: number | null;
  created_at: string;
  updated_at: string;
}

export interface ServerMetrics {
  cpu_usage_pct: number;
  mem_used_bytes: number;
  mem_total_bytes: number;
  swap_used_bytes: number;
  swap_total_bytes: number;
  disk_used_bytes: number;
  disk_total_bytes: number;
  docker_ok: boolean;
  checked_at: string;
}

export interface Deployment {
  id: number;
  repo_id: number;
  status: string;
  commit_sha: string | null;
  image_name: string | null;
  container_id: string | null;
  domain: string | null;
  port: number | null;
  build_log: string | null;
  created_at: string;
  updated_at: string;
}
