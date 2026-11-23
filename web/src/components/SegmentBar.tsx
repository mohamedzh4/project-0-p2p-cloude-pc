import type { ReactNode } from "react";

interface SegmentBarProps {
  icon: ReactNode;
  label: string;
  readout: string;
  /** How many of `segments` are filled. */
  filled: number;
  segments?: number;
  tone?: "share" | "rent";
}

/** A read-only share meter: one block per slice of the resource. */
export function SegmentBar({ icon, label, readout, filled, segments = 10, tone = "share" }: SegmentBarProps) {
  return (
    <div className="segbar" data-tone={tone}>
      <div className="segbar-head">
        <span>{label}</span>
        <span className="segbar-readout">{readout}</span>
      </div>
      <div className="segbar-track" role="img" aria-label={`${label}: ${readout}`}>
        <span className="segbar-icon" aria-hidden="true">
          {icon}
        </span>
        {Array.from({ length: segments }, (_, index) => (
          <span key={index} className="segbar-cell" data-on={index < filled ? "true" : "false"} />
        ))}
      </div>
    </div>
  );
}
