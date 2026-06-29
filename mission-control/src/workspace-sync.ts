import type { WorkspaceSnapshot, WorkspaceUpdateBatch } from "./contracts";

export type WorkspaceUpdateResult =
  | { readonly kind: "applied"; readonly snapshot: WorkspaceSnapshot }
  | { readonly kind: "resync-required"; readonly reason: string };

export function applyWorkspaceUpdates(
  snapshot: WorkspaceSnapshot,
  batch: WorkspaceUpdateBatch,
): WorkspaceUpdateResult {
  if (batch.after_revision !== snapshot.revision) {
    return { kind: "resync-required", reason: "Update batch starts from a different revision." };
  }
  if (batch.current_revision < batch.after_revision) {
    return { kind: "resync-required", reason: "Update batch is malformed or has a revision gap." };
  }
  const expectedRevisions = Array.from(
    { length: batch.current_revision - batch.after_revision },
    (_, index) => batch.after_revision + index + 1,
  );
  if (
    expectedRevisions.length !== batch.events.length ||
    batch.events.some(
      (event, index) =>
        event.revision !== expectedRevisions[index] ||
        event.kind !== "workspace-preferences-updated" ||
        event.active_mission_id !== snapshot.active_mission?.id,
    )
  ) {
    return { kind: "resync-required", reason: "Update batch is malformed or has a revision gap." };
  }
  const finalEvent = batch.events.at(-1);
  if (!finalEvent) {
    return batch.current_revision === snapshot.revision
      ? { kind: "applied", snapshot }
      : { kind: "resync-required", reason: "Update batch omitted acknowledged revisions." };
  }
  return {
    kind: "applied",
    snapshot: {
      ...snapshot,
      revision: batch.current_revision,
      conversation_scope: finalEvent.conversation_scope,
      operations_view: finalEvent.operations_view,
    },
  };
}
