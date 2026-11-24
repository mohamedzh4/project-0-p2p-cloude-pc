import { formatGpuLabel } from "../format";
import type { GpuInfo } from "../types";

interface GpuShareFieldProps {
  gpus: GpuInfo[];
  shareGpu: boolean;
  sharedGpuId: string | null;
  disabled: boolean;
  onShareChange: (share: boolean) => void;
  onGpuChange: (id: string) => void;
}

export function GpuShareField({
  gpus,
  shareGpu,
  sharedGpuId,
  disabled,
  onShareChange,
  onGpuChange,
}: GpuShareFieldProps) {
  // The iGPU keeps the owner's screen; only other cards can be dedicated.
  const dedicatable = gpus.filter((gpu) => !gpu.integrated);
  const selected = dedicatable.find((gpu) => gpu.id === sharedGpuId) ?? dedicatable[0] ?? null;

  return (
    <div className="field">
      <label className="switch" data-disabled={disabled}>
        <input
          type="checkbox"
          checked={shareGpu && !disabled}
          disabled={disabled}
          onChange={(event) => onShareChange(event.target.checked)}
        />
        {disabled
          ? gpus.length > 0
            ? "Dedicate a GPU — unavailable, only an integrated GPU was found"
            : "Dedicate a GPU — unavailable, no GPU detected"
          : "Dedicate a GPU to guest sessions"}
      </label>

      <p className="hint gpu-warning">
        Unlike CPU and RAM, this is not a slice you can spare while you keep using it.
        For the session, that GPU is unavailable to this PC — your display and your own
        apps cannot use it. Keep an iGPU or a second card for yourself, or leave this off.
      </p>

      {dedicatable.length > 1 ? (
        <div className="gpu-picker">
          <label htmlFor="shared-gpu">GPU to dedicate</label>
          <select
            id="shared-gpu"
            value={selected?.id ?? ""}
            disabled={disabled}
            onChange={(event) => onGpuChange(event.target.value)}
          >
            {dedicatable.map((gpu) => (
              <option key={gpu.id} value={gpu.id}>
                {formatGpuLabel(gpu.model, gpu.vramBytes)}
              </option>
            ))}
          </select>
        </div>
      ) : selected ? (
        <p className="hint">
          The GPU you can dedicate: {formatGpuLabel(selected.model, selected.vramBytes)}.
        </p>
      ) : null}
    </div>
  );
}
