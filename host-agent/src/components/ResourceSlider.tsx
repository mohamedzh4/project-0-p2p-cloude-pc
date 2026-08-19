interface ResourceSliderProps {
  id: string;
  label: string;
  readout: string;
  min: number;
  max: number;
  step?: number;
  value: number;
  hint?: string;
  disabled?: boolean;
  onChange: (value: number) => void;
}

export function ResourceSlider({
  id,
  label,
  readout,
  min,
  max,
  step = 1,
  value,
  hint,
  disabled = false,
  onChange,
}: ResourceSliderProps) {
  const clamp = (raw: number) => {
    if (Number.isNaN(raw)) {
      return min;
    }

    return Math.min(Math.max(raw, min), max);
  };

  return (
    <div className="field">
      <div className="field-head">
        <label htmlFor={id}>{label}</label>
        <span className="readout">{readout}</span>
      </div>

      <div className="slider-row">
        <input
          id={id}
          type="range"
          min={min}
          max={max}
          step={step}
          value={value}
          disabled={disabled}
          onChange={(event) => onChange(clamp(Number(event.target.value)))}
        />
        <input
          type="number"
          aria-label={`${label} (exact value)`}
          min={min}
          max={max}
          step={step}
          value={value}
          disabled={disabled}
          onChange={(event) => onChange(clamp(Number(event.target.value)))}
        />
      </div>

      {hint ? <p className="hint">{hint}</p> : null}
    </div>
  );
}
