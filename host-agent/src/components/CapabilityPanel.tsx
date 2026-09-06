import { HardwareCard } from "./HardwareCard";
import type { HostCapabilities } from "../types";

interface CapabilityPanelProps {
  capabilities: HostCapabilities;
}

export function CapabilityPanel({ capabilities }: CapabilityPanelProps) {
  return (
    <section className="panel reveal" data-reveal={1}>
      <h2>What this PC can host</h2>
      <p className="capability-summary">{capabilities.summary}</p>
      <div className="cards">
        <HardwareCard
          title="Standard sessions"
          value={capabilities.standard.offered ? "Available" : "Not available"}
          detail={capabilities.standard.reason}
          empty={!capabilities.standard.offered}
        />
        <HardwareCard
          title="Pro sessions"
          value={capabilities.pro.offered ? "Available" : "Not available"}
          detail={capabilities.pro.reason}
          empty={!capabilities.pro.offered}
        />
      </div>
      <p className="hint">
        {capabilities.secondDisplayPath
          ? "A second display path was detected (iGPU or more than one GPU), so this PC can keep a screen while a guest uses a dedicated GPU."
          : "No second display path was detected. Pro hosting needs an iGPU or another GPU so you can keep using this PC."}
      </p>
    </section>
  );
}
