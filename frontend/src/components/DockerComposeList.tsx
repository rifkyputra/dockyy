import React, { useState } from "react";
import type { DockerComposeFile } from "../types";

type Props = {
  composeFiles: DockerComposeFile[];
};

const DockerComposeList: React.FC<Props> = ({ composeFiles = [] }: Props) => {
  const [selectedFile, setSelectedFile] = useState<DockerComposeFile | null>(
    null
  );

  const handleAction = (
    fileId: string,
    action: "start" | "stop" | "restart" | "rebuild"
  ) => {
    // Mock action - in real implementation, this would call the API
    console.log(`Performing ${action} on compose file ${fileId}`);
    // You could update the status here based on the action
  };

  const getStatusColor = (status: string) => {
    switch (status) {
      case "running":
        return "badge-success";
      case "stopped":
        return "badge-warning";
      case "error":
        return "badge-error";
      default:
        return "badge-neutral";
    }
  };

  return (
    <div className="p-6">
      <h2 className="text-2xl font-bold mb-6">Docker Compose Files</h2>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Compose Files List */}
        <div className="space-y-4">
          <h3 className="text-lg font-semibold">Compose Files</h3>
          {composeFiles.length === 0 ? (
            <div className="card bg-base-100 shadow-md">
              <div className="card-body p-8 text-center">
                <div className="text-gray-500">
                  <svg
                    className="w-16 h-16 mx-auto mb-4 opacity-50"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"
                    />
                  </svg>
                  <p>No compose files found</p>
                </div>
              </div>
            </div>
          ) : (
            composeFiles.map((file) => (
              <div
                key={file.id}
                className={`card bg-base-100 shadow-md cursor-pointer transition-all hover:shadow-lg ${
                  selectedFile?.id === file.id ? "ring-2 ring-primary" : ""
                }`}
                onClick={() => setSelectedFile(file)}
              >
                <div className="card-body p-4">
                  <div className="flex justify-between items-start mb-3">
                    <div>
                      <h4 className="card-title text-base">{file.name}</h4>
                      <p className="text-sm text-gray-600 truncate max-w-xs">
                        {file.path}
                      </p>
                    </div>
                    <div className={`badge ${getStatusColor(file.status)}`}>
                      {file.status}
                    </div>
                  </div>

                  <div className="flex flex-wrap gap-2">
                    <button
                      className="btn btn-sm btn-success"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleAction(file.id, "start");
                      }}
                      disabled={file.status === "running"}
                    >
                      Start
                    </button>
                    <button
                      className="btn btn-sm btn-warning"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleAction(file.id, "stop");
                      }}
                      disabled={file.status === "stopped"}
                    >
                      Stop
                    </button>
                    <button
                      className="btn btn-sm btn-info"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleAction(file.id, "restart");
                      }}
                    >
                      Restart
                    </button>
                    <button
                      className="btn btn-sm btn-secondary"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleAction(file.id, "rebuild");
                      }}
                    >
                      Rebuild
                    </button>
                  </div>
                </div>
              </div>
            ))
          )}
        </div>

        {/* Compose File Viewer */}
        {composeFiles.length > 0 && (
          <div className="space-y-4">
            <h3 className="text-lg font-semibold">Compose File Viewer</h3>
            {selectedFile ? (
              <div className="card bg-base-100 shadow-md">
                <div className="card-body p-4">
                  <div className="flex justify-between items-center mb-4">
                    <h4 className="card-title text-base">
                      {selectedFile.name}
                    </h4>
                    <div
                      className={`badge ${getStatusColor(selectedFile.status)}`}
                    >
                      {selectedFile.status}
                    </div>
                  </div>
                  <div className="mb-4">
                    <label className="label">
                      <span className="label-text font-medium">File Path:</span>
                    </label>
                    <div className="bg-base-200 p-2 rounded text-sm font-mono break-all">
                      {selectedFile.path}
                    </div>
                  </div>
                  <div>
                    <label className="label">
                      <span className="label-text font-medium">Content:</span>
                    </label>
                    <pre className="bg-base-200 p-4 rounded text-sm overflow-x-auto max-h-96 whitespace-pre-wrap">
                      {selectedFile.content}
                    </pre>
                  </div>
                </div>
              </div>
            ) : (
              <div className="card bg-base-100 shadow-md">
                <div className="card-body p-8 text-center">
                  <div className="text-gray-500">
                    <svg
                      className="w-16 h-16 mx-auto mb-4 opacity-50"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24"
                    >
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"
                      />
                    </svg>
                    <p>Select a compose file to view its contents</p>
                  </div>
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
};

export default DockerComposeList;
