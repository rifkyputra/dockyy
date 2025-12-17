import {
  useContainers,
  useStartContainer,
  useStopContainer,
  useStartProject,
  useStopProject,
} from "@/hooks/useApi";

const ContainersTable = () => {
  const {
    data: containers,
    isLoading: containersLoading,
    error: containersError,
    refetch: refetchContainers,
  } = useContainers();
  const startContainer = useStartContainer();
  const stopContainer = useStopContainer();
  const startProject = useStartProject();
  const stopProject = useStopProject();

  const handleStartContainer = async (id: string) => {
    try {
      await startContainer.mutateAsync(id);
    } catch (error) {
      console.error("Failed to start container:", error);
    }
  };

  const handleStopContainer = async (id: string) => {
    try {
      await stopContainer.mutateAsync(id);
    } catch (error) {
      console.error("Failed to stop container:", error);
    }
  };

  const handleStartProject = async (projectName: string) => {
    try {
      await startProject.mutateAsync(projectName);
    } catch (error) {
      console.error("Failed to start project:", error);
    }
  };

  const handleStopProject = async (projectName: string) => {
    try {
      await stopProject.mutateAsync(projectName);
    } catch (error) {
      console.error("Failed to stop project:", error);
    }
  };

  return (
    <div className="card bg-base-100 shadow-xl">
      <div className="card-body">
        <div className="flex justify-between items-center mb-4">
          <h2 className="card-title text-2xl">
            Containers
            {containers && (
              <div className="badge badge-primary">{containers.length}</div>
            )}
          </h2>
          <button
            className="btn btn-sm btn-primary"
            onClick={() => refetchContainers()}
            disabled={containersLoading}
          >
            {containersLoading ? (
              <span className="loading loading-spinner loading-sm"></span>
            ) : (
              "🔄 Refresh"
            )}
          </button>
        </div>

        {containersError && (
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
            <span>Error: {containersError.message}</span>
          </div>
        )}

        {containersLoading ? (
          <div className="flex justify-center py-8">
            <span className="loading loading-spinner loading-lg"></span>
          </div>
        ) : containers && containers.length > 0 ? (
          <div className="space-y-4">
            {/* Group containers by compose project */}
            {(() => {
              // Group containersx
              const grouped = containers.reduce((acc, container) => {
                const project = container.compose_project || "standalone";
                if (!acc[project]) acc[project] = [];
                acc[project].push(container);
                return acc;
              }, {} as Record<string, typeof containers>);

              return Object.entries(grouped).map(
                ([project, projectContainers]) => (
                  <div key={project} className="bg-base-200 rounded-lg">
                    {/* Header with buttons */}
                    <div className="flex items-center justify-between p-4">
                      <div className="flex items-center gap-2 text-lg font-medium">
                        {project === "standalone" ? (
                          <>
                            📦 Standalone Containers
                            <div className="badge badge-neutral">
                              {projectContainers.length}
                            </div>
                          </>
                        ) : (
                          <>
                            🐙 {project}
                            <div className="badge badge-primary">
                              {projectContainers.length}
                            </div>
                          </>
                        )}
                      </div>
                      {project !== "standalone" && (
                        <div className="flex gap-2">
                          {projectContainers.every(
                            (c) => c.status === "running"
                          ) ? (
                            <button
                              className="btn btn-xs btn-error"
                              onClick={() => handleStopProject(project)}
                              disabled={stopProject.isPending}
                            >
                              {stopProject.isPending ? (
                                <span className="loading loading-spinner loading-xs"></span>
                              ) : (
                                "⏹ Stop All"
                              )}
                            </button>
                          ) : projectContainers.every(
                              (c) => c.status !== "running"
                            ) ? (
                            <button
                              className="btn btn-xs btn-success"
                              onClick={() => handleStartProject(project)}
                              disabled={startProject.isPending}
                            >
                              {startProject.isPending ? (
                                <span className="loading loading-spinner loading-xs"></span>
                              ) : (
                                "▶ Start All"
                              )}
                            </button>
                          ) : (
                            <>
                              <button
                                className="btn btn-xs btn-success"
                                onClick={() => handleStartProject(project)}
                                disabled={startProject.isPending}
                              >
                                {startProject.isPending ? (
                                  <span className="loading loading-spinner loading-xs"></span>
                                ) : (
                                  "▶ Start"
                                )}
                              </button>
                              <button
                                className="btn btn-xs btn-error"
                                onClick={() => handleStopProject(project)}
                                disabled={stopProject.isPending}
                              >
                                {stopProject.isPending ? (
                                  <span className="loading loading-spinner loading-xs"></span>
                                ) : (
                                  "⏹ Stop"
                                )}
                              </button>
                            </>
                          )}
                        </div>
                      )}
                    </div>
                    {/* Collapsible content */}
                    <div className="collapse collapse-arrow">
                      <input
                        aria-label="Toggle container group"
                        type="checkbox"
                        defaultChecked
                      />
                      <div className="collapse-title text-sm font-medium">
                        Show/Hide Containers
                      </div>
                      <div className="collapse-content">
                        <div className="overflow-x-auto">
                          <table className="table table-zebra table-sm">
                            <thead>
                              <tr>
                                <th>ID</th>
                                <th>Name</th>
                                <th>Status</th>
                                <th>Image</th>
                                <th>Actions</th>
                              </tr>
                            </thead>
                            <tbody>
                              {projectContainers.map((container) => (
                                <tr key={container.id}>
                                  <td>
                                    <code className="text-xs">
                                      {container.id}
                                    </code>
                                  </td>
                                  <td className="font-semibold">
                                    {container.name}
                                  </td>
                                  <td>
                                    <div
                                      className={`badge badge-sm ${
                                        container.status === "running"
                                          ? "badge-success"
                                          : container.status === "exited"
                                          ? "badge-error"
                                          : "badge-warning"
                                      }`}
                                    >
                                      {container.status}
                                    </div>
                                  </td>
                                  <td className="text-sm">{container.image}</td>
                                  <td>
                                    <div className="flex gap-2">
                                      {container.status === "running" ? (
                                        <button
                                          className="btn btn-xs btn-error"
                                          onClick={() =>
                                            handleStopContainer(container.id)
                                          }
                                          disabled={stopContainer.isPending}
                                        >
                                          {stopContainer.isPending ? (
                                            <span className="loading loading-spinner loading-xs"></span>
                                          ) : (
                                            "⏹ Stop"
                                          )}
                                        </button>
                                      ) : (
                                        <button
                                          className="btn btn-xs btn-success"
                                          onClick={() =>
                                            handleStartContainer(container.id)
                                          }
                                          disabled={startContainer.isPending}
                                        >
                                          {startContainer.isPending ? (
                                            <span className="loading loading-spinner loading-xs"></span>
                                          ) : (
                                            "▶ Start"
                                          )}
                                        </button>
                                      )}
                                    </div>
                                  </td>
                                </tr>
                              ))}
                            </tbody>
                          </table>
                        </div>
                      </div>
                    </div>
                  </div>
                )
              );
            })()}
          </div>
        ) : (
          <div className="text-center py-8 text-base-content/60">
            <p>No containers found</p>
          </div>
        )}
      </div>
    </div>
  );
};

export default ContainersTable;
