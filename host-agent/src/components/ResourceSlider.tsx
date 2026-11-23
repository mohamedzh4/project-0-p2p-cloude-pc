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

/** Above this many units the meter groups several units into one block. */
const MAX_SEGMENTS = 16;

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

  const segments = Math.max(1, Math.min(max - min, MAX_SEGMENTS));
  const filled = max > min ? Math.round(((value - min) / (max - min)) * segments) : 0;

  return (
    <div className="field">
      <div className="field-head">
        <label htmlFor={id}>{label}</label>
        <span className="readout">{readout}</span>
      </div>

      <div className="slider-row">
        <div className="meter" data-disabled={disabled}>
          {/* The range input sits on top of the blocks, invisible, so mouse,
              touch and keyboard all drive the real control. */}
          <div className="meter-cells" aria-hidden="true">
            {Array.from({ length: segments }, (_, index) => (
              <span key={index} data-on={index < filled ? "true" : "false"} />
            ))}
          </div>
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
        </div>
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
