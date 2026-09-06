import { HardwareCard } from "./HardwareCard";
import type { GuestImageStatus, HostCapabilities, HypervisorStatus } from "../types";

interface CapabilityPanelProps {
  capabilities: HostCapabilities;
  hypervisor: HypervisorStatus;
  guestImage: GuestImageStatus;
}

function hypervisorHeadline(status: HypervisorStatus): { value: string; empty: boolean } {
  switch (status.action) {
    case "skippedPresent":
    case "fetched":
      return {
        value: status.version ? `${status.hypervisor} ${status.version}` : status.hypervisor,
        empty: false,
      };
    case "wouldFetch":
      return { value: "Would fetch QEMU", empty: true };
    case "skippedOs":
      return { value: "Not fetched", empty: true };
    case "refused":
      return { value: "Not offered", empty: true };
    case "checksumMismatch":
      return { value: "Checksum failed", empty: true };
    case "fetchFailed":
      return { value: "Fetch failed", empty: true };
  }
}

function guestImageHeadline(status: GuestImageStatus): { value: string; empty: boolean } {
  switch (status.action) {
    case "skippedPresent":
    case "fetched":
      return {
        value: status.labTier ? `${status.labTier} (${status.backend})` : status.backend,
        empty: false,
      };
    case "wouldFetch":
      return { value: "Would fetch QEMU image", empty: true };
    case "skippedOs":
      return { value: "Not fetched", empty: true };
    case "refused":
      return { value: "Not offered", empty: true };
    case "rejected":
      return { value: "Signature failed", empty: true };
    case "fetchFailed":
      return { value: "Fetch failed", empty: true };
  }
}

export function CapabilityPanel({ capabilities, hypervisor, guestImage }: CapabilityPanelProps) {
  const headline = hypervisorHeadline(hypervisor);
  const imageHeadline = guestImageHeadline(guestImage);

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
        <HardwareCard
          title="Hypervisor binary"
          value={headline.value}
          detail={hypervisor.log}
          empty={headline.empty}
        />
        <HardwareCard
          title="Guest image"
          value={imageHeadline.value}
          detail={guestImage.log}
          empty={imageHeadline.empty}
        />
      </div>
      <p className="hint">
        {capabilities.secondDisplayPath
          ? "A second display path was detected (iGPU or more than one GPU), so this PC can keep a screen while a guest uses a dedicated GPU."
          : "No second display path was detected. Pro hosting needs an iGPU or another GPU so you can keep using this PC."}
      </p>
      <p className="hint">
        The agent locates or fetches the hypervisor and signed guest image for this OS after the
        probe. It does not boot a guest, and this window does not download anything.
      </p>
    </section>
  );
}
