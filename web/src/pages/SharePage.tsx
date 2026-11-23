import type { HostKitOffer, HostKitOs } from "../api";
import { SharePcPanel } from "../components/SharePcPanel";

interface SharePageProps {
  kits: HostKitOffer[] | null;
  busy: HostKitOs | null;
  message: string | null;
  onDownload: (os: HostKitOs) => void;
}

export function SharePage(props: SharePageProps) {
  return (
    <>
      <div className="page-head">
        <h1>Share my PC</h1>
        <p>
          Lend your computer's spare CPU and memory and earn credits. The setup is made for your
          account; do not send it to someone else.
        </p>
      </div>
      <SharePcPanel {...props} />
      <ol className="card share-flow">
        <li>
          <h3>Install</h3>
          <p>Run the setup. It installs Host Agent and the desktop disk sessions boot from.</p>
        </li>
        <li>
          <h3>Choose what to lend</h3>
          <p>Pick cores, memory and, if you like, your graphics card. You can change it any time.</p>
        </li>
        <li>
          <h3>Wait for approval</h3>
          <p>Your PC is offered to renters once the operator approves it.</p>
        </li>
      </ol>
    </>
  );
}
