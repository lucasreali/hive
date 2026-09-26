import { CaretDownIcon, CaretUpIcon } from "@phosphor-icons/react";

/**
 * A number field with increase/decrease buttons in the app's look, in place of the native spin
 * buttons (hidden in CSS). The buttons and ↑/↓ step by `step` within `min`–`max`; text that is
 * not a number steps from the nearer bound. `value` is the text, so a half-typed number stays.
 */
export function NumberInput(props: {
  id?: string;
  value: string;
  onChange: (text: string) => void;
  min: number;
  max: number;
  step?: number;
  /** What the field sets, for the buttons' names ("Increase …"). */
  label: string;
}) {
  const { value, onChange, min, max } = props;
  const step = props.step ?? 1;
  const n = Number(value);
  const valid = value.trim() !== "" && Number.isFinite(n);
  const by = (dir: 1 | -1) => {
    const from = valid ? n : dir > 0 ? min - step : max + step;
    onChange(String(Math.min(max, Math.max(min, from + dir * step))));
  };
  return (
    <div className="stepper">
      <input
        id={props.id}
        type="number"
        min={min}
        max={max}
        step={step}
        value={value}
        spellCheck={false}
        autoComplete="off"
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
          e.preventDefault();
          by(e.key === "ArrowUp" ? 1 : -1);
        }}
      />
      <div className="stepper-buttons">
        <button
          type="button"
          className="stepper-button"
          tabIndex={-1}
          aria-label={`Increase ${props.label}`}
          disabled={valid && n >= max}
          onClick={() => by(1)}
        >
          <CaretUpIcon size={10} weight="bold" aria-hidden />
        </button>
        <button
          type="button"
          className="stepper-button"
          tabIndex={-1}
          aria-label={`Decrease ${props.label}`}
          disabled={valid && n <= min}
          onClick={() => by(-1)}
        >
          <CaretDownIcon size={10} weight="bold" aria-hidden />
        </button>
      </div>
    </div>
  );
}
