import { HardwareCard } from "./HardwareCard";
import type {
  EligibilityReport,
  GpuBindStatus,
  GuestImageStatus,
  HostCapabilities,
  HypervisorStatus,
  LaunchConfig,
} from "../types";

interface CapabilityPanelProps {
  capabilities: HostCapabilities;
  hypervisor: HypervisorStatus;
  guestImage: GuestImageStatus;
  standardLaunch: LaunchConfig | null;
  standardLaunchError: string | null;
  proLaunch: LaunchConfig | null;
  proLaunchError: string | null;
  gpuBind: GpuBindStatus | null;
  offers: EligibilityReport | null;
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

function launchHeadline(config: LaunchConfig | null, error: string | null): { value: string; empty: boolean } {
  if (error) {
    return { value: "Cannot launch", empty: true };
  }
  if (!config) {
    return { value: "Generating…", empty: true };
  }
  return {
    value: `${config.vcpuCount} vCPU · ${config.memoryMib} MiB`,
    empty: false,
  };
}

function gpuBindHeadline(status: GpuBindStatus | null): { value: string; empty: boolean } {
  if (!status) {
    return { value: "Planning…", empty: true };
  }
  switch (status.action) {
    case "planned":
      return {
        value: status.pciAddress ? `VFIO plan · ${status.pciAddress}` : "VFIO plan",
        empty: false,
      };
    case "bound":
      return { value: "Bound to vfio-pci", empty: false };
    case "wouldAssign":
      return { value: "Would assign DDA/GPU-P", empty: true };
    case "skipped":
      return { value: "Not planned", empty: true };
    case "refused":
      return { value: "Not eligible", empty: true };
  }
}

function launchDetail(config: LaunchConfig | null, error: string | null): string {
  if (error) {
    return error;
  }
  if (!config) {
    return "Launch configuration is generated from your sliders. It does not boot a guest.";
  }
  const gpu = config.gpu
    ? `GPU ${config.gpu.id} exclusive`
    : "GPU omitted";
  const rootfs = config.rootfsPath ? ` · ${config.rootfsPath}` : "";
  return `${config.backend} · ${gpu}${rootfs}. Not booted.`;
}

export function CapabilityPanel({
  capabilities,
  hypervisor,
  guestImage,
  standardLaunch,
  standardLaunchError,
  proLaunch,
  proLaunchError,
  gpuBind,
  offers,
}: CapabilityPanelProps) {
  const headline = hypervisorHeadline(hypervisor);
  const imageHeadline = guestImageHeadline(guestImage);
  const standard = launchHeadline(standardLaunch, standardLaunchError);
  const pro = launchHeadline(proLaunch, proLaunchError);
  const bind = gpuBindHeadline(gpuBind);
  const standardOffer = offers?.standard ?? capabilities.standard;
  const proOffer = offers?.pro ?? capabilities.pro;
  const summary = offers?.summary ?? capabilities.summary;

  return (
    <section className="panel reveal" data-reveal={1}>
      <h2>What this PC can host</h2>
      <p className="capability-summary">{summary}</p>
      <div className="cards">
        <HardwareCard
          title="Standard sessions"
          value={standardOffer.offered ? "Available" : "Not available"}
          detail={standardOffer.reason}
          empty={!standardOffer.offered}
        />
        <HardwareCard
          title="Pro sessions"
          value={proOffer.offered ? "Available" : "Not available"}
          detail={proOffer.reason}
          empty={!proOffer.offered}
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
        <HardwareCard
          title="Standard launch config"
          value={standard.value}
          detail={launchDetail(standardLaunch, standardLaunchError)}
          empty={standard.empty}
        />
        <HardwareCard
          title="Pro launch config"
          value={pro.value}
          detail={launchDetail(proLaunch, proLaunchError)}
          empty={pro.empty}
        />
        <HardwareCard
          title="Pro GPU assignment"
          value={bind.value}
          detail={gpuBind?.log ?? "GPU assignment is planned from the dedicated GPU id. It does not bind a live device from this window."}
          empty={bind.empty}
        />
      </div>
      <p className="hint">
        {capabilities.secondDisplayPath
          ? "A second display path was detected (iGPU or more than one GPU), so this PC can keep a screen while a guest uses a dedicated GPU."
          : "No second display path was detected. Pro hosting needs an iGPU or another GPU so you can keep using this PC."}
      </p>
      {proOffer.offered ? (
        <p className="hint gpu-warning">
          When Pro is offered, the dedicated GPU is exclusive for the session — not a concurrent
          slice like CPU or RAM. This PC cannot use that card while a guest is connected.
        </p>
      ) : null}
      {!standardOffer.offered || !proOffer.offered ? (
        <p className="hint">
          A blocked tier is not offered. This agent will not start a session for it, and it will not
          leave a half-built guest.
        </p>
      ) : null}
      <p className="hint">
        The agent locates or fetches the hypervisor and signed guest image for this OS after the
        probe, then maps your CPU/RAM sliders (and a dedicated GPU, when you opt in) into a launch
        document and a VFIO / DDA plan. It computes PCI addresses itself — you do not run lspci.
        It does not boot a guest or bind a live GPU from this window. Standard needs at least 2 vCPU,
        4 GiB RAM, and 50 GiB spare disk.
      </p>
    </section>
  );
}

