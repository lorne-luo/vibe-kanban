import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { CheckCircle2, XCircle } from 'lucide-react';
import type {
  ProjectWorkflowStatus,
  WorkflowEntryState,
} from 'shared/types';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  status: ProjectWorkflowStatus;
  onReload?: () => void;
}

function entryIcon(state: WorkflowEntryState) {
  return state === 'ok' ? (
    <CheckCircle2 className="h-4 w-4 text-emerald-500" />
  ) : (
    <XCircle className="h-4 w-4 text-red-500" />
  );
}

export function WorkflowStatusDialog({
  open,
  onOpenChange,
  status,
  onReload,
}: Props) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            Workflow status — checked at{' '}
            {new Date(status.checked_at).toLocaleTimeString()}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          {status.repos.map((r) => (
            <div key={r.repo_id} className="border rounded p-3">
              <div className="font-mono text-sm mb-1">{r.workflows_dir}</div>
              <div className="text-xs text-muted-foreground mb-2">
                repo: {r.repo_name}
              </div>
              {r.error && (
                <div className="text-red-500 text-sm mb-2">{r.error}</div>
              )}
              {r.workflows.length === 0 && !r.error && (
                <div className="text-muted-foreground text-sm italic">
                  no .yml files
                </div>
              )}
              <ul className="space-y-1">
                {r.workflows.map((w) => (
                  <li key={w.name} className="flex items-start gap-2 text-sm">
                    {entryIcon(w.state)}
                    <span className="font-medium">{w.name}</span>
                    {w.error && (
                      <span className="text-muted-foreground">— {w.error}</span>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>

        {onReload && (
          <div className="mt-4 flex justify-end">
            <button
              type="button"
              className="text-sm px-3 py-1 rounded border hover:bg-muted"
              onClick={onReload}
            >
              Reload now
            </button>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
