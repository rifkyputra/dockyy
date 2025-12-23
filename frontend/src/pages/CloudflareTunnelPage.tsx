import {
  useCloudflaredStatus,
  useCloudflaredConfig,
  useCloudflaredTunnels,
} from "../hooks/useApi";
import { useState } from "react";
import { tunnelApi } from "../lib/api";
import { useMutation, useQueryClient } from "@tanstack/react-query";

export default function CloudflareTunnelPage() {
  const { data: cloudflaredStatus, isLoading, error } = useCloudflaredStatus();
  const { data: configData } = useCloudflaredConfig();
  const { data: tunnelsData } = useCloudflaredTunnels();
  const [isEditModalOpen, setIsEditModalOpen] = useState(false);
  const [editedConfig, setEditedConfig] = useState("");
  const [configError, setConfigError] = useState<string | null>(null);

  const queryClient = useQueryClient();

  const updateConfigMutation = useMutation({
    mutationFn: (config: any) => tunnelApi.updateConfig(config),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["cloudflaredConfig"] });
      setIsEditModalOpen(false);
      setConfigError(null);
    },
    onError: (error: Error) => {
      setConfigError(error.message);
    },
  });

  const installServiceMutation = useMutation({
    mutationFn: () =>
      tunnelApi.installService("/home/ubuntu/.cloudflared/config.yaml"),
    onSuccess: (data) => {
      alert(data.message || "Service installed successfully");
    },
    onError: (error: Error) => {
      alert("Error installing service: " + error.message);
    },
  });

  const uninstallServiceMutation = useMutation({
    mutationFn: () =>
      tunnelApi.uninstallService("/home/ubuntu/.cloudflared/config.yaml"),
    onSuccess: (data) => {
      alert(data.message || "Service uninstalled successfully");
    },
    onError: (error: Error) => {
      alert("Error uninstalling service: " + error.message);
    },
  });

  const handleEditConfig = () => {
    if (configData?.config) {
      setEditedConfig(JSON.stringify(configData.config, null, 2));
      setConfigError(null);
      setIsEditModalOpen(true);
    }
  };

  const handleSaveConfig = () => {
    try {
      const parsedConfig = JSON.parse(editedConfig);
      updateConfigMutation.mutate(parsedConfig);
    } catch (e) {
      setConfigError("Invalid JSON format. Please check your configuration.");
    }
  };

  return (
    <div className="space-y-6">
      <h1 className="text-3xl font-bold">Cloudflare Tunnel</h1>

      <div className="card bg-base-100 shadow-xl">
        <div className="card-body">
          <h2 className="card-title">Cloudflared Status</h2>
          {isLoading ? (
            <div className="flex items-center space-x-2">
              <span className="loading loading-spinner loading-sm"></span>
              <span>Checking cloudflared installation...</span>
            </div>
          ) : error ? (
            <div className="alert alert-error">
              <span>Error checking cloudflared: {error.message}</span>
            </div>
          ) : cloudflaredStatus ? (
            <div className="space-y-2">
              <div className="flex items-center space-x-2">
                <div
                  className={`badge ${
                    cloudflaredStatus.installed
                      ? "badge-success"
                      : "badge-error"
                  }`}
                >
                  {cloudflaredStatus.installed ? "Installed" : "Not Installed"}
                </div>
                {cloudflaredStatus.version && (
                  <span className="text-sm text-base-content/70">
                    Version: {cloudflaredStatus.version}
                  </span>
                )}
              </div>
              {!cloudflaredStatus.installed && cloudflaredStatus.error && (
                <p className="text-error text-sm">{cloudflaredStatus.error}</p>
              )}
            </div>
          ) : null}
        </div>
      </div>

      <div className="card bg-base-100 shadow-xl">
        <div className="card-body">
          <div className="flex justify-between items-center">
            <h2 className="card-title">Cloudflare Configuration</h2>
            {configData?.config && (
              <button
                className="btn btn-primary btn-sm"
                onClick={handleEditConfig}
              >
                Edit Configuration
              </button>
            )}
          </div>
          {configData?.config ? (
            <div className="space-y-4">
              <div className="text-sm text-base-content/70">
                Config file: {configData.config_path}
              </div>
              <pre className="bg-base-200 p-4 rounded-lg overflow-x-auto text-sm">
                {JSON.stringify(configData.config, null, 2)}
              </pre>
            </div>
          ) : configData?.error ? (
            <div className="alert alert-warning">
              <span>{configData.error}</span>
            </div>
          ) : (
            <div className="text-base-content/70">Loading configuration...</div>
          )}
        </div>
      </div>

      <div className="card bg-base-100 shadow-xl">
        <div className="card-body">
          <h2 className="card-title">Cloudflare Tunnels</h2>
          {tunnelsData?.tunnels ? (
            <div className="space-y-4">
              {tunnelsData.tunnels.length > 0 ? (
                <div className="overflow-x-auto">
                  <table className="table table-zebra w-full">
                    <thead>
                      <tr>
                        <th>Name</th>
                        <th>ID</th>
                        <th>Created</th>
                        <th>Connections</th>
                      </tr>
                    </thead>
                    <tbody>
                      {tunnelsData.tunnels.map((tunnel: any) => (
                        <tr key={tunnel.id}>
                          <td>{tunnel.name}</td>
                          <td className="font-mono text-sm">{tunnel.id}</td>
                          <td>
                            {new Date(tunnel.created_at).toLocaleString()}
                          </td>
                          <td>{tunnel.connections || 0}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              ) : (
                <div className="text-base-content/70">No tunnels found</div>
              )}
            </div>
          ) : tunnelsData?.error ? (
            <div className="alert alert-warning">
              <span>{tunnelsData.error}</span>
            </div>
          ) : (
            <div className="text-base-content/70">Loading tunnels...</div>
          )}
        </div>
      </div>

      <div className="card bg-base-100 shadow-xl">
        <div className="card-body">
          <h2 className="card-title">Cloudflare Tunnel Service</h2>
          <p className="text-base-content/70 mb-4">
            Install or uninstall the cloudflared service as a system service.
          </p>
          <div className="flex gap-4">
            <button
              className="btn btn-success"
              onClick={() => installServiceMutation.mutate()}
              disabled={
                installServiceMutation.isPending ||
                uninstallServiceMutation.isPending
              }
            >
              {installServiceMutation.isPending ? (
                <>
                  <span className="loading loading-spinner loading-sm"></span>
                  Installing...
                </>
              ) : (
                "Install Service"
              )}
            </button>
            <button
              className="btn btn-error"
              onClick={() => uninstallServiceMutation.mutate()}
              disabled={
                installServiceMutation.isPending ||
                uninstallServiceMutation.isPending
              }
            >
              {uninstallServiceMutation.isPending ? (
                <>
                  <span className="loading loading-spinner loading-sm"></span>
                  Uninstalling...
                </>
              ) : (
                "Uninstall Service"
              )}
            </button>
          </div>
        </div>
      </div>

      {/* Edit Configuration Modal */}
      {isEditModalOpen && (
        <div className="modal modal-open">
          <div className="modal-box max-w-4xl">
            <h3 className="font-bold text-lg mb-4">
              Edit Cloudflare Configuration
            </h3>

            {configError && (
              <div className="alert alert-error mb-4">
                <span>{configError}</span>
              </div>
            )}

            <div className="form-control">
              <label className="label">
                <span className="label-text">Configuration (JSON format)</span>
              </label>
              <textarea
                className="textarea textarea-bordered h-96 font-mono text-sm"
                value={editedConfig}
                onChange={(e) => setEditedConfig(e.target.value)}
                placeholder="Enter configuration in JSON format"
              />
            </div>

            <div className="modal-action">
              <button
                className="btn btn-ghost"
                onClick={() => {
                  setIsEditModalOpen(false);
                  setConfigError(null);
                }}
                disabled={updateConfigMutation.isPending}
              >
                Cancel
              </button>
              <button
                className="btn btn-primary"
                onClick={handleSaveConfig}
                disabled={updateConfigMutation.isPending}
              >
                {updateConfigMutation.isPending ? (
                  <>
                    <span className="loading loading-spinner loading-sm"></span>
                    Saving...
                  </>
                ) : (
                  "Save Configuration"
                )}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
