import { useEffect, useRef } from "react";
import type { ShellTerminalController } from "./use-shell-terminal";

export function ShellTerminalPanel({
  terminal,
}: {
  readonly terminal: ShellTerminalController;
}) {
  const actionStatusRef = useRef<HTMLParagraphElement>(null);
  useEffect(() => {
    if (terminal.actionStatus) actionStatusRef.current?.focus();
  }, [terminal.actionStatus]);
  return (
    <section className="shell-terminal" aria-label="Shell Terminal">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">Audit drill-down</span>
          <h2>Shell Terminal Detail</h2>
        </div>
      </div>
      <div className="terminal-boundary" aria-label="Execution boundary">
        <span className="eyebrow">Working directory</span>
        <code>{terminal.workingDirectory}</code>
        <span>Requester / mission-commander</span>
        <span>Access / {terminal.accessLevel}</span>
      </div>
      <div className="terminal-actions">
        <label className="terminal-field">
          <span>Command</span>
          <textarea
            aria-label="Command"
            value={terminal.commandDraft}
            onChange={(event) => terminal.setCommandDraft(event.target.value)}
          />
        </label>
        <label className="terminal-field">
          <span>Working directory</span>
          <input
            aria-label="Working directory"
            value={terminal.workingDirectory}
            onChange={(event) => terminal.setWorkingDirectory(event.target.value)}
          />
        </label>
        <label className="terminal-field">
          <span>Requested paths</span>
          <textarea
            aria-label="Requested paths"
            value={terminal.requestedPathsDraft}
            onChange={(event) => terminal.setRequestedPathsDraft(event.target.value)}
          />
        </label>
        <label className="terminal-field">
          <span>Access level</span>
          <select
            aria-label="Access level"
            value={terminal.accessLevel}
            onChange={(event) => terminal.setAccessLevel(event.target.value as "read" | "write")}
          >
            <option value="read">Read</option>
            <option value="write">Write</option>
          </select>
        </label>
        <button
          type="button"
          disabled={
            !terminal.commandDraft.trim() ||
            !terminal.workingDirectory.trim() ||
            terminal.actionStatus?.state === "pending"
          }
          onClick={() => void terminal.submit()}
        >
          Run command
        </button>
      </div>
      {terminal.actionStatus ? (
        <p
          ref={actionStatusRef}
          role={terminal.actionStatus.state === "rejected" ? "alert" : "status"}
          tabIndex={-1}
        >
          {terminal.actionStatus.message}
        </p>
      ) : null}
      {terminal.transcript.length ? (
        <section className="terminal-transcript" aria-label="Current session output">
          <h3>Current session output</h3>
          {terminal.transcript.map((entry, index) => (
            <article key={`${entry.command_id}-${index}`}>
              <code>{entry.command}</code>
              <strong>{entry.classification} / {entry.status}</strong>
              <span>Exit / {entry.exit_code ?? "not run"}</span>
              {entry.stdout || entry.stderr ? (
                <pre aria-label="Command output">{entry.stdout}{entry.stderr}</pre>
              ) : null}
            </article>
          ))}
        </section>
      ) : null}
      {terminal.loadStatus === "pending" ? <p role="status">Loading terminal metadata.</p> : null}
      {terminal.loadStatus === "rejected" ? (
        <p role="alert">{terminal.errorMessage}</p>
      ) : null}
      <section className="terminal-records" aria-label="Command history">
        <h3>Command history</h3>
        {terminal.projection?.commands.length ? (
          terminal.projection.commands.map((command) => (
            <article className="terminal-record" key={command.command_id}>
              <code>{command.command}</code>
              <strong>{command.classification} / {command.status}</strong>
              <span>Working directory / {command.working_directory}</span>
              <span>Access / {command.access_level}</span>
              <span>Requester / {command.requester}</span>
              {command.exit_code === null ? null : <span>Exit / {command.exit_code}</span>}
              {command.requested_paths.length ? (
                <span>Requested paths / {command.requested_paths.join(", ")}</span>
              ) : null}
              {command.status === "pending-approval" ? (
                <div className="terminal-governance">
                  {command.classification === "human-required" ? (
                    <button
                      type="button"
                      aria-label={`Approve ${command.command_id}`}
                      disabled={terminal.actionStatus?.state === "pending"}
                      onClick={() => void terminal.decide(command, "approve")}
                    >
                      Approve
                    </button>
                  ) : (
                    <span>Awaiting Frontier Model approval</span>
                  )}
                  <label>
                    <span>Denial reason</span>
                    <input
                      aria-label={`Denial reason ${command.command_id}`}
                      value={terminal.denialReasons[command.command_id] ?? ""}
                      onChange={(event) => terminal.setDenialReason(command.command_id, event.target.value)}
                    />
                  </label>
                  <button
                    type="button"
                    aria-label={`Deny ${command.command_id}`}
                    className="action--danger"
                    disabled={
                      !terminal.denialReasons[command.command_id]?.trim() ||
                      terminal.actionStatus?.state === "pending"
                    }
                    onClick={() => void terminal.decide(command, "deny")}
                  >
                    Deny
                  </button>
                </div>
              ) : null}
            </article>
          ))
        ) : (
          <p>No persisted command metadata. Output remains local to this application session.</p>
        )}
      </section>
      <section className="path-grants" aria-label="Additional Path Grants">
        <h3>Additional Path Grants</h3>
        <p>Additional authority is requested inline from the Agent Console when a blocked action needs it.</p>
        {terminal.projection?.grants.length ? (
          terminal.projection.grants.map((grant) => {
            const expired = Date.parse(grant.expires_at) <= Date.now();
            return (
              <article className="path-grant" key={grant.grant_id}>
                <code>{grant.path}</code>
                <strong>{grant.access_level} / {grant.duration_seconds} seconds</strong>
                <span>Granted by / Mission Commander</span>
                <span>Granted at / {grant.granted_at}</span>
                <span>{expired ? "Expired" : `Active until ${grant.expires_at}`}</span>
              </article>
            );
          })
        ) : (
          <p>Workspace-contained commands do not need an Additional Path Grant.</p>
        )}
      </section>
    </section>
  );
}
