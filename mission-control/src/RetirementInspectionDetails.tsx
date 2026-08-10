import type { ReactElement } from "react";
import type { WorkstationCardDetail } from "./workstation-projection";

export interface RetirementInspectionDetailsProps {
  readonly detail: WorkstationCardDetail;
  readonly sectionClassName: string;
  readonly factsClassName?: string;
}

function humanize(value: string | undefined, fallback = "Not recorded"): string {
  if (!value) return fallback;
  return value
    .split("-")
    .map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1)}`)
    .join(" ");
}

function formatBytes(value: number | undefined): string {
  if (value === undefined) return "Not recorded";
  const units = ["bytes", "KiB", "MiB", "GiB"] as const;
  let amount = value;
  let unit = 0;
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit += 1;
  }
  return `${Number.isInteger(amount) ? amount : amount.toFixed(1)} ${units[unit]}`;
}

export function RetirementInspectionDetails({
  detail,
  sectionClassName,
  factsClassName,
}: RetirementInspectionDetailsProps): ReactElement | null {
  const blocked =
    detail.retirementPhase === "preservation-blocked" ||
    detail.retirementPhase === "retirement-blocked";
  const runner = detail.retirementRunnerBoundary;
  const budget = detail.preservationBudget;
  const record = detail.retirementRecord;
  if (!blocked && !record) return null;

  return (
    <>
      {blocked ? (
        <section className={sectionClassName}>
          <h5>Retirement Unit Inspection</h5>
          <dl className={factsClassName}>
            <div><dt>Phase</dt><dd>{humanize(detail.retirementPhase)}</dd></div>
            <div><dt>Blocked reason</dt><dd>{detail.retirementBlockedReason || "No reason recorded."}</dd></div>
            <div><dt>Runner operation</dt><dd><code>{runner?.runner_operation_id || "Not recorded"}</code></dd></div>
            <div><dt>Runner owner</dt><dd>{runner?.owner_released_at ? `Released at ${runner.owner_released_at}` : runner?.owner_identity || "Not recorded"}</dd></div>
            <div><dt>Process group</dt><dd>{runner?.process_group_identity || "Not recorded"}</dd></div>
            <div><dt>Preservation state</dt><dd>{humanize(budget?.state)}</dd></div>
            <div><dt>Reserved capacity</dt><dd>{formatBytes(budget?.reserved_bytes)}</dd></div>
            <div><dt>Budget bound</dt><dd>{budget?.bound === undefined ? "Not recorded" : budget.bound ? "Yes" : "No"}</dd></div>
          </dl>
        </section>
      ) : null}
      {record ? (
        <section className={sectionClassName}>
          <h5>Retirement Record</h5>
          <dl className={factsClassName}>
            <div><dt>Disposition</dt><dd>{humanize(record.payload_disposition, "Unknown")}</dd></div>
            <div><dt>Manifest</dt><dd><code>{record.manifest_sha256 || "Not recorded"}</code></dd></div>
            <div><dt>Worktree identity</dt><dd><code>{record.worktree_identity || "Not recorded"}</code></dd></div>
            <div><dt>Payload size</dt><dd>{formatBytes(record.snapshot_bytes)}</dd></div>
            <div><dt>Expires</dt><dd>{record.expires_at || "Not recorded"}</dd></div>
            <div><dt>Pinned</dt><dd>{record.pinned ? "Yes" : "No"}</dd></div>
          </dl>
        </section>
      ) : null}
    </>
  );
}
