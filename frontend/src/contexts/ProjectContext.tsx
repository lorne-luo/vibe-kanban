import { useContext, ReactNode, useMemo } from 'react';
import { createHmrContext } from '@/lib/hmrContext.ts';
import { useLocation } from 'react-router-dom';
import type { ProjectWithStatus, ProjectWorkflowStatus } from 'shared/types';
import { useProjects } from '@/hooks/useProjects';
import { usePageTitle } from '@/hooks/usePageTitle';

interface ProjectContextValue {
  projectId: string | undefined;
  project: ProjectWithStatus | undefined;
  workflow_status: ProjectWorkflowStatus | null;
  isLoading: boolean;
  error: Error | null;
  isError: boolean;
}

const ProjectContext = createHmrContext<ProjectContextValue | null>(
  'ProjectContext',
  null
);

interface ProjectProviderProps {
  children: ReactNode;
}

export function ProjectProvider({ children }: ProjectProviderProps) {
  const location = useLocation();

  // Extract projectId from current route path
  const projectId = useMemo(() => {
    const match = location.pathname.match(/^\/local-projects\/([^/]+)/);
    return match ? match[1] : undefined;
  }, [location.pathname]);

  const { projectsById, isLoading, error } = useProjects();
  const project = projectId ? projectsById[projectId] : undefined;
  const workflow_status = project?.workflow_status ?? null;

  const value = useMemo(
    () => ({
      projectId,
      project,
      workflow_status,
      isLoading,
      error,
      isError: !!error,
    }),
    [projectId, project, workflow_status, isLoading, error]
  );

  usePageTitle(project?.name);

  return (
    <ProjectContext.Provider value={value}>{children}</ProjectContext.Provider>
  );
}

export function useProject(): ProjectContextValue {
  const context = useContext(ProjectContext);
  if (!context) {
    throw new Error('useProject must be used within a ProjectProvider');
  }
  return context;
}
