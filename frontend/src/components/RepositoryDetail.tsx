import { useParams, useNavigate, Link } from "react-router-dom";
import {
  useRepository,
  useUpdateRepository,
  useDeleteRepository,
  useRepositoryFilesystemStatus,
  useStartProject,
  useStopProject,
  useRestartProject,
  useRebuildProject,
  useContainers,
  useRepositoryComposeFile,
} from "@/hooks/useApi";
import { UpdateRepositoryInput } from "@/types";
import { useState, useEffect } from "react";
import GitControls from "./GitControls";
import ReadmeSection from "./ReadmeSection";

const RepositoryDetail = () => {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const repoId = id ? parseInt(id, 10) : 0;

  const { data: repository, isLoading, error } = useRepository(repoId);
  const updateRepository = useUpdateRepository();
  const deleteRepository = useDeleteRepository();
  const { data: filesystemStatus } = useRepositoryFilesystemStatus(repoId);
  const startProject = useStartProject();
  const stopProject = useStopProject();
  const restartProject = useRestartProject();
  const rebuildProject = useRebuildProject();
  const { data: containers } = useContainers();
  const {
    data: composeFile,
    isLoading: composeLoading,
    error: composeError,
  } = useRepositoryComposeFile(repoId);

  const [isEditing, setIsEditing] = useState(false);
  const [editData, setEditData] = useState<UpdateRepositoryInput>({});

  const handleEdit = () => {
    if (repository) {
      setEditData({
        name: repository.name,
        owner: repository.owner,
        url: repository.url,
        description: repository.description,
        webhook_url: repository.webhook_url,
        filesystem_path: repository.filesystem_path,
        is_private: repository.is_private,
        default_branch: repository.default_branch,
      });
      setIsEditing(true);
    }
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      await updateRepository.mutateAsync({ id: repoId, data: editData });
      setIsEditing(false);
    } catch (error) {
      console.error("Failed to update repository:", error);
    }
  };

  const handleDelete = async () => {
    if (confirm("Are you sure you want to delete this repository?")) {
      try {
        await deleteRepository.mutateAsync(repoId);
        navigate("/");
      } catch (error) {
        console.error("Failed to delete repository:", error);
      }
    }
  };

  const handleStartProject = async () => {
    if (!repository?.filesystem_path) return;
    const projectName =
      repository.filesystem_path.split("/").pop() || repository.name;
    try {
      await startProject.mutateAsync(projectName);
    } catch (error) {
      console.error("Failed to start project:", error);
    }
  };

  const handleRestartProject = async () => {
    if (!repository?.filesystem_path) return;
    const projectName =
      repository.filesystem_path.split("/").pop() || repository.name;
    try {
      await restartProject.mutateAsync(projectName);
    } catch (error) {
      console.error("Failed to restart project:", error);
    }
  };

  const handleRebuildProject = async () => {
    if (!repository?.filesystem_path) return;
    const projectName =
      repository.filesystem_path.split("/").pop() || repository.name;
    try {
      await rebuildProject.mutateAsync({
        projectName,
        path: repository.filesystem_path,
      });
    } catch (error) {
      console.error("Failed to rebuild project:", error);
    }
  };

  const handleStopProject = async () => {
    if (!repository?.filesystem_path) return;
    const projectName =
      repository.filesystem_path.split("/").pop() || repository.name;
    try {
      await stopProject.mutateAsync(projectName);
    } catch (error) {
      console.error("Failed to stop project:", error);
    }
  };

  const formatDate = (s?: string) => (s ? new Date(s).toLocaleString() : "-");

  if (isLoading) {
    return (
      <div className="flex justify-center items-center min-h-screen">
        <span className="loading loading-spinner loading-lg"></span>
      </div>
    );
  }

  if (error || !repository) {
    return (
      <div className="card bg-base-100 shadow-xl">
        <div className="card-body">
          <div className="alert alert-error">
            <span>Failed to load repository details</span>
          </div>
          <Link to="/" className="btn btn-primary">
            ← Back to Dashboard
          </Link>
        </div>
      </div>
    );
  }

  const projectName =
    repository.filesystem_path?.split("/")?.pop() || repository.name;
  const projectContainers =
    containers?.filter((c) => c.compose_project === projectName) || [];
  const projectIsRunning = projectContainers.some(
    (c) => c.status === "running"
  );

  return (
    <div className="space-y-4">
      {/* Breadcrumb */}
      <div className="text-sm breadcrumbs">
        <ul>
          <li>
            <Link to="/">Dashboard</Link>
          </li>
          <li>Repository</li>
          <li>{repository.name}</li>
        </ul>
      </div>

      {/* Repository Header */}
      <div className="card bg-base-100 shadow-xl gap-5 p-8">
        <div className="card-body">
          <div className="flex justify-between items-start">
            <div>
              <h1 className="card-title text-3xl mb-2">
                {repository.owner}/{repository.name}
              </h1>
              {repository.description && (
                <p className="text-base-content/70">{repository.description}</p>
              )}
            </div>
            <div className="flex gap-2 flex-wrap">
              {!isEditing && (
                <>
                  <button
                    onClick={handleEdit}
                    className="btn btn-primary btn-sm"
                  >
                    Edit
                  </button>
                  <button
                    onClick={handleDelete}
                    className="btn btn-error btn-sm"
                  >
                    Delete
                  </button>
                </>
              )}
            </div>
          </div>
        </div>

        <div className="divider"></div>

        {isEditing ? (
          <form onSubmit={handleSave} className="space-y-4">
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div className="form-control">
                <label className="label">
                  <span className="label-text">Repository Name *</span>
                </label>
                <input
                  type="text"
                  title="edit-data-name"
                  arial-label="edit-data-name"
                  className="input input-bordered"
                  value={editData.name || ""}
                  onChange={(e) =>
                    setEditData({ ...editData, name: e.target.value })
                  }
                  required
                />
              </div>

              <div className="form-control">
                <label className="label">
                  <span className="label-text">Owner *</span>
                </label>
                <input
                  aria-label="edit-data-owner"
                  type="text"
                  className="input input-bordered"
                  value={editData.owner || ""}
                  onChange={(e) =>
                    setEditData({ ...editData, owner: e.target.value })
                  }
                  required
                />
              </div>

              <div className="form-control md:col-span-2">
                <label className="label">
                  <span className="label-text">URL *</span>
                </label>
                <input
                  type="url"
                  title="url-label"
                  className="input input-bordered"
                  value={editData.url || ""}
                  onChange={(e) =>
                    setEditData({ ...editData, url: e.target.value })
                  }
                  required
                />
              </div>

              <div className="form-control md:col-span-2">
                <label className="label">
                  <span className="label-text">Description</span>
                </label>
                <textarea
                  className="textarea textarea-bordered"
                  title="description"
                  value={editData.description || ""}
                  onChange={(e) =>
                    setEditData({ ...editData, description: e.target.value })
                  }
                  rows={3}
                />
              </div>

              <div className="form-control md:col-span-2">
                <label className="label">
                  <span className="label-text">Filesystem Path</span>
                </label>
                <input
                  type="text"
                  title="filesystem-path"
                  className="input input-bordered"
                  value={editData.filesystem_path || ""}
                  onChange={(e) =>
                    setEditData({
                      ...editData,
                      filesystem_path: e.target.value,
                    })
                  }
                />
              </div>

              <div className="form-control">
                <label className="label">
                  <span className="label-text">Default Branch</span>
                </label>
                <input
                  title="default-branch"
                  type="text"
                  className="input input-bordered"
                  value={editData.default_branch || ""}
                  onChange={(e) =>
                    setEditData({
                      ...editData,
                      default_branch: e.target.value,
                    })
                  }
                />
              </div>

              <div className="form-control">
                <label className="label cursor-pointer">
                  <span className="label-text">Private Repository</span>
                  <input
                    type="checkbox"
                    className="checkbox checkbox-primary"
                    checked={editData.is_private || false}
                    onChange={(e) =>
                      setEditData({
                        ...editData,
                        is_private: e.target.checked,
                      })
                    }
                  />
                </label>
              </div>

              <div className="form-control md:col-span-2">
                <label className="label">
                  <span className="label-text">Webhook URL</span>
                </label>
                <input
                  title="edit-data-webhook"
                  type="url"
                  className="input input-bordered"
                  value={editData.webhook_url || ""}
                  onChange={(e) =>
                    setEditData({ ...editData, webhook_url: e.target.value })
                  }
                />
              </div>
            </div>

            <div className="flex gap-2 justify-end">
              <button
                type="button"
                onClick={() => setIsEditing(false)}
                className="btn btn-ghost"
              >
                Cancel
              </button>
              <button
                type="submit"
                className="btn btn-primary"
                disabled={updateRepository.isPending}
              >
                {updateRepository.isPending ? (
                  <span className="loading loading-spinner"></span>
                ) : (
                  "Save Changes"
                )}
              </button>
            </div>
          </form>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
            <div>
              <h3 className="font-semibold text-sm text-base-content/60 mb-1">
                Repository URL
              </h3>
              <a
                href={repository.url}
                target="_blank"
                rel="noopener noreferrer"
                className="link link-primary"
              >
                {repository.url}
              </a>
            </div>

            <div>
              <h3 className="font-semibold text-sm text-base-content/60 mb-1">
                Owner
              </h3>
              <p>{repository.owner}</p>
            </div>

            <div>
              <h3 className="font-semibold text-sm text-base-content/60 mb-1">
                Default Branch
              </h3>
              <p>{repository.default_branch}</p>
            </div>

            {repository.filesystem_path && (
              <div>
                <h3 className="font-semibold text-sm text-base-content/60 mb-1">
                  Filesystem Path
                </h3>
                <p className="font-mono text-sm">
                  {repository.filesystem_path}
                </p>
                {filesystemStatus && (
                  <div className="mt-2 flex gap-2">
                    <div
                      className={`badge ${
                        filesystemStatus.has_git_repo
                          ? "badge-success"
                          : "badge-error"
                      }`}
                    >
                      {filesystemStatus.has_git_repo
                        ? "Git Repo"
                        : "No Git Repo"}
                    </div>
                    <div
                      className={`badge ${
                        filesystemStatus.has_docker_compose
                          ? "badge-success"
                          : "badge-neutral"
                      }`}
                    >
                      {filesystemStatus.has_docker_compose
                        ? "Docker Compose"
                        : "No Docker Compose"}
                    </div>
                  </div>
                )}
              </div>
            )}

            <div>
              <h3 className="font-semibold text-sm text-base-content/60 mb-1">
                Visibility
              </h3>
              <div
                className={`badge ${
                  repository.is_private ? "badge-error" : "badge-success"
                }`}
              >
                {repository.is_private ? "Private" : "Public"}
              </div>
            </div>

            {repository.webhook_url && (
              <div>
                <h3 className="font-semibold text-sm text-base-content/60 mb-1">
                  Webhook URL
                </h3>
                <a
                  href={repository.webhook_url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="link link-primary"
                >
                  {repository.webhook_url}
                </a>
              </div>
            )}

            <div>
              <h3 className="font-semibold text-sm text-base-content/60 mb-1">
                Created
              </h3>
              <p>{formatDate(repository.created_at)}</p>
            </div>

            <div>
              <h3 className="font-semibold text-sm text-base-content/60 mb-1">
                Updated
              </h3>
              <p>{formatDate(repository.updated_at)}</p>
            </div>

            <div>
              <h3 className="font-semibold text-sm text-base-content/60 mb-1">
                ID
              </h3>
              <p>{repository.id}</p>
            </div>
          </div>
        )}
      </div>

      {filesystemStatus?.has_docker_compose && (
        <div className="card bg-base-100 shadow-md">
          <div className="card-body">
            <div className="flex justify-between items-center">
              <h2 className="card-title">Docker Compose</h2>
              <div className="flex gap-2">
                {projectIsRunning ? (
                  <button
                    onClick={handleStopProject}
                    className="btn btn-warning btn-sm"
                    disabled={stopProject.isPending}
                  >
                    {stopProject.isPending ? (
                      <span className="loading loading-spinner loading-sm"></span>
                    ) : (
                      "Stop Project"
                    )}
                  </button>
                ) : (
                  <button
                    onClick={handleStartProject}
                    className="btn btn-success btn-sm"
                    disabled={startProject.isPending}
                  >
                    {startProject.isPending ? (
                      <span className="loading loading-spinner loading-sm"></span>
                    ) : (
                      "Start Project"
                    )}
                  </button>
                )}
                <button
                  onClick={handleRestartProject}
                  className="btn btn-sm"
                  disabled={restartProject.isPending}
                >
                  {restartProject.isPending ? (
                    <span className="loading loading-spinner loading-sm"></span>
                  ) : (
                    "Restart"
                  )}
                </button>
                <button
                  onClick={handleRebuildProject}
                  className="btn btn-sm btn-outline"
                  disabled={rebuildProject.isPending}
                >
                  {rebuildProject.isPending ? (
                    <span className="loading loading-spinner loading-sm"></span>
                  ) : (
                    "Rebuild"
                  )}
                </button>
              </div>
            </div>

            <p className="text-sm text-base-content/70 mt-2">
              Project: <span className="font-mono">{projectName}</span>
            </p>

            <div className="mt-4">
              <h3 className="font-semibold text-sm mb-2">Containers</h3>
              {projectContainers.length > 0 ? (
                <div className="grid gap-2">
                  {projectContainers.map((c) => (
                    <div
                      key={c.id}
                      className="flex items-center justify-between p-2 border rounded"
                    >
                      <div>
                        <div className="font-semibold">{c.name}</div>
                        <div className="text-xs text-base-content/60">
                          {c.image}
                        </div>
                      </div>
                      <div className="flex items-center gap-2">
                        <div
                          className={`badge ${
                            c.status === "running"
                              ? "badge-success"
                              : c.status === "exited"
                              ? "badge-error"
                              : "badge-warning"
                          }`}
                        >
                          {c.status}
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <p className="text-sm text-base-content/60">
                  No containers found for this project.
                </p>
              )}
            </div>

            <div className="mt-6">
              <h3 className="font-semibold text-sm mb-2">Compose File</h3>
              {composeLoading ? (
                <div className="text-sm">Loading compose file...</div>
              ) : composeError ? (
                <div className="text-sm text-error">
                  Failed to load compose file
                </div>
              ) : composeFile ? (
                <div>
                  <p className="text-xs text-base-content/60 mb-2">
                    Path: <span className="font-mono">{composeFile.path}</span>
                  </p>
                  <div className="mb-3">
                    <h4 className="font-medium text-sm mb-1">Services</h4>
                    {(() => {
                      const services: string[] = [];
                      if (composeFile.content) {
                        const re = /^[ \t]{2,}([a-zA-Z0-9_\-]+):/gm;
                        let m;
                        while ((m = re.exec(composeFile.content)) !== null) {
                          if (m[1] && !services.includes(m[1]))
                            services.push(m[1]);
                        }
                      }
                      return services.length > 0 ? (
                        <div className="flex flex-wrap gap-2">
                          {services.map((s) => (
                            <div key={s} className="badge badge-outline">
                              {s}
                            </div>
                          ))}
                        </div>
                      ) : (
                        <p className="text-sm text-base-content/60">
                          No services detected in compose file.
                        </p>
                      );
                    })()}
                  </div>

                  <pre className="bg-base-200 p-3 rounded text-sm overflow-x-auto">
                    {composeFile.content}
                  </pre>
                </div>
              ) : (
                <p className="text-sm text-base-content/60">
                  No compose file available.
                </p>
              )}
            </div>
          </div>
        </div>
      )}
      {filesystemStatus?.has_git_repo && (
        <div className="mt-4 space-y-4">
          <GitControls repoId={repository.id} />
          <ReadmeSection
            repositoryId={repository.id}
            filesystemPath={repository?.filesystem_path}
          />
        </div>
      )}
    </div>
  );
};

export default RepositoryDetail;
