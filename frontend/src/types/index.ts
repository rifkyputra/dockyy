export interface Container {
  id: string;
  name: string;
  status: string;
  image: string;
  compose_project?: string;
}

export interface Repository {
  id: number;
  name: string;
  owner: string;
  url: string;
  description?: string;
  webhook_url?: string;
  filesystem_path?: string;
  is_private: boolean;
  default_branch: string;
  created_at?: string;
  updated_at?: string;
}

export interface CreateRepositoryInput {
  name: string;
  owner: string;
  url: string;
  description?: string;
  webhook_url?: string;
  filesystem_path?: string;
  is_private?: boolean;
  default_branch?: string;
}

export interface UpdateRepositoryInput {
  name?: string;
  owner?: string;
  url?: string;
  description?: string;
  webhook_url?: string;
  filesystem_path?: string;
  is_private?: boolean;
  default_branch?: string;
}
