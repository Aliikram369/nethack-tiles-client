import type { DisplaySettings } from "../lib/protocol";

/**
 * Tuning for how the game looks, adjustable without leaving it.
 *
 * A tile is drawn into its terminal cell, so the cell *is* the tile: the width
 * and height controls here are what make a square 16x16 tile square, since a
 * monospace cell is normally about half as wide as it is tall.
 */
interface Props {
  settings: DisplaySettings;
  onChange: (settings: DisplaySettings) => void;
  onClose: () => void;
}

/** Families likely to be installed, plus whatever the profile already names. */
const FONTS = [
  "Menlo, DejaVu Sans Mono, Consolas, monospace",
  "SF Mono, Menlo, monospace",
  "Monaco, monospace",
  "Courier New, monospace",
  "Consolas, monospace",
  "monospace",
];

export function DisplayControls({ settings, onChange, onClose }: Props) {
  const set = <K extends keyof DisplaySettings>(
    field: K,
    value: DisplaySettings[K],
  ) => onChange({ ...settings, [field]: value });

  const fonts = FONTS.includes(settings.fontFamily)
    ? FONTS
    : [settings.fontFamily, ...FONTS];

  return (
    <aside className="display-panel" aria-label="Display settings">
      <div className="display-panel__head">
        <h2>Display</h2>
        <button onClick={onClose} aria-label="Close display settings">
          ×
        </button>
      </div>

      <label className="display-field">
        <span>Font</span>
        <select
          value={settings.fontFamily}
          onChange={(e) => set("fontFamily", e.target.value)}
        >
          {fonts.map((font) => (
            <option key={font} value={font}>
              {font.split(",")[0]}
            </option>
          ))}
        </select>
      </label>

      <Slider
        label="Font size"
        value={settings.fontSize}
        min={8}
        max={48}
        step={1}
        unit="px"
        onChange={(v) => set("fontSize", v)}
      />
      <Slider
        label="Cell width"
        hint="Extra pixels per column. Widen until tiles stop looking squashed."
        value={settings.letterSpacing}
        min={0}
        max={24}
        step={0.5}
        unit="px"
        onChange={(v) => set("letterSpacing", v)}
      />
      <Slider
        label="Cell height"
        hint="Row height as a multiple of the font size."
        value={settings.lineHeight}
        min={1}
        max={2.5}
        step={0.05}
        unit="×"
        onChange={(v) => set("lineHeight", v)}
      />

      <label className="display-check">
        <input
          type="checkbox"
          checked={settings.pixelPerfect}
          onChange={(e) => set("pixelPerfect", e.target.checked)}
        />
        <span>
          Whole-pixel tiles
          <small>
            Draws tiles at 1×, 2×, 3× their 16px art instead of stretching.
            Needs a cell at least 16px each way, so raise the size or the cell
            width first.
          </small>
        </span>
      </label>

      <p className="display-panel__note">Saved with this server's profile.</p>
    </aside>
  );
}

function Slider({
  label,
  hint,
  value,
  min,
  max,
  step,
  unit,
  onChange,
}: {
  label: string;
  hint?: string;
  value: number;
  min: number;
  max: number;
  step: number;
  unit: string;
  onChange: (value: number) => void;
}) {
  return (
    <label className="display-field">
      <span>
        {label}
        <output>
          {Math.round(value * 100) / 100}
          {unit}
        </output>
      </span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
      />
      {hint && <small>{hint}</small>}
    </label>
  );
}
