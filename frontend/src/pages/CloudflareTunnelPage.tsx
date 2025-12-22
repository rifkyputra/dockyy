import {
  useCloudflaredStatus,
  useCloudflaredConfig,
  useCloudflaredTunnels,
} from "../hooks/useApi";

export default function CloudflareTunnelPage() {
  const { data: cloudflaredStatus, isLoading, error } = useCloudflaredStatus();
  const { data: configData } = useCloudflaredConfig();
  const { data: tunnelsData } = useCloudflaredTunnels();

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
          <h2 className="card-title">Cloudflare Configuration</h2>
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
          <h2 className="card-title">Cloudflare Tunnel Management</h2>
          <p className="text-base-content/70">
            Cloudflare Tunnel management interface coming soon...
          </p>
        </div>
      </div>
    </div>
  );
}
