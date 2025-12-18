import { useCloudflaredStatus } from "../hooks/useApi";

export default function CloudflareTunnelPage() {
  const { data: cloudflaredStatus, isLoading, error } = useCloudflaredStatus();

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
          <h2 className="card-title">Cloudflare Tunnel Management</h2>
          <p className="text-base-content/70">
            Cloudflare Tunnel management interface coming soon...
          </p>
        </div>
      </div>
    </div>
  );
}
