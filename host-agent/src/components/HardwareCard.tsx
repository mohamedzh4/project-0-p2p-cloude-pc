interface HardwareCardProps {
  title: string;
  value: string;
  detail?: string | null;
  /** Renders the value as a muted placeholder, e.g. "No GPU detected". */
  empty?: boolean;
}

export function HardwareCard({ title, value, detail, empty = false }: HardwareCardProps) {
  return (
    <div className="card" data-empty={empty}>
      <h3>{title}</h3>
      <div className="value">{value}</div>
      {detail ? <div className="detail">{detail}</div> : null}
    </div>
  );
}
