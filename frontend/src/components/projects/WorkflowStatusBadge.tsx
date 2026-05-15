import { Circle } from 'lucide-react';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';
import type { ProjectWorkflowStatus } from 'shared/types';

type Variant = 'dot' | 'pill';

interface Props {
  status: ProjectWorkflowStatus | null | undefined;
  variant?: Variant;
  onClick?: () => void;
}

const COLOR: Record<ProjectWorkflowStatus['state'], string> = {
  ready: 'text-emerald-500',
  invalid: 'text-red-500',
  not_configured: 'text-muted-foreground',
};

const LABEL: Record<ProjectWorkflowStatus['state'], string> = {
  ready: 'Ready',
  invalid: 'Issues',
  not_configured: 'No workflow',
};

function tooltipText(status: ProjectWorkflowStatus): string {
  const total = status.repos.reduce((n, r) => n + r.workflows.length, 0);
  switch (status.state) {
    case 'ready':
      return `Workflow agent ready (${total} workflow${total === 1 ? '' : 's'})`;
    case 'invalid':
      return 'Workflow has issues — click for details';
    case 'not_configured':
      return 'No .agents/kanban-workflows/ configured';
  }
}

export function WorkflowStatusBadge({
  status,
  variant = 'dot',
  onClick,
}: Props) {
  if (!status) return null;
  const color = COLOR[status.state];
  const tip = tooltipText(status);
  const clickable = !!onClick;

  const content =
    variant === 'pill' ? (
      <span
        className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium bg-muted ${color}`}
      >
        <Circle className="h-2.5 w-2.5 fill-current" />
        {LABEL[status.state]}
      </span>
    ) : (
      <Circle className={`h-2.5 w-2.5 fill-current ${color}`} />
    );

  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            aria-label={LABEL[status.state]}
            onClick={onClick}
            disabled={!clickable}
            className={clickable ? 'cursor-pointer' : 'cursor-default'}
          >
            {content}
          </button>
        </TooltipTrigger>
        <TooltipContent>{tip}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
