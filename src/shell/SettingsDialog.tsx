import { type ReactNode, useEffect, useState } from "react";
import { version as appVersion } from "../../package.json";
import { COMMANDS } from "../shortcuts";
import { openModal, type Settings, useHive } from "../store";
import { transport } from "../transport";
import { Select } from "../ui/Select";
import { keyText } from "../window";
import { CloseIcon } from "./icons";

const close = () => openModal(null);

/** How long typing in a text or number field must pause before the settings are saved. */
export const SAVE_DELAY_MS = 400;

export const SECTIONS = [
  "Terminal",
  "Appearance",
  "Notifications",
  "Agents",
  "Worktrees",
  "Projects",
  "Shortcuts",
  "About",
] as const;
type Section = (typeof SECTIONS)[number];

/** Sends the whole settings with `change` made to the ones in use (#37: the service checks). */
function save(change: (next: Settings) => void): void {
  const next = structuredClone(useHive.getState().settings);
  change(next);
  void transport.setSettings(next);
}

/**
 * A text (or number) field saved once typing pauses. It keeps what was typed while the
 * service refuses it (the reason shows at the top) and follows the stored value when it changes.
 */
function Typed(props: {
  id: string;
  value: string;
  onSave: (text: string) => void;
  type?: "text" | "number" | "range";
  min?: number;
  max?: number;
  placeholder?: string;
}) {
  const { value, onSave } = props;
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  useEffect(() => {
    if (draft === value) return;
    const later = setTimeout(() => onSave(draft), SAVE_DELAY_MS);
    return () => clearTimeout(later);
  }, [draft, value, onSave]);
  return (
    <input
      id={props.id}
      type={props.type ?? "text"}
      min={props.min}
      max={props.max}
      value={draft}
      placeholder={props.placeholder}
      spellCheck={false}
      autoComplete="off"
      onChange={(e) => setDraft(e.target.value)}
    />
  );
}

/** A number field for `get`/`set` of the settings; blank or non-numeric text is not sent. */
function NumberSetting(props: {
  id: string;
  get: (s: Settings) => number;
  set: (s: Settings, n: number) => void;
  min: number;
  max: number;
  type?: "number" | "range";
}) {
  const value = useHive((s) => props.get(s.settings));
  const { set } = props;
  return (
    <Typed
      id={props.id}
      type={props.type ?? "number"}
      min={props.min}
      max={props.max}
      value={String(value)}
      onSave={(text) => {
        const n = Number(text);
        if (text.trim() !== "" && Number.isInteger(n)) save((s) => set(s, n));
      }}
    />
  );
}

function Toggle(props: {
  label: string;
  get: (s: Settings) => boolean;
  set: (s: Settings, on: boolean) => void;
}) {
  const on = useHive((s) => props.get(s.settings));
  const { set } = props;
  return (
    <label className="checkbox">
      <input type="checkbox" checked={on} onChange={(e) => save((s) => set(s, e.target.checked))} />
      {props.label}
    </label>
  );
}

/** A setting; a checkbox (`check`) is its own label, a select is labelled by `<id>-label`. */
type Field = {
  section: Section;
  label: string;
  help?: string;
  check?: boolean;
  control: (id: string) => ReactNode;
};

const CURSORS = [
  { value: "block", label: "Block" },
  { value: "bar", label: "Bar" },
  { value: "underline", label: "Underline" },
];
const THEMES = [
  { value: "one-dark", label: "One Dark" },
  { value: "one-light", label: "One Light" },
];

/** Every setting, in section order; the search box filters them by label. */
const FIELDS: Field[] = [
  {
    section: "Terminal",
    label: "Font family",
    control: (id) => <FontFamily id={id} />,
  },
  {
    section: "Terminal",
    label: "Font size",
    help: "8 to 32.",
    control: (id) => (
      <NumberSetting
        id={id}
        min={8}
        max={32}
        get={(s) => s.terminal.font_size}
        set={(s, n) => {
          s.terminal.font_size = n;
        }}
      />
    ),
  },
  {
    section: "Terminal",
    label: "Scrollback lines",
    help: "1000 to 100000.",
    control: (id) => (
      <NumberSetting
        id={id}
        min={1000}
        max={100000}
        get={(s) => s.terminal.scrollback}
        set={(s, n) => {
          s.terminal.scrollback = n;
        }}
      />
    ),
  },
  {
    section: "Terminal",
    label: "Cursor style",
    control: (id) => <CursorStyle id={id} />,
  },
  {
    section: "Terminal",
    label: "Blinking cursor",
    check: true,
    control: () => (
      <Toggle
        label="Blinking cursor"
        get={(s) => s.terminal.cursor_blink}
        set={(s, on) => {
          s.terminal.cursor_blink = on;
        }}
      />
    ),
  },
  {
    section: "Terminal",
    label: "Copy on select",
    check: true,
    control: () => (
      <Toggle
        label="Copy on select"
        get={(s) => s.terminal.copy_on_select}
        set={(s, on) => {
          s.terminal.copy_on_select = on;
        }}
      />
    ),
  },
  { section: "Appearance", label: "Theme", control: (id) => <Theme id={id} /> },
  {
    section: "Notifications",
    label: "Alert volume",
    help: "0 mutes the tone.",
    control: (id) => (
      <NumberSetting
        id={id}
        type="range"
        min={0}
        max={100}
        get={(s) => s.notifications.volume}
        set={(s, n) => {
          s.notifications.volume = n;
        }}
      />
    ),
  },
  {
    section: "Agents",
    label: "Silence before waiting for you (seconds)",
    help: "2 to 60: how long a working agent's terminal stays quiet before it needs you.",
    control: (id) => (
      <NumberSetting
        id={id}
        min={2}
        max={60}
        get={(s) => s.agents.silence_secs}
        set={(s, n) => {
          s.agents.silence_secs = n;
        }}
      />
    ),
  },
  {
    section: "Agents",
    label: "Confirm closing Hive with agents running",
    check: true,
    control: () => (
      <Toggle
        label="Confirm closing Hive with agents running"
        get={(s) => s.agents.confirm_close}
        set={(s, on) => {
          s.agents.confirm_close = on;
        }}
      />
    ),
  },
  {
    section: "Worktrees",
    label: "Default base branch",
    help: "Empty: the project's current branch.",
    control: (id) => <DefaultBase id={id} />,
  },
];

function FontFamily({ id }: { id: string }) {
  const value = useHive((s) => s.settings.terminal.font_family);
  return (
    <Typed
      id={id}
      value={value}
      onSave={(text) =>
        save((s) => {
          s.terminal.font_family = text;
        })
      }
    />
  );
}

function DefaultBase({ id }: { id: string }) {
  const value = useHive((s) => s.settings.worktrees.default_base);
  return (
    <Typed
      id={id}
      value={value ?? ""}
      placeholder="Current branch"
      onSave={(text) =>
        save((s) => {
          s.worktrees.default_base = text.trim() === "" ? null : text;
        })
      }
    />
  );
}

function CursorStyle({ id }: { id: string }) {
  const value = useHive((s) => s.settings.terminal.cursor_style);
  return (
    <Select
      aria-labelledby={`${id}-label`}
      value={value}
      options={CURSORS}
      onChange={(v) =>
        save((s) => {
          s.terminal.cursor_style = v as Settings["terminal"]["cursor_style"];
        })
      }
    />
  );
}

function Theme({ id }: { id: string }) {
  const value = useHive((s) => s.settings.appearance.theme);
  return (
    <Select
      aria-labelledby={`${id}-label`}
      value={value}
      options={THEMES}
      onChange={(v) =>
        save((s) => {
          s.appearance.theme = v as Settings["appearance"]["theme"];
        })
      }
    />
  );
}

function Fields({ fields }: { fields: Field[] }) {
  return fields.map((f) => {
    const id = `setting-${f.label.toLowerCase().replaceAll(/[^a-z]+/g, "-")}`;
    return (
      <div className="field" key={f.label}>
        {!f.check && (
          <label id={`${id}-label`} htmlFor={id}>
            {f.label}
          </label>
        )}
        {f.control(id)}
        {f.help && <p className="field-help">{f.help}</p>}
      </div>
    );
  });
}

/** Read-only: the command table `shortcut()` runs. */
function Shortcuts() {
  return (
    <table className="settings-keys">
      <tbody>
        {COMMANDS.map((c) => (
          <tr key={c.id}>
            <td>{c.label}</td>
            <td>
              <kbd>{keyText(c.keys)}</kbd>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function About() {
  const connection = useHive((s) => s.connection);
  const diagnostics = useHive((s) => s.diagnostics);
  const unhooked = useHive((s) =>
    Object.values(s.terminals)
      .filter((t) => t.unhooked && !t.exited)
      .map((t) => t.id)
      .join(", "),
  );
  useEffect(() => void transport.getDiagnostics(), []);
  const rows: [string, string][] = [
    ["Hive app", appVersion],
    ["Hive service", connection.status === "connected" ? connection.version : "not connected"],
    ["Settings file", diagnostics?.settings_file ?? "…"],
    ["claude wrapper", diagnostics?.wrapper ?? "…"],
    ["claude it runs", diagnostics ? (diagnostics.claude ?? "not found on PATH") : "…"],
    ["Terminals without hooks", unhooked || "none"],
  ];
  return (
    <dl className="settings-about">
      {rows.map(([name, value]) => (
        <div key={name}>
          <dt>{name}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function Content({ section, query }: { section: Section; query: string }) {
  if (query) {
    const found = FIELDS.filter((f) => f.label.toLowerCase().includes(query.toLowerCase()));
    if (found.length === 0) return <p className="field-help">No setting matches.</p>;
    return <Fields fields={found} />;
  }
  if (section === "Shortcuts") return <Shortcuts />;
  if (section === "About") return <About />;
  if (section === "Projects") return <p className="field-help">No per-project settings yet.</p>;
  return <Fields fields={FIELDS.filter((f) => f.section === section)} />;
}

/**
 * Ctrl+, (6.2): the service's settings by section, or every field matching the search. Each
 * change is sent at once (text and numbers once typing pauses) as the whole settings; the
 * service's refusal shows at the top. Terminals and the theme follow the stored settings.
 */
export function SettingsDialog() {
  const [section, setSection] = useState<Section>("Terminal");
  const [query, setQuery] = useState("");
  const error = useHive((s) => s.settingsError);
  return (
    <dialog
      className="dialog settings"
      aria-labelledby="settings-title"
      ref={(dialog) => {
        if (dialog && !dialog.open) {
          dialog.showModal();
          dialog.querySelector("input")?.focus();
        }
      }}
      onClose={close}
    >
      <header>
        <h2 id="settings-title">Settings</h2>
        <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
          <CloseIcon />
        </button>
      </header>
      <div className="settings-main">
        <nav className="settings-nav">
          <input
            type="search"
            aria-label="Search settings"
            placeholder="Search settings"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          {SECTIONS.map((name) => (
            <button
              type="button"
              className="settings-section"
              key={name}
              aria-current={!query && name === section ? "page" : undefined}
              onClick={() => {
                setSection(name);
                setQuery("");
              }}
            >
              {name}
            </button>
          ))}
        </nav>
        <div className="dialog-body settings-body">
          {error && (
            <span className="field-error" role="alert">
              {error}
            </span>
          )}
          <Content section={section} query={query.trim()} />
        </div>
      </div>
      <footer>
        <button
          type="button"
          className="secondary settings-file"
          onClick={() => void transport.openSettingsFile()}
        >
          Open settings file
        </button>
        <button type="button" className="secondary" onClick={close}>
          Close <kbd>Esc</kbd>
        </button>
      </footer>
    </dialog>
  );
}
