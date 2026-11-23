import { useState, type PointerEvent, type ReactNode } from "react";

interface SegmentBarProps {
  icon: ReactNode;
  label: string;
  readout: string;
  /** How many of `segments` are filled. */
  filled: number;
  segments?: number;
  tone?: "share" | "rent";
  /**
   * When set, the meter follows the mouse: hovering fills it up to the
   * pointer and this turns the filled count into the reading shown.
   */
  preview?: (filled: number) => string;
}

/** A share meter: one square block per slice of the resource. */
export function SegmentBar({
  icon,
  label,
  readout,
  filled,
  segments = 10,
  tone = "share",
  preview,
}: SegmentBarProps) {
  const [hover, setHover] = useState<number | null>(null);
  const shown = hover ?? filled;

  function track(event: PointerEvent<HTMLDivElement>) {
    if (!preview || event.pointerType === "touch") return;
    const cells = event.currentTarget.querySelectorAll<HTMLElement>(".segbar-cell");
    let count = 0;
    cells.forEach((cell, index) => {
      if (event.clientX >= cell.getBoundingClientRect().left) count = index + 1;
    });
    setHover(count);
  }

  return (
    <div className="segbar" data-tone={tone} data-interactive={preview ? "true" : "false"}>
      <div className="segbar-head">
        <span>{label}</span>
        <span className="segbar-readout">{hover != null && preview ? preview(hover) : readout}</span>
      </div>
      <div
        className="segbar-track"
        role="img"
        aria-label={`${label}: ${readout}`}
        onPointerMove={track}
        onPointerLeave={() => setHover(null)}
      >
        <span className="segbar-icon" aria-hidden="true">
          {icon}
        </span>
        {Array.from({ length: segments }, (_, index) => (
          <span key={index} className="segbar-cell" data-on={index < shown ? "true" : "false"} />
        ))}
      </div>
    </div>
  );
}
