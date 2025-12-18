import type {
  Container,
  Repository,
  CreateRepositoryInput,
  UpdateRepositoryInput,
} from "../types";

const API_BASE = "/api";

// Container API
export const containerApi = {
  getAll: async (): Promise<Container[]> => {
    const response = await fetch(`${API_BASE}/containers`);
    if (!response.ok) throw new Error("Failed to fetch containers");
    return response.json();
  },

  start: async (containerId: string): Promise<void> => {
    const response = await fetch(
      `${API_BASE}/containers/${containerId}/start`,
      {
        method: "POST",
      }
    );
    if (!response.ok) throw new Error("Failed to start container");
  },

  stop: async (containerId: string): Promise<void> => {
    const response = await fetch(`${API_BASE}/containers/${containerId}/stop`, {
      method: "POST",
    });
    if (!response.ok) throw new Error("Failed to stop container");
  },

  startProject: async (projectName: string): Promise<void> => {
    const response = await fetch(`${API_BASE}/projects/${projectName}/start`, {
      method: "POST",
    });
    if (!response.ok) throw new Error("Failed to start project");
  },

  stopProject: async (projectName: string): Promise<void> => {
    const response = await fetch(`${API_BASE}/projects/${projectName}/stop`, {
      method: "POST",
    });
    if (!response.ok) throw new Error("Failed to stop project");
  },
  restartProject: async (
    projectName: string
  ): Promise<{ stopped: number; started: number; errors?: string[] }> => {
    const response = await fetch(
      `${API_BASE}/projects/${projectName}/restart`,
      { method: "POST" }
    );
    if (!response.ok) throw new Error("Failed to restart project");
    return response.json();
  },

  rebuildProject: async (
    projectName: string,
    path?: string
  ): Promise<{ returncode: number; stdout: string; stderr: string }> => {
    const response = await fetch(
      `${API_BASE}/projects/${projectName}/rebuild`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path }),
      }
    );
    if (!response.ok) throw new Error("Failed to rebuild project");
    return response.json();
  },
};

// Repository API
export const repositoryApi = {
  getAll: async (): Promise<Repository[]> => {
    const response = await fetch(`${API_BASE}/repositories`);
    if (!response.ok) throw new Error("Failed to fetch repositories");
    return response.json();
  },

  getById: async (id: number): Promise<Repository> => {
    const response = await fetch(`${API_BASE}/repositories/${id}`);
    if (!response.ok) throw new Error("Failed to fetch repository");
    return response.json();
  },

  create: async (data: CreateRepositoryInput): Promise<Repository> => {
    const response = await fetch(`${API_BASE}/repositories`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(data),
    });
    if (!response.ok) throw new Error("Failed to create repository");
    return response.json();
  },

  update: async (
    id: number,
    data: UpdateRepositoryInput
  ): Promise<Repository> => {
    const response = await fetch(`${API_BASE}/repositories/${id}`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(data),
    });
    if (!response.ok) throw new Error("Failed to update repository");
    return response.json();
  },

  delete: async (id: number): Promise<void> => {
    const response = await fetch(`${API_BASE}/repositories/${id}`, {
      method: "DELETE",
    });
    if (!response.ok) throw new Error("Failed to delete repository");
  },

  getFilesystemStatus: async (
    id: number
  ): Promise<{ has_git_repo: boolean; has_docker_compose: boolean }> => {
    const response = await fetch(
      `${API_BASE}/repositories/${id}/filesystem-status`
    );
    if (!response.ok) throw new Error("Failed to get filesystem status");
    return response.json();
  },
  getComposeFile: async (
    id: number
  ): Promise<{ path: string; content: string }> => {
    const response = await fetch(`${API_BASE}/repositories/${id}/compose-file`);
    if (!response.ok) throw new Error("Failed to get compose file");
    return response.json();
  },
  // Git operations
  gitStatus: async (
    id: number
  ): Promise<{ returncode: number; stdout: string; stderr: string }> => {
    const response = await fetch(`${API_BASE}/repositories/${id}/git/status`);
    if (!response.ok) throw new Error("Failed to get git status");
    return response.json();
  },

  gitFetch: async (
    id: number
  ): Promise<{ returncode: number; stdout: string; stderr: string }> => {
    const response = await fetch(`${API_BASE}/repositories/${id}/git/fetch`, {
      method: "POST",
    });
    if (!response.ok) throw new Error("Failed to run git fetch");
    return response.json();
  },

  gitPull: async (
    id: number,
    opts?: { remote?: string; branch?: string }
  ): Promise<{ returncode: number; stdout: string; stderr: string }> => {
    const response = await fetch(`${API_BASE}/repositories/${id}/git/pull`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(opts || {}),
    });
    if (!response.ok) throw new Error("Failed to run git pull");
    return response.json();
  },
};

// README API
export const readmeApi = {
  post: async (id: number, path?: string): Promise<{ content: string }> => {
    const response = await fetch(`${API_BASE}/repositories/${id}/readme`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path }),
    });
    if (!response.ok) throw new Error("Failed to fetch README");
    return response.json();
  },
};
