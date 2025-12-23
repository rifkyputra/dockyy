import {
  useRepositories,
  useCreateRepository,
  useDeleteRepository,
} from "@/hooks/useApi";
import { CreateRepositoryInput } from "@/types";
import React, { useState } from "react";
import { Link } from "react-router-dom";

const RepositoriesTable = () => {
  const {
    data: repositories,
    isLoading: repositoriesLoading,
    error: repositoriesError,
  } = useRepositories();
  const createRepository = useCreateRepository();
  const deleteRepository = useDeleteRepository();

  const [showAddRepo, setShowAddRepo] = useState(false);
  const [newRepo, setNewRepo] = useState<CreateRepositoryInput>({
    name: "",
    owner: "",
    url: "",
    description: "",
    webhook_url: "",
    filesystem_path: "",
    ssh_password: "",
    is_private: false,
    default_branch: "main",
  });

  const handleCreateRepo = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      await createRepository.mutateAsync(newRepo);
      setShowAddRepo(false);
      setNewRepo({
        name: "",
        owner: "",
        url: "",
        description: "",
        webhook_url: "",
        filesystem_path: "",
        ssh_password: "",
        is_private: false,
        default_branch: "main",
      });
    } catch (error) {
      console.error("Failed to create repository:", error);
    }
  };

  const handleDeleteRepo = async (id: number) => {
    if (confirm("Are you sure you want to delete this repository?")) {
      try {
        await deleteRepository.mutateAsync(id);
      } catch (error) {
        console.error("Failed to delete repository:", error);
      }
    }
  };

  return (
    <div className="card bg-base-100 shadow-xl">
      <div className="card-body">
        <div className="flex justify-between items-center mb-4">
          <h2 className="card-title text-2xl">
            GitHub Repositories
            {repositories && (
              <div className="badge badge-secondary">{repositories.length}</div>
            )}
          </h2>
          <button
            className="btn btn-sm btn-secondary"
            onClick={() => setShowAddRepo(!showAddRepo)}
          >
            {showAddRepo ? "✕ Cancel" : "+ Add Repository"}
          </button>
        </div>

        {/* Add Repository Form */}
        {showAddRepo && (
          <div className="card bg-base-200 mb-4">
            <div className="card-body">
              <h3 className="font-bold text-lg mb-2">Add New Repository</h3>
              <form onSubmit={handleCreateRepo} className="space-y-3">
                <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                  <div className="form-control">
                    <label className="label">
                      <span className="label-text">Repository Name *</span>
                    </label>
                    <input
                      type="text"
                      placeholder="my-repo"
                      className="input input-bordered"
                      value={newRepo.name}
                      onChange={(e) =>
                        setNewRepo({ ...newRepo, name: e.target.value })
                      }
                      required
                    />
                  </div>
                  <div className="form-control">
                    <label className="label">
                      <span className="label-text">Owner *</span>
                    </label>
                    <input
                      type="text"
                      placeholder="username"
                      className="input input-bordered"
                      value={newRepo.owner}
                      onChange={(e) =>
                        setNewRepo({ ...newRepo, owner: e.target.value })
                      }
                      required
                    />
                  </div>
                </div>
                <div className="form-control">
                  <label className="label">
                    <span className="label-text">Repository URL *</span>
                  </label>
                  <input
                    type="url"
                    placeholder="https://github.com/username/my-repo"
                    className="input input-bordered"
                    value={newRepo.url}
                    onChange={(e) =>
                      setNewRepo({ ...newRepo, url: e.target.value })
                    }
                    required
                  />
                </div>
                <div className="form-control">
                  <label className="label">
                    <span className="label-text">Description</span>
                  </label>
                  <textarea
                    className="textarea textarea-bordered"
                    placeholder="Repository description"
                    value={newRepo.description}
                    onChange={(e) =>
                      setNewRepo({
                        ...newRepo,
                        description: e.target.value,
                      })
                    }
                  />
                </div>
                <div className="form-control">
                  <label className="label">
                    <span className="label-text">Webhook URL</span>
                  </label>
                  <input
                    type="url"
                    placeholder="https://example.com/webhook"
                    className="input input-bordered"
                    value={newRepo.webhook_url}
                    onChange={(e) =>
                      setNewRepo({
                        ...newRepo,
                        webhook_url: e.target.value,
                      })
                    }
                  />
                </div>
                <div className="form-control">
                  <label className="label">
                    <span className="label-text">Filesystem Path</span>
                  </label>
                  <input
                    type="text"
                    placeholder="/path/to/local/repository"
                    className="input input-bordered"
                    value={newRepo.filesystem_path}
                    onChange={(e) =>
                      setNewRepo({
                        ...newRepo,
                        filesystem_path: e.target.value,
                      })
                    }
                  />
                </div>
                <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                  <div className="form-control">
                    <label className="label">
                      <span className="label-text">Default Branch</span>
                    </label>
                    <input
                      type="text"
                      placeholder="main"
                      className="input input-bordered"
                      value={newRepo.default_branch}
                      onChange={(e) =>
                        setNewRepo({
                          ...newRepo,
                          default_branch: e.target.value,
                        })
                      }
                    />
                  </div>
                  <div className="form-control">
                    <label className="label cursor-pointer justify-start gap-2">
                      <input
                        type="checkbox"
                        className="checkbox checkbox-primary"
                        checked={newRepo.is_private}
                        onChange={(e) =>
                          setNewRepo({
                            ...newRepo,
                            is_private: e.target.checked,
                          })
                        }
                      />
                      <span className="label-text">Private Repository</span>
                    </label>
                  </div>
                </div>
                {newRepo.is_private && (
                  <div className="form-control">
                    <label className="label">
                      <span className="label-text">SSH Password</span>
                      <span className="label-text-alt text-warning">
                        For private repositories requiring SSH password
                        authentication
                      </span>
                    </label>
                    <input
                      type="password"
                      placeholder="Enter SSH password (optional)"
                      className="input input-bordered"
                      value={newRepo.ssh_password || ""}
                      onChange={(e) =>
                        setNewRepo({
                          ...newRepo,
                          ssh_password: e.target.value,
                        })
                      }
                    />
                  </div>
                )}
                <div className="card-actions justify-end">
                  <button
                    type="submit"
                    className="btn btn-primary"
                    disabled={createRepository.isPending}
                  >
                    {createRepository.isPending ? (
                      <span className="loading loading-spinner"></span>
                    ) : (
                      "Create Repository"
                    )}
                  </button>
                </div>
              </form>
            </div>
          </div>
        )}

        {repositoriesError && (
          <div className="alert alert-error">
            <svg
              xmlns="http://www.w3.org/2000/svg"
              className="stroke-current shrink-0 h-6 w-6"
              fill="none"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth="2"
                d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z"
              />
            </svg>
            <span>Error: {repositoriesError.message}</span>
          </div>
        )}

        {repositoriesLoading ? (
          <div className="flex justify-center py-8">
            <span className="loading loading-spinner loading-lg"></span>
          </div>
        ) : repositories && repositories.length > 0 ? (
          <div className="overflow-x-auto">
            <table className="table">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Owner</th>
                  <th>Path</th>
                  <th>Branch</th>
                  <th>Private</th>
                  <th>Actions</th>
                </tr>
              </thead>
              <tbody>
                {repositories.map((repo) => (
                  <tr key={repo.id} className="hover">
                    <td>
                      <Link
                        to={`/repositories/${repo.id}`}
                        className="font-bold hover:link"
                      >
                        {repo.name}
                      </Link>
                      {repo.description && (
                        <div className="text-sm opacity-50">
                          {repo.description}
                        </div>
                      )}
                    </td>
                    <td>{repo.owner}</td>

                    <td>
                      {repo.filesystem_path ? (
                        <code className="text-xs">{repo.filesystem_path}</code>
                      ) : (
                        <span className="text-base-content/40 text-xs">
                          Not set
                        </span>
                      )}
                    </td>
                    <td>
                      <code className="text-xs">{repo.default_branch}</code>
                    </td>
                    <td>
                      {repo.is_private ? (
                        <div className="badge badge-warning">Private</div>
                      ) : (
                        <div className="badge badge-success">Public</div>
                      )}
                    </td>
                    <td>
                      <div className="flex gap-1">
                        <Link
                          to={`/repositories/${repo.id}`}
                          className="btn btn-ghost btn-xs"
                        >
                          Details
                        </Link>
                        <button
                          className="btn btn-ghost btn-xs text-error"
                          onClick={() => handleDeleteRepo(repo.id)}
                          disabled={deleteRepository.isPending}
                        >
                          Delete
                        </button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <div className="text-center py-8 text-base-content/60">
            <p>No repositories found. Add your first repository!</p>
          </div>
        )}
      </div>
    </div>
  );
};

export default RepositoriesTable;
