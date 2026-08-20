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
  const selected = gpus.find((gpu) => gpu.id === sharedGpuId) ?? gpus[0] ?? null;

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
          ? "Dedicate a GPU — unavailable, no GPU detected"
          : "Dedicate a GPU to guest sessions"}
      </label>

      <p className="hint gpu-warning">
        Unlike CPU and RAM, this is not a slice you can spare while you keep using it.
        For the session, that GPU is unavailable to this PC — your display and your own
        apps cannot use it. Keep an iGPU or a second card for yourself, or leave this off.
      </p>

      {gpus.length > 1 ? (
        <div className="gpu-picker">
          <label htmlFor="shared-gpu">GPU to dedicate</label>
          <select
            id="shared-gpu"
            value={selected?.id ?? ""}
            disabled={disabled}
            onChange={(event) => onGpuChange(event.target.value)}
          >
            {gpus.map((gpu) => (
              <option key={gpu.id} value={gpu.id}>
                {formatGpuLabel(gpu.model, gpu.vramBytes)}
              </option>
            ))}
          </select>
        </div>
      ) : selected ? (
        <p className="hint">
          This machine has one supported GPU: {formatGpuLabel(selected.model, selected.vramBytes)}.
        </p>
      ) : null}
    </div>
  );
}
