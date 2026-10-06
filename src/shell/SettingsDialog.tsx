import {
  BellIcon,
  FolderSimpleIcon,
  GitBranchIcon,
  type Icon,
  InfoIcon,
  KeyboardIcon,
  PaletteIcon,
  RobotIcon,
  TerminalWindowIcon,
  UserCircleIcon,
} from "@phosphor-icons/react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { version as appVersion } from "../../package.json";
import type { AppMode, ProjectScripts, Settings } from "../protocol";
import { COMMANDS } from "../shortcuts";
import { ask, openModal, panelWorktree, scriptsOf, useHive } from "../store";
import { openClaude, showOpenFailure } from "../terminals";
import { transport } from "../transport";
import { NumberInput } from "../ui/NumberInput";
import { Select } from "../ui/Select";
import { openSettingsFile } from "../viewer/external";
import { keyText } from "../window";
import { CloseIcon, ICON } from "./icons";
import { SearchField } from "./SearchField";

const close = () => openModal(null);

/** How long typing in a text or number field must pause before the settings are saved. */
export const SAVE_DELAY_MS = 400;

export const SECTIONS = [
  "Terminal",
  "Appearance",
  "Notifications",
  "Agents",
  "Accounts",
  "Worktrees",
  "Projects",
  "Shortcuts",
  "About",
] as const;
type Section = (typeof SECTIONS)[number];

const SECTION_ICON: Record<Section, Icon> = {
  Terminal: TerminalWindowIcon,
  Appearance: PaletteIcon,
  Notifications: BellIcon,
  Agents: RobotIcon,
  Accounts: UserCircleIcon,
  Worktrees: GitBranchIcon,
  Projects: FolderSimpleIcon,
  Shortcuts: KeyboardIcon,
  About: InfoIcon,
};

/** Changes made while a save waits for its answer, sent together once it arrives. */
let queued: ((next: Settings) => void)[] = [];

/**
 * Sends the whole settings with `change` made to the ones stored (#37: the service checks). One
 * save at a time (9.24): the next one waits for the answer and builds on it, so a quick second
 * save cannot undo the first.
 */
export function saveSettings(change: (next: Settings) => void): void {
  queued.push(change);
  if (!useHive.getState().settingsPending) send();
}

function send(): void {
  const next = structuredClone(useHive.getState().settings);
  for (const change of queued) change(next);
  queued = [];
  useHive.setState({ settingsPending: true });
  void transport.setSettings(next);
}

useHive.subscribe((s, before) => {
  if (before.settingsPending && !s.settingsPending && queued.length > 0) send();
});

/**
 * A text (or number) field saved once typing pauses. It keeps what was typed while the
 * service refuses it (the reason shows at the top) and follows the stored value when it changes.
 */
function Typed(props: {
  id: string;
  value: string;
  onSave: (text: string) => void;
  type?: "text" | "number" | "range" | "multiline";
  min?: number;
  max?: number;
  placeholder?: string;
  "aria-label"?: string;
  /** A number field's name, for its stepper buttons. */
  label?: string;
}) {
  const { value, onSave } = props;
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  useEffect(() => {
    if (draft === value) return;
    const later = setTimeout(() => onSave(draft), SAVE_DELAY_MS);
    return () => clearTimeout(later);
  }, [draft, value, onSave]);
  if (props.type === "multiline") {
    return (
      <textarea
        id={props.id}
        aria-label={props["aria-label"]}
        value={draft}
        placeholder={props.placeholder}
        spellCheck={false}
        rows={3}
        onChange={(e) => setDraft(e.target.value)}
      />
    );
  }
  if (props.type === "number") {
    return (
      <NumberInput
        id={props.id}
        value={draft}
        onChange={setDraft}
        min={props.min ?? 0}
        max={props.max ?? Number.MAX_SAFE_INTEGER}
        label={props.label ?? ""}
      />
    );
  }
  return (
    <input
      id={props.id}
      type={props.type ?? "text"}
      min={props.min}
      max={props.max}
      value={draft}
      placeholder={props.placeholder}
      aria-label={props["aria-label"]}
      spellCheck={false}
      autoComplete="off"
      onChange={(e) => setDraft(e.target.value)}
    />
  );
}

/** A number field for `get`/`set` of the settings; blank or non-numeric text is not sent. */
function NumberSetting(props: {
  id: string;
  label: string;
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
      label={props.label}
      min={props.min}
      max={props.max}
      value={String(value)}
      onSave={(text) => {
        const n = Number(text);
        if (text.trim() !== "" && Number.isInteger(n)) saveSettings((s) => set(s, n));
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
      <input type="checkbox" checked={on} onChange={() => saveSettings((s) => set(s, !on))} />
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
  /** Shown only where the app offers WSL or Windows (12.5.4). */
  modes?: boolean;
  /** Shown only while the service runs natively on Windows (the app's mode, 12.5.4). */
  windows?: boolean;
  control: (id: string, label: string) => ReactNode;
};

const CURSORS = [
  { value: "block", label: "Block" },
  { value: "bar", label: "Bar" },
  { value: "underline", label: "Underline" },
];
const SHELLS = [
  { value: "default", label: "PowerShell" },
  { value: "cmd", label: "Command Prompt" },
  { value: "git_bash", label: "Git Bash" },
];
const THEMES = [
  { value: "one-dark", label: "One Dark" },
  { value: "one-light", label: "One Light" },
];

/** Every setting, in section order; the search box filters them by label. */
const FIELDS: Field[] = [
  {
    section: "Terminal",
    label: "Service",
    help: "Where Hive's service runs your terminals and agents. WSL and Windows each keep their own projects, spaces and settings.",
    modes: true,
    control: (id) => <ServiceMode id={id} />,
  },
  {
    section: "Terminal",
    label: "Font family",
    control: (id) => <FontFamily id={id} />,
  },
  {
    section: "Terminal",
    label: "Font size",
    help: "8 to 32.",
    control: (id, label) => (
      <NumberSetting
        id={id}
        label={label}
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
    control: (id, label) => (
      <NumberSetting
        id={id}
        label={label}
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
  {
    section: "Terminal",
    label: "Shell",
    help: "PowerShell is pwsh when installed, else Windows PowerShell. For new terminals.",
    windows: true,
    control: (id) => <Shell id={id} />,
  },
  { section: "Appearance", label: "Theme", control: (id) => <Theme id={id} /> },
  {
    section: "Notifications",
    label: "Alert volume",
    help: "0 mutes the tone.",
    control: (id, label) => (
      <NumberSetting
        id={id}
        label={label}
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
    control: (id, label) => (
      <NumberSetting
        id={id}
        label={label}
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

const MODES: { value: AppMode; label: string }[] = [
  { value: "wsl", label: "WSL" },
  { value: "native", label: "Windows" },
];

/** Not a service setting: the app keeps it, and switching reconnects (12.5.4). Asked first. */
function ServiceMode({ id }: { id: string }) {
  const value = useHive((s) => s.appMode?.mode ?? "wsl");
  return (
    <Select
      aria-labelledby={`${id}-label`}
      value={value}
      options={MODES}
      onChange={(v) => {
        const mode = MODES.find((m) => m.value === v) as (typeof MODES)[number];
        ask({
          title: `Run Hive on ${mode.label}?`,
          text: `Hive reconnects to its service on ${mode.label}. Every open terminal ends, and the projects, spaces and settings become that service's own.`,
          action: "Switch",
          back: "settings",
          run: () => void transport.setMode(mode.value),
        });
      }}
    />
  );
}

function FontFamily({ id }: { id: string }) {
  const value = useHive((s) => s.settings.terminal.font_family);
  return (
    <Typed
      id={id}
      value={value}
      onSave={(text) =>
        saveSettings((s) => {
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
        saveSettings((s) => {
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
        saveSettings((s) => {
          s.terminal.cursor_style = v as Settings["terminal"]["cursor_style"];
        })
      }
    />
  );
}

function Shell({ id }: { id: string }) {
  const value = useHive((s) => s.settings.terminal.shell);
  return (
    <Select
      aria-labelledby={`${id}-label`}
      value={value}
      options={SHELLS}
      onChange={(v) =>
        saveSettings((s) => {
          s.terminal.shell = v as Settings["terminal"]["shell"];
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
        saveSettings((s) => {
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
        {f.control(id, f.label)}
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

/** Saves `change` made to the scripts of the project `id`. */
function saveScripts(id: string, change: (scripts: ProjectScripts) => void): void {
  saveSettings((s) => {
    const scripts = structuredClone(scriptsOf(s, id));
    change(scripts);
    s.projects[id] = { ...s.projects[id], scripts };
  });
}

/** Blank text is no script. */
const script = (text: string) => (text.trim() === "" ? null : text);

/**
 * Per-project scripts (6.8), the user's own: they live in the settings only, never in the
 * repository. The service checks them.
 */
function ProjectScriptsSection() {
  const projects = Object.values(useHive((s) => s.projects) ?? {});
  const [picked, setPicked] = useState<string | null>(null);
  const id = projects.find((p) => p.id === picked)?.id ?? projects[0]?.id;
  if (id === undefined) return <p className="field-help">Add a project to give it scripts.</p>;
  return (
    <>
      <div className="field">
        <label id="setting-project-label" htmlFor="setting-project">
          Project
        </label>
        <Select
          aria-labelledby="setting-project-label"
          value={id}
          options={projects.map((p) => ({ value: p.id, label: p.name }))}
          onChange={setPicked}
        />
        <p className="field-help">
          Scripts are kept in Hive's settings, never read from the repository. Hive's terminals in a
          worktree have <code>$HIVE_PORT</code> (the first of 10 ports kept for it),{" "}
          <code>$HIVE_WORKTREE_PATH</code> and <code>$HIVE_ROOT_PATH</code>.
        </p>
      </div>
      <ScriptFields key={id} id={id} />
    </>
  );
}

type RunScript = ProjectScripts["run"][number];
type RunRow = RunScript & { key: number };
let runKeys = 0;
const keyed = (run: RunScript[]): RunRow[] => run.map((r) => ({ ...r, key: runKeys++ }));
const sameRuns = (a: RunScript[], b: RunScript[]) =>
  a.length === b.length && a.every((r, i) => r.name === b[i]?.name && r.command === b[i]?.command);

function ScriptFields({ id }: { id: string }) {
  const scripts = useHive((s) => scriptsOf(s.settings, id));
  // On native Windows the archive script runs in the terminals' shell (12.5.6a).
  const shell = useHive((s) =>
    s.appMode?.mode === "native"
      ? SHELLS.find((o) => o.value === s.settings.terminal.shell)?.label
      : undefined,
  );
  // The run scripts as shown, each row with a key of its own (9.24): renaming one keeps its
  // fields (and the focus), and an edit names its row, never a position a removal shifted.
  const [rows, setRows] = useState(() => keyed(scripts.run));
  const shown = useRef(rows);
  // A stored list other than ours, with nothing of ours on the way, is shown as it is. A refused
  // save changes nothing stored: its edit stays shown, like a refused field's text.
  useEffect(() => {
    if (useHive.getState().settingsPending || sameRuns(shown.current, scripts.run)) return;
    shown.current = keyed(scripts.run);
    setRows(shown.current);
  }, [scripts.run]);
  const edit = (change: (rows: RunRow[]) => RunRow[]) => {
    shown.current = change(shown.current);
    setRows(shown.current);
    const run = shown.current.map(({ name, command }) => ({ name, command }));
    saveScripts(id, (s) => {
      s.run = run;
    });
  };
  const editRow = (key: number, change: Partial<RunScript>) =>
    edit((rows) => rows.map((r) => (r.key === key ? { ...r, ...change } : r)));
  const [name, setName] = useState("");
  const [command, setCommand] = useState("");
  const add = () => {
    edit((rows) => [...rows, ...keyed([{ name: name.trim(), command }])]);
    setName("");
    setCommand("");
  };
  return (
    <>
      <div className="field">
        <label htmlFor="setting-setup">Setup script</label>
        <Typed
          id="setting-setup"
          type="multiline"
          value={scripts.setup ?? ""}
          placeholder="bun install"
          onSave={(text) =>
            saveScripts(id, (s) => {
              s.setup = script(text);
            })
          }
        />
        <p className="field-help">Typed into a new terminal in each worktree Hive creates.</p>
      </div>
      <div className="field">
        <span>Run scripts</span>
        {rows.map((run) => (
          <div className="script-run" key={run.key}>
            <Typed
              id={`setting-run-${run.key}`}
              aria-label="Name"
              value={run.name}
              onSave={(text) => editRow(run.key, { name: text.trim() })}
            />
            <Typed
              id={`setting-run-${run.key}-command`}
              aria-label={`Command of ${run.name}`}
              value={run.command}
              onSave={(text) => editRow(run.key, { command: text })}
            />
            <button
              type="button"
              className="ghost"
              title={`Remove ${run.name}`}
              onClick={() => edit((rows) => rows.filter((r) => r.key !== run.key))}
            >
              <CloseIcon />
            </button>
          </div>
        ))}
        <form
          className="script-run"
          onSubmit={(e) => {
            e.preventDefault();
            add();
          }}
        >
          <input
            aria-label="New run script name"
            placeholder="dev"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
          <input
            aria-label="New run script command"
            placeholder="bun run dev --port $HIVE_PORT"
            value={command}
            onChange={(e) => setCommand(e.target.value)}
          />
          <button type="submit" className="secondary" disabled={!name.trim() || !command.trim()}>
            Add
          </button>
        </form>
        <p className="field-help">
          Each is a "Run" item in the worktree's menu: a new terminal with the command typed in.
        </p>
      </div>
      <div className="field">
        <label htmlFor="setting-archive">Archive script</label>
        <Typed
          id="setting-archive"
          type="multiline"
          value={scripts.archive ?? ""}
          placeholder="docker compose down"
          onSave={(text) =>
            saveScripts(id, (s) => {
              s.archive = script(text);
            })
          }
        />
        <p className="field-help">
          Run by the service{" "}
          {shell === undefined ? (
            <>
              with <code>sh -c</code>
            </>
          ) : (
            `in ${shell} (the Shell setting; Command Prompt takes one line only)`
          )}{" "}
          in the worktree before removing it, for at most 60 s. If it fails, the worktree stays
          (unless the removal is forced) and its output is shown.
        </p>
      </div>
    </>
  );
}

/** The default Claude account's name (12.2): no `CLAUDE_CONFIG_DIR`, so `~/.claude`. */
export const DEFAULT_ACCOUNT = "Default";

/**
 * "Log in…" for the account whose folder is `dir` (null: the default one): a new terminal where
 * a new terminal would open (the files panel's worktree) running `claude` as that account, where
 * the user types `/login`. Hive never reads or writes credentials (#45).
 */
function LogIn({ dir, name }: { dir: string | null; name: string }) {
  const where = useHive((s) => panelWorktree(s)?.worktree.path ?? null);
  return (
    <button
      type="button"
      className="secondary"
      disabled={where === null}
      title={
        where === null
          ? "Select a worktree to open the terminal in"
          : `A terminal running claude as ${name}: type /login there`
      }
      onClick={() => {
        close();
        showOpenFailure(openClaude(where as string, "", { config_dir: dir }));
      }}
    >
      Log in…
    </button>
  );
}

/**
 * The Claude accounts (12.2): the default one (`~/.claude`, fixed) and the user's, each a name
 * and a `CLAUDE_CONFIG_DIR`, renamed, removed or added here and picked in the status bar. The
 * service checks them.
 */
function Accounts() {
  const accounts = useHive((s) => s.settings.claude.accounts);
  const [name, setName] = useState("");
  const [dir, setDir] = useState("");
  const add = () => {
    const account = { name: name.trim(), config_dir: dir.trim() };
    saveSettings((s) => {
      s.claude.accounts.push(account);
    });
    setName("");
    setDir("");
  };
  return (
    <div className="field">
      <span>Claude accounts</span>
      <div className="script-run">
        <input aria-label="Name of the default account" value={DEFAULT_ACCOUNT} readOnly />
        <input aria-label="Folder of the default account" value="~/.claude" readOnly />
        <LogIn dir={null} name={DEFAULT_ACCOUNT} />
      </div>
      {accounts.map((a) => (
        <div className="script-run" key={a.config_dir}>
          <Typed
            id={`setting-account-${a.config_dir}`}
            aria-label={`Name of ${a.config_dir}`}
            value={a.name}
            onSave={(text) =>
              saveSettings((s) => {
                const renamed = s.claude.accounts.find((b) => b.config_dir === a.config_dir);
                if (renamed) renamed.name = text.trim();
              })
            }
          />
          <input aria-label={`Folder of ${a.name}`} value={a.config_dir} readOnly />
          <LogIn dir={a.config_dir} name={a.name} />
          <button
            type="button"
            className="ghost"
            title={`Remove ${a.name}`}
            onClick={() =>
              saveSettings((s) => {
                s.claude.accounts = s.claude.accounts.filter((b) => b.config_dir !== a.config_dir);
              })
            }
          >
            <CloseIcon />
          </button>
        </div>
      ))}
      <form
        className="script-run"
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
      >
        <input
          aria-label="New account name"
          placeholder="Work"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <input
          aria-label="New account folder"
          placeholder="/home/you/.claude-work"
          spellCheck={false}
          value={dir}
          onChange={(e) => setDir(e.target.value)}
        />
        <button type="submit" className="secondary" disabled={!name.trim() || !dir.trim()}>
          Add
        </button>
      </form>
      <p className="field-help">
        Each account is a <code>CLAUDE_CONFIG_DIR</code> (an existing folder): its own login,
        settings and sessions. The status bar picks the one new terminals get; open ones keep
        theirs. "Log in…" opens a terminal running <code>claude</code> as the account: type{" "}
        <code>/login</code> there. Hive never reads or writes credentials. Removing an account
        leaves its folder on disk.
      </p>
    </div>
  );
}

function Content({ section, query }: { section: Section; query: string }) {
  const modes = useHive((s) => s.appMode?.wsl === true);
  const windows = useHive((s) => s.appMode?.mode === "native");
  const fields = FIELDS.filter((f) => (modes || !f.modes) && (windows || !f.windows));
  if (query) {
    const found = fields.filter((f) => f.label.toLowerCase().includes(query.toLowerCase()));
    if (found.length === 0) return <p className="field-help">No setting matches.</p>;
    return <Fields fields={found} />;
  }
  if (section === "Shortcuts") return <Shortcuts />;
  if (section === "About") return <About />;
  if (section === "Projects") return <ProjectScriptsSection />;
  if (section === "Accounts") return <Accounts />;
  return <Fields fields={fields.filter((f) => f.section === section)} />;
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
          <SearchField
            aria-label="Search settings"
            placeholder="Search settings"
            value={query}
            onChange={setQuery}
          />
          {SECTIONS.map((name) => {
            const Shape = SECTION_ICON[name];
            return (
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
                <Shape {...ICON} />
                {name}
              </button>
            );
          })}
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
        <button type="button" className="secondary settings-file" onClick={openSettingsFile}>
          Open settings file
        </button>
        <button type="button" className="secondary" onClick={close}>
          Close
        </button>
      </footer>
    </dialog>
  );
}
