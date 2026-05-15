import { useCallback, useMemo } from 'react';
import { useJsonPatchWsStream } from './useJsonPatchWsStream';
import type { ProjectWithStatus } from 'shared/types';

type ProjectsState = {
  projects: Record<string, ProjectWithStatus>;
};

export interface UseProjectsResult {
  projects: ProjectWithStatus[];
  projectsById: Record<string, ProjectWithStatus>;
  isLoading: boolean;
  isConnected: boolean;
  error: Error | null;
}

export function useProjects(): UseProjectsResult {
  const endpoint = '/api/projects/stream/ws';

  const initialData = useCallback((): ProjectsState => ({ projects: {} }), []);

  const { data, isConnected, isInitialized, error } =
    useJsonPatchWsStream<ProjectsState>(endpoint, true, initialData);

  const projectsById = useMemo(() => data?.projects ?? {}, [data]);

  const projects = useMemo(() => {
    return Object.values(projectsById).sort(
      (a, b) =>
        new Date(b.created_at as unknown as string).getTime() -
        new Date(a.created_at as unknown as string).getTime()
    );
  }, [projectsById]);

  const projectsData = data ? projects : undefined;
  const errorObj = useMemo(() => (error ? new Error(error) : null), [error]);

  return {
    projects: projectsData ?? [],
    projectsById,
    isLoading: !isInitialized && !error,
    isConnected,
    error: errorObj,
  };
}
