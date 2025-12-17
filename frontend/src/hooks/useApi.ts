import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { containerApi, repositoryApi } from "../lib/api";
import type { CreateRepositoryInput, UpdateRepositoryInput } from "../types";

// Container hooks
export const useContainers = () => {
  return useQuery({
    queryKey: ["containers"],
    queryFn: containerApi.getAll,
  });
};

export const useStartContainer = () => {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (containerId: string) => containerApi.start(containerId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["containers"] });
    },
  });
};

export const useStopContainer = () => {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (containerId: string) => containerApi.stop(containerId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["containers"] });
    },
  });
};

export const useStartProject = () => {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (projectName: string) => containerApi.startProject(projectName),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["containers"] });
    },
  });
};

export const useStopProject = () => {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (projectName: string) => containerApi.stopProject(projectName),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["containers"] });
    },
  });
};

export const useRestartProject = () => {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (projectName: string) => containerApi.restartProject(projectName),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["containers"] });
    },
  });
};

export const useRebuildProject = () => {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ projectName, path }: { projectName: string; path?: string }) =>
      containerApi.rebuildProject(projectName, path),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["containers"] });
    },
  });
};

// Repository hooks
export const useRepositories = () => {
  return useQuery({
    queryKey: ["repositories"],
    queryFn: repositoryApi.getAll,
  });
};

export const useRepository = (id: number) => {
  return useQuery({
    queryKey: ["repositories", id],
    queryFn: () => repositoryApi.getById(id),
    enabled: !!id,
  });
};

export const useCreateRepository = () => {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (data: CreateRepositoryInput) => repositoryApi.create(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
    },
  });
};

export const useUpdateRepository = () => {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, data }: { id: number; data: UpdateRepositoryInput }) =>
      repositoryApi.update(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
    },
  });
};

export const useDeleteRepository = () => {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => repositoryApi.delete(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
    },
  });
};

export const useRepositoryFilesystemStatus = (id: number) => {
  return useQuery({
    queryKey: ["repositories", id, "filesystem-status"],
    queryFn: () => repositoryApi.getFilesystemStatus(id),
    enabled: !!id,
  });
};

export const useRepositoryComposeFile = (id: number) => {
  return useQuery({
    queryKey: ["repositories", id, "compose-file"],
    queryFn: () => repositoryApi.getComposeFile(id),
    enabled: !!id,
  });
};
