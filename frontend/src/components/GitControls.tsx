import React, { useState } from "react";
import { repositoryApi } from "@/lib/api";

type Props = {
  repoId: number;
};

const GitControls: React.FC<Props> = ({ repoId }) => {
  const [loading, setLoading] = useState<string | null>(null);
  const [output, setOutput] = useState<string>("");
  const [diffFiles, setDiffFiles] = useState<any[] | null>(null);

  const run = async (action: "status" | "fetch" | "pull") => {
    setLoading(action);
    setOutput("");
    try {
      let res: any;
      if (action === "status") {
        res = await repositoryApi.gitStatus(repoId);
      } else if (action === "fetch") {
        res = await repositoryApi.gitFetch(repoId);
      } else {
        res = await repositoryApi.gitPull(repoId);
      }
      // if status, we may receive structured diff
      if (action === "status") {
        setDiffFiles(res.diff || null);
        const st = res.status || {};
        const out = [] as string[];
        if (st.stdout) out.push(st.stdout);
        if (st.stderr) out.push(st.stderr);
        out.push(`exit code: ${st.returncode ?? ""}`);
        setOutput(out.join("\n"));
      } else {
        const out = [] as string[];
        if (res.stdout) out.push(res.stdout);
        if (res.stderr) out.push(res.stderr);
        out.push(`exit code: ${res.returncode}`);
        setOutput(out.join("\n"));
        setDiffFiles(null);
      }
    } catch (err: any) {
      setOutput(err?.message || String(err));
    } finally {
      setLoading(null);
    }
  };

  return (
    <div className="card bg-base-100 shadow-md">
      <div className="card-body">
        <div className="flex justify-between items-center">
          <h2 className="card-title">Git</h2>
          <div className="flex gap-2">
            <button
              className="btn btn-sm"
              onClick={() => run("status")}
              disabled={!!loading}
            >
              {loading === "status" ? (
                <span className="loading loading-spinner loading-sm"></span>
              ) : (
                "Status"
              )}
            </button>
            <button
              className="btn btn-sm"
              onClick={() => run("fetch")}
              disabled={!!loading}
            >
              {loading === "fetch" ? (
                <span className="loading loading-spinner loading-sm"></span>
              ) : (
                "Fetch"
              )}
            </button>
            <button
              className="btn btn-sm btn-primary"
              onClick={() => run("pull")}
              disabled={!!loading}
            >
              {loading === "pull" ? (
                <span className="loading loading-spinner loading-sm"></span>
              ) : (
                "Pull"
              )}
            </button>
          </div>
        </div>

        <div className="mt-3">
          <label className="label">
            <span className="label-text">Output</span>
          </label>
          <pre className="bg-base-200 p-3 rounded text-sm whitespace-pre-wrap">
            {output || "(no output)"}
          </pre>
        </div>
        {diffFiles && diffFiles.length > 0 && (
          <div className="mt-3">
            <label className="label">
              <span className="label-text">Changed Lines</span>
            </label>
            <div className="space-y-2">
              {diffFiles.map((f, idx) => (
                <div key={idx} className="border rounded p-2">
                  <div className="font-medium">{f.to || f.from}</div>
                  {f.hunks.map((h: any, hi: number) => (
                    <div key={hi} className="mt-2">
                      <div className="text-xs text-base-content/60">
                        Hunk: +{h.new_start},{h.new_count} -{h.old_start},
                        {h.old_count}
                      </div>
                      <pre className="bg-base-200 p-2 rounded text-sm overflow-auto">
                        {h.changes.map((c: any, ci: number) => (
                          <div key={ci}>
                            <span
                              className={
                                c.type === "add"
                                  ? "text-green-600"
                                  : "text-red-600"
                              }
                            >
                              {c.type === "add" ? "+" : "-"}
                              {c.line}
                            </span>{" "}
                            {c.content}
                          </div>
                        ))}
                      </pre>
                    </div>
                  ))}
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

export default GitControls;
