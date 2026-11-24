import { PixelIcon, type PixelIconName } from "./PixelIcon";

interface HardwareCardProps {
  title: string;
  value: string;
  detail?: string | null;
  icon?: PixelIconName;
  /** Renders the value as a muted placeholder, e.g. "No GPU detected". */
  empty?: boolean;
}

export function HardwareCard({ title, value, detail, icon, empty = false }: HardwareCardProps) {
  return (
    <div className="card" data-empty={empty}>
      {icon ? <PixelIcon name={icon} size={18} className="card-icon" /> : null}
      <h3>{title}</h3>
      <div className="value">{value}</div>
      {detail ? <div className="detail">{detail}</div> : null}
    </div>
  );
}
