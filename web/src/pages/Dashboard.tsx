import { PixelIcon } from "../components/PixelIcon";

import { SessionPanel } from "../components/SessionPanel";
import { routeHref } from "../router";
import type { Notice, SessionHistoryResponse, SessionResponse } from "../types";

interface DashboardProps {
  session: SessionResponse | null;
  history: SessionHistoryResponse | null;
  notice: Notice | null;
  busy: boolean;
  intent: string;
  onIntent: (value: string) => void;
  onRequest: () => void;
  onEnd: () => void;
}

const STATUS_LABEL: Record<string, string> = {
  active: "Running",
  provisioning: "Starting",
  ended: "Ended",
  interrupted: "Host went offline",
  failed: "Did not start",
};

export function formatDuration(seconds: number | null): string {
  if (seconds == null) return "—";
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min`;
  return `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
}

function formatDate(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

export function Dashboard({ history, ...panel }: DashboardProps) {
  const sessions = history?.sessions ?? [];
  const totalSeconds = sessions.reduce((sum, item) => sum + (item.durationSeconds ?? 0), 0);

  return (
    <>
      <div className="page-head">
        <h1>Sessions</h1>
        <p>Start a computer, open it, and end it when you're done.</p>
      </div>

      <div className="dash-grid">
        <SessionPanel {...panel} />

        <aside className="dash-side">
          <div className="card stat-card">
            <dl className="stats">
              <div>
                <dt>Credits used</dt>
                <dd>{history ? history.creditsDebited : "—"}</dd>
              </div>
              <div>
                <dt>Sessions</dt>
                <dd>{history ? sessions.length : "—"}</dd>
              </div>
              <div>
                <dt>Time on computers</dt>
                <dd>{history ? formatDuration(totalSeconds) : "—"}</dd>
              </div>
            </dl>
            <p className="hint">Credits are kept on the lab's ledger. Nothing is charged to a card.</p>
          </div>
          <a className="card promo-card" href={routeHref("share")}>
            <PixelIcon name="server" size={18} />
            <div>
              <h3>Lend your PC</h3>
              <p>Earn credits while your computer sits idle.</p>
            </div>
          </a>
        </aside>
      </div>

      <section className="card history" aria-labelledby="history-title">
        <h2 id="history-title">History</h2>
        {history === null ? (
          <p className="hint">History isn't available right now. Check that the lab server is reachable.</p>
        ) : sessions.length === 0 ? (
          <p className="hint">No sessions yet. Your first one will show up here.</p>
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th scope="col">Started</th>
                  <th scope="col">Tier</th>
                  <th scope="col">Status</th>
                  <th scope="col" className="num">
                    Length
                  </th>
                  <th scope="col" className="num">
                    Credits
                  </th>
                </tr>
              </thead>
              <tbody>
                {sessions.map((item) => (
                  <tr key={item.sessionId}>
                    <td>{formatDate(item.startedAt)}</td>
                    <td>
                      <span className="tier-dot" data-tier={item.labTier} />
                      {item.labTier === "pro" ? "Pro" : "Standard"}
                    </td>
                    <td>
                      <span className="status-tag" data-status={item.status}>
                        {STATUS_LABEL[item.status] ?? item.status}
                      </span>
                    </td>
                    <td className="num">{formatDuration(item.durationSeconds)}</td>
                    <td className="num">{item.creditsDebited}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </>
  );
}
