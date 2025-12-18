import React from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useReadme } from "@/hooks/useApi";

type props = {
  repositoryId: number;
  filesystemPath?: string;
};

const ReadmeSection: React.FC<props> = ({ repositoryId, filesystemPath }) => {
  const { data, isLoading, error } = useReadme(repositoryId, filesystemPath);

  return (
    <div className="card bg-base-100 shadow-md">
      <div className="card-body">
        <div className="flex justify-between items-center">
          <h2 className="card-title">Readme</h2>
        </div>
        <div className="mt-2">
          {isLoading ? (
            <div className="flex justify-center items-center py-8">
              <span className="loading loading-spinner loading-lg"></span>
            </div>
          ) : error ? (
            <div className="alert alert-error">
              <span>Failed to load README</span>
            </div>
          ) : data?.content ? (
            <div className="prose max-w-full">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>
                {data.content}
              </ReactMarkdown>
            </div>
          ) : (
            <p className="text-sm text-base-content/60">No README available.</p>
          )}
        </div>
      </div>
    </div>
  );
};

export default ReadmeSection;
