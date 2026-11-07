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
import { type CSSProperties, Fragment, type ReactNode, useEffect, useRef, useState } from "react";
import { version as appVersion } from "../../package.json";
import type { Account, AppMode, ClaudeLogin, ProjectScripts, Settings } from "../protocol";
import { commandKeys, platformCommands } from "../shortcuts";
import { ask, openModal, panelWorktree, scriptsOf, useHive } from "../store";
import { MAC_TERMINAL_KEYS, openClaude, showOpenFailure } from "../terminals";
import { transport } from "../transport";
import { NumberInput } from "../ui/NumberInput";
import { Select } from "../ui/Select";
import { openSettingsFile, openSharedFolder } from "../viewer/external";
import { homeText, isMac } from "../window";
import { FolderField, useFolderListing } from "./FolderField";
import { CloseIcon, ICON } from "./icons";
import { SearchField } from "./SearchField";
import { accountName, useGhAccounts } from "./SpaceDialog";

const close = () => openModal(null);

/** How long typing in a text or number field must pause before the settings are saved. */
export const SAVE_DELAY_MS = 400;
/** The default account's name when it is not renamed (21.4). */
const DEFAULT_ACCOUNT = "Default";

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

/**
 * Changes made while a save waits for its answer, sent together once it arrives, and requests
 * answered by the settings too (22.3), each sent alone in its turn.
 */
const queued: (((next: Settings) => void) | { request: () => Promise<void> })[] = [];

/**
 * Sends the whole settings with `change` made to the ones stored (#37: the service checks). One
 * save at a time (9.24): the next one waits for the answer and builds on it, so a quick second
 * save cannot undo the first.
 */
export function saveSettings(change: (next: Settings) => void): void {
  queued.push(change);
  if (!useHive.getState().settingsPending) send();
}

/**
 * Sends `request`, answered by `settings` or `settings_failed` (22.3: an account made or
 * removed by the service), in turn with the saves: a save after it builds on its answer.
 */
export function requestSettings(request: () => Promise<void>): void {
  queued.push({ request });
  if (!useHive.getState().settingsPending) send();
}

function send(): void {
  useHive.setState({ settingsPending: true });
  const first = queued[0];
  if (typeof first === "object") {
    queued.shift();
    void first.request();
    return;
  }
  const next = structuredClone(useHive.getState().settings);
  while (typeof queued[0] === "function") (queued.shift() as (next: Settings) => void)(next);
  void transport.setSettings(next);
}

useHive.subscribe((s, before) => {
  if (before.settingsPending && !s.settingsPending && queued.length > 0) send();
});

/**
 * A text (or number) field saved once typing pauses. It keeps what was typed while the
 * service refuses it (the reason shows at the top) and follows the stored value when it changes,
 * except while focused: the service's answer never overwrites the text being typed (22.4).
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
  useEffect(() => {
    if (document.activeElement?.id !== props.id) setDraft(value);
  }, [value, props.id]);
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
  /** The rest of the help, in its tooltip (22.6: the help itself is one line). */
  tip?: string;
  check?: boolean;
  /** Shown only where the app offers WSL or Windows (12.5.4). */
  modes?: boolean;
  /** Shown only while the service runs natively on Windows (the app's mode, 12.5.4). */
  windows?: boolean;
  /** Shown only on macOS (the Dock, 17.16; Option as Meta, 17.14; the power assertion, 17.17). */
  mac?: boolean;
  control: (id: string, label: string) => ReactNode;
};

const CURSORS = [
  { value: "block", label: "Block" },
  { value: "bar", label: "Bar" },
  { value: "underline", label: "Underline" },
];
const OPTION_SIDES = [
  { value: "off", label: "Off" },
  { value: "left", label: "Left Option" },
  { value: "right", label: "Right Option" },
  { value: "both", label: "Both" },
];
const SHELLS = [
  { value: "default", label: "PowerShell" },
  { value: "cmd", label: "Command Prompt" },
  { value: "git_bash", label: "Git Bash" },
];
const THEMES = [
  { value: "one-dark", label: "One Dark" },
  { value: "one-light", label: "One Light" },
  { value: "system", label: "System" },
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
    section: "Accounts",
    label: "Share sessions between accounts",
    check: true,
    help: "A resumed session runs as the space's account; its log moves into that account's folder.",
    tip:
      "Resuming a session from the history runs it as the current space's account, not the " +
      "one it ran as: its log moves into that account's folder (rewind checkpoints from before " +
      "the move are lost). Off, a session resumes as its own account.",
    control: () => (
      <Toggle
        label="Share sessions between accounts"
        get={(s) => s.claude.share_sessions}
        set={(s, on) => {
          s.claude.share_sessions = on;
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
    label: "Option as Meta",
    help: "This Option key + a key sends Esc and the key (Claude Code's Option+P and Option+T need it) instead of typing accents and symbols; the other Option still types them.",
    mac: true,
    control: (id) => <OptionAsMeta id={id} />,
  },
  {
    section: "Terminal",
    label: "Screen reader",
    check: true,
    help: "Lets VoiceOver and other screen readers read the terminal's text.",
    control: () => (
      <Toggle
        label="Screen reader"
        get={(s) => s.terminal.screen_reader}
        set={(s, on) => {
          s.terminal.screen_reader = on;
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
    section: "Notifications",
    label: "Bounce the Dock icon when an agent needs an action",
    help: "Once, while Hive is not focused: permission, plan, question or error.",
    check: true,
    mac: true,
    control: () => (
      <Toggle
        label="Bounce the Dock icon when an agent needs an action"
        get={(s) => s.notifications.bounce}
        set={(s, on) => {
          s.notifications.bounce = on;
        }}
      />
    ),
  },
  {
    section: "Notifications",
    label: "Show agents in the menu bar",
    help: "An icon in the color of the most urgent state; its menu lists the agents that need you.",
    check: true,
    mac: true,
    control: () => (
      <Toggle
        label="Show agents in the menu bar"
        get={(s) => s.notifications.menu_bar}
        set={(s, on) => {
          s.notifications.menu_bar = on;
        }}
      />
    ),
  },
  {
    section: "Notifications",
    label: "Long commands (seconds)",
    help:
      "0 to 3600, 0 never: a command that ran this long notifies when it ends, in a terminal " +
      "with no agent and not in view. " +
      "Needs a shell that marks its commands (OSC 133), as fish 4 does and zsh does in Hive.",
    control: (id, label) => (
      <NumberSetting
        id={id}
        label={label}
        min={0}
        max={3600}
        get={(s) => s.notifications.command_secs}
        set={(s, n) => {
          s.notifications.command_secs = n;
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
    section: "Agents",
    label: "Keep the Mac awake while agents work",
    help: "While an agent is working or has subagents at work, the Mac does not sleep when idle. The display still sleeps, and closing the lid on battery still sleeps the Mac.",
    check: true,
    mac: true,
    control: () => (
      <Toggle
        label="Keep the Mac awake while agents work"
        get={(s) => s.agents.keep_awake}
        set={(s, on) => {
          s.agents.keep_awake = on;
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

function OptionAsMeta({ id }: { id: string }) {
  const value = useHive((s) => s.settings.terminal.option_as_meta);
  return (
    <Select
      aria-labelledby={`${id}-label`}
      value={value}
      options={OPTION_SIDES}
      onChange={(v) =>
        saveSettings((s) => {
          s.terminal.option_as_meta = v as Settings["terminal"]["option_as_meta"];
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
        {f.help && (
          <p className="field-help" title={f.tip}>
            {f.help}
          </p>
        )}
      </div>
    );
  });
}

/** Read-only: the command table `shortcut()` runs and, on macOS, the terminal's own keys. */
function Shortcuts() {
  const rows = [
    ...platformCommands().map((c) => ({ id: c.id, label: c.label, keys: commandKeys(c) })),
    ...(isMac() ? MAC_TERMINAL_KEYS.map((k) => ({ ...k, id: k.keys })) : []),
  ];
  return (
    <table className="settings-keys">
      <tbody>
        {rows.map((row) => (
          <tr key={row.id}>
            <td>{row.label}</td>
            <td>
              <kbd>{row.keys}</kbd>
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
    ...(diagnostics?.git_error ? [["git", diagnostics.git_error] as [string, string]] : []),
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
  // Opened for a project (the Scripts tab's link, 23.4): that one first.
  const [picked, setPicked] = useState<string | null>(() => useHive.getState().modalProject);
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
          These scripts are kept in Hive's settings. Run scripts also come from the repository's{" "}
          <code>package.json</code>, <code>Taskfile.yml</code> and <code>Makefile</code> (the
          Scripts tab); one here with the same name replaces it. Setup and archive come only from
          here, and nothing runs until you click it. Hive's terminals in a worktree have{" "}
          <code>$HIVE_PORT</code> (the first of 10 ports kept for it),{" "}
          <code>$HIVE_WORKTREE_PATH</code> and <code>$HIVE_ROOT_PATH</code>. They also have that
          port as <code>$PORT</code> (unless your shell already sets one), which most dev servers
          read; Vite needs <code>server.port: Number(process.env.PORT)</code> in its config.
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

/**
 * Whether the account whose folder is `dir` (null: the default one) has logged in (21.6): yes
 * until the service says otherwise.
 */
export const loggedIn = (logins: ClaudeLogin[] | null, dir: string | null) =>
  logins?.find((l) => l.config_dir === dir)?.logged_in ?? true;

/**
 * Asks the service whether each Claude account has logged in (21.6): once connected, whenever
 * the accounts change, and whenever the window comes back to the front (a `/login` finishes in
 * the browser).
 */
export function useClaudeLogins() {
  const connected = useHive((s) => s.connection.status === "connected");
  const accounts = useHive((s) => s.settings.claude.accounts);
  // biome-ignore lint/correctness/useExhaustiveDependencies: `accounts` says when to ask again.
  useEffect(() => {
    if (!connected) return;
    const ask = () => void transport.listClaudeLogins();
    ask();
    window.addEventListener("focus", ask);
    return () => window.removeEventListener("focus", ask);
  }, [connected, accounts]);
}

/**
 * "Not logged in" (21.6) as a badge in its own column (22.6), for an account that never finished
 * its `/login`; an empty cell when it did, so the row's actions stay in place.
 */
export function NotLoggedIn({ dir }: { dir: string | null }) {
  const logins = useHive((s) => s.claudeLogins);
  return (
    <span className="account-status">
      {loggedIn(logins, dir) ? null : <span className="tab-badge warn">Not logged in</span>}
    </span>
  );
}

/**
 * Opens a new terminal where a new terminal would open (the files panel's worktree) running
 * `claude` as the account whose folder is `dir` (null: the default one), where the user types
 * `/login`; null while no worktree is selected. Hive never reads or writes credentials (#45).
 */
export function useLogIn(dir: string | null): (() => void) | null {
  const where = useHive((s) => panelWorktree(s)?.worktree.path ?? null);
  if (where === null) return null;
  return () => {
    close();
    showOpenFailure(openClaude(where, "", { config_dir: dir }));
  };
}

/** "Log in…" for the account whose folder is `dir` (null: the default one), `useLogIn`. */
export function LogIn({ dir, name }: { dir: string | null; name: string }) {
  const logIn = useLogIn(dir);
  return (
    <button
      type="button"
      className="secondary"
      disabled={logIn === null}
      title={
        logIn === null
          ? "Select a worktree to open the terminal in"
          : `A terminal running claude as ${name}: type /login there`
      }
      onClick={logIn ?? undefined}
    >
      Log in…
    </button>
  );
}

/**
 * A folder in a fixed-width cell (22.6): the start of the path gives way first, so its last
 * segment (`.claude-work`) stays readable; the whole path in the tooltip.
 */
function FolderCell({ path }: { path: string }) {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  const tail = { "--tail": path.length - cut } as CSSProperties;
  return (
    <span className="folder-cell" title={path} style={tail}>
      <span className="folder-head">{path.slice(0, cut)}</span>
      <span className="folder-tail">{path.slice(cut)}</span>
    </span>
  );
}

/**
 * The Claude accounts (12.2): the default one (`~/.claude`, renamed here but never removed,
 * 21.4) and the user's, each a name and a `CLAUDE_CONFIG_DIR`, renamed, removed or added here
 * and picked in each space's settings. The service checks them. One row per account in fixed
 * columns (22.6): name, folder, status, "Log in…", remove (an empty slot on the default row).
 */
function Accounts() {
  const accounts = useHive((s) => s.settings.claude.accounts);
  const defaultName = useHive((s) => s.settings.claude.default_name);
  // Asked again when the section opens: a `/login` may have finished in a Hive terminal.
  useEffect(() => void transport.listClaudeLogins(), []);
  // The 22.3 form, opened by "New account".
  const [adding, setAdding] = useState(false);
  const nameField = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (adding) nameField.current?.focus();
  }, [adding]);
  const [name, setName] = useState("");
  // An existing folder chosen (as before 22.3), else the service makes one.
  const [existing, setExisting] = useState(false);
  const [dir, setDir] = useState("");
  const listing = useFolderListing(dir);
  // The account just made (22.3), offered its log in.
  const [made, setMade] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [deleteFolder, setDeleteFolder] = useState(false);
  const fresh = accounts.find((a) => a.hive_folder && a.name === made);
  const add = () => {
    const trimmed = name.trim();
    if (existing) {
      const account = { name: trimmed, config_dir: dir.trim() };
      saveSettings((s) => {
        s.claude.accounts.push(account);
      });
    } else {
      requestSettings(() => transport.createAccount(trimmed));
      setMade(trimmed);
    }
    setName("");
    setDir("");
    setAdding(false);
  };
  const remove = (a: Account) => {
    // Only a folder Hive made may be deleted (22.3): it asks first.
    if (a.hive_folder) {
      setRemoving(a.config_dir);
      setDeleteFolder(false);
      return;
    }
    saveSettings((s) => {
      s.claude.accounts = s.claude.accounts.filter((b) => b.config_dir !== a.config_dir);
    });
  };
  return (
    <section className="field settings-group" aria-labelledby="accounts-title">
      <h3 id="accounts-title">Claude accounts</h3>
      <p
        className="field-help"
        title={
          "The first one is ~/.claude itself: it can be renamed but not removed, and an empty " +
          'name is "Default" again. An account a space uses cannot be removed; removing one ' +
          "leaves a folder you chose on disk, and one Hive made may be deleted with it. Hive " +
          "never reads or writes credentials."
        }
      >
        Each account is a <code>CLAUDE_CONFIG_DIR</code> with its own login, settings and sessions;
        each space picks one.
      </p>
      <div className="accounts">
        <div className="account-row">
          <Typed
            id="setting-account-default"
            aria-label="Name of the default account"
            // An empty field is the default name (22.4): the service settles "" to "Default".
            placeholder={DEFAULT_ACCOUNT}
            value={defaultName === DEFAULT_ACCOUNT ? "" : defaultName}
            onSave={(text) => {
              if ((text.trim() || DEFAULT_ACCOUNT) === defaultName) return;
              saveSettings((s) => {
                s.claude.default_name = text.trim();
              });
            }}
          />
          <FolderCell path="~/.claude" />
          <NotLoggedIn dir={null} />
          <LogIn dir={null} name={defaultName} />
          {/* The default account cannot be removed: its slot stays empty. */}
          <span />
        </div>
        {accounts.map((a) => (
          <Fragment key={a.config_dir}>
            <div className="account-row">
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
              <FolderCell path={a.config_dir} />
              <NotLoggedIn dir={a.config_dir} />
              <LogIn dir={a.config_dir} name={a.name} />
              <button
                type="button"
                className="ghost"
                title={`Remove ${a.name}`}
                onClick={() => remove(a)}
              >
                <CloseIcon />
              </button>
            </div>
            {removing === a.config_dir && (
              <fieldset className="new-account account-wide" aria-label={`Remove ${a.name}`}>
                <p className="field-help">
                  Hive made its folder, <code>{a.config_dir}</code>: deleting it loses the account's
                  sessions, settings and skills. Run <code>/logout</code> in its terminal first:
                  Claude Code keeps its credentials outside the folder (on macOS, in the Keychain),
                  and Hive never touches them.
                </p>
                <label className="checkbox">
                  <input
                    type="checkbox"
                    checked={deleteFolder}
                    onChange={(e) => setDeleteFolder(e.target.checked)}
                  />
                  Also delete its folder
                </label>
                <div className="script-run">
                  <button type="button" className="secondary" onClick={() => setRemoving(null)}>
                    Cancel
                  </button>
                  <button
                    type="button"
                    className="primary danger"
                    onClick={() => {
                      setRemoving(null);
                      requestSettings(() => transport.removeAccount(a.config_dir, deleteFolder));
                    }}
                  >
                    Remove
                  </button>
                </div>
              </fieldset>
            )}
          </Fragment>
        ))}
      </div>
      {fresh && (
        // Its row above has the "Log in…" (22.6).
        <p className="field-help" role="status">
          {fresh.name} is made: log in to it.
        </p>
      )}
      {adding ? (
        <form
          className="new-account"
          aria-label="New account"
          onSubmit={(e) => {
            e.preventDefault();
            add();
          }}
        >
          <input
            aria-label="New account name"
            placeholder="Name, e.g. Work"
            value={name}
            ref={nameField}
            onChange={(e) => setName(e.target.value)}
          />
          <label className="checkbox">
            <input
              type="checkbox"
              checked={existing}
              onChange={(e) => setExisting(e.target.checked)}
            />
            Use an existing folder
          </label>
          {existing ? (
            <>
              {/* Starts at the home folder; account folders are often hidden ones (22.2). */}
              <FolderField
                id="new-account-folder"
                aria-label="New account folder"
                value={dir}
                onChange={setDir}
                placeholder={homeText("/home/you/.claude-work")}
                hidden
                home
              />
              {listing?.path === dir && listing.claude && (
                <p className="field-help" role="status">
                  A Claude config folder: it has a <code>.claude.json</code> or{" "}
                  <code>settings.json</code>.
                </p>
              )}
            </>
          ) : (
            <p className="field-help">
              Hive makes its folder, in <code>accounts</code> in Hive's folder (<code>~/.hive</code>
              ).
            </p>
          )}
          <div className="form-buttons">
            <button
              type="button"
              className="secondary"
              onClick={() => {
                setAdding(false);
                setName("");
                setDir("");
              }}
            >
              Cancel
            </button>
            <button
              type="submit"
              className="primary"
              disabled={!name.trim() || (existing && !dir.trim())}
            >
              Add
            </button>
          </div>
        </form>
      ) : (
        <div>
          <button type="button" className="secondary" onClick={() => setAdding(true)}>
            New account
          </button>
        </div>
      )}
    </section>
  );
}

/**
 * The GitHub accounts (24.13): those logged in to `gh` in its own config (`~/.config/gh`, never
 * removed) and in each gh config folder added here, one row per folder in the Claude accounts'
 * columns (22.6): its logins, the folder, why none is listed, remove (an empty slot on gh's
 * own). Each space picks one of them in its settings; the service refuses to remove a folder a
 * space's account is in.
 */
function GhAccountsPart() {
  const folders = useGhAccounts();
  const [adding, setAdding] = useState(false);
  const [dir, setDir] = useState("");
  const add = () => {
    const folder = dir.trim();
    saveSettings((s) => {
      s.gh.config_dirs.push(folder);
    });
    setDir("");
    setAdding(false);
  };
  const remove = (folder: string) =>
    saveSettings((s) => {
      s.gh.config_dirs = s.gh.config_dirs.filter((d) => d !== folder);
    });
  return (
    <section className="field settings-group" aria-labelledby="gh-accounts-title">
      <h3 id="gh-accounts-title">GitHub accounts</h3>
      <p
        className="field-help"
        title={
          "The first one is gh's own config. A space whose account is in another folder gets " +
          "it as GH_CONFIG_DIR; a folder a space's account is in cannot be removed. Run " +
          "gh auth login with GH_CONFIG_DIR set to add an account to a folder. Hive never " +
          "reads a token into the app."
        }
      >
        Each gh config folder is a <code>GH_CONFIG_DIR</code> with its own logins; each space picks
        an account.
      </p>
      <div className="accounts">
        {folders.map(({ dir, listed }) => {
          const logins = (listed?.accounts ?? []).map(accountName).join(", ");
          return (
            <div className="account-row" key={dir ?? ""}>
              <span className="gh-logins" title={logins}>
                {listed ? logins || "No account" : "…"}
              </span>
              <FolderCell path={dir ?? "~/.config/gh"} />
              <span className="account-status">
                {listed?.problem && (
                  <span className="tab-badge warn" title={listed.problem}>
                    {listed.accounts.length ? "Incomplete" : "Not logged in"}
                  </span>
                )}
              </span>
              <span />
              {dir === null ? (
                // gh's own config cannot be removed: its slot stays empty.
                <span />
              ) : (
                <button
                  type="button"
                  className="ghost"
                  title={`Remove ${dir}`}
                  onClick={() => remove(dir)}
                >
                  <CloseIcon />
                </button>
              )}
            </div>
          );
        })}
      </div>
      {adding ? (
        <form
          className="new-account"
          aria-label="Add a gh config folder"
          onSubmit={(e) => {
            e.preventDefault();
            add();
          }}
        >
          {/* Starts at the home folder; gh config folders are often hidden ones (22.2). */}
          <FolderField
            id="new-gh-folder"
            aria-label="New gh config folder"
            value={dir}
            onChange={setDir}
            placeholder={homeText("/home/you/.config/gh-work")}
            hidden
            home
          />
          <div className="form-buttons">
            <button
              type="button"
              className="secondary"
              onClick={() => {
                setAdding(false);
                setDir("");
              }}
            >
              Cancel
            </button>
            <button type="submit" className="primary" disabled={!dir.trim()}>
              Add
            </button>
          </div>
        </form>
      ) : (
        <div>
          <button type="button" className="secondary" onClick={() => setAdding(true)}>
            Add a gh config folder
          </button>
        </div>
      )}
    </section>
  );
}

/**
 * The folder every account shares (21.9), a titled group (22.6): its path, a button that opens
 * it in the file manager, and what the service found in it on one line, asked when the section
 * opens.
 */
function SharedPlugin() {
  const shared = useHive((s) => s.shared);
  useEffect(() => void transport.getShared(), []);
  const named = (names: string[]) => names.map((n) => `/${shared?.plugin}:${n}`);
  const held = shared
    ? (
        [
          ["Skills", named(shared.skills)],
          ["Commands", named(shared.commands)],
          ["Agents", shared.agents.map((n) => `${shared.plugin}:${n}`)],
          ["MCP servers", shared.mcp_servers],
        ] as const
      ).filter(([, names]) => names.length)
    : [];
  return (
    <section className="field settings-group" aria-labelledby="shared-title">
      <h3 id="shared-title">Shared</h3>
      <p
        className="field-help"
        title={
          "Skills (skills/<name>/SKILL.md), agents (agents/), commands (commands/) and MCP " +
          "servers (.mcp.json) in this folder reach every account's claude in Hive's terminals, " +
          "as a plugin. Credentials are never shared: each account logs in on its own."
        }
      >
        Skills, agents, commands and MCP servers here reach every account, as a plugin.
      </p>
      <div className="shared-row">
        {shared ? <FolderCell path={shared.folder} /> : <span className="folder-cell">…</span>}
        <button type="button" className="secondary" onClick={openSharedFolder}>
          Open folder
        </button>
      </div>
      {shared?.error && <p className="field-error">{shared.error}</p>}
      {shared && (
        <p className="field-help shared-held">
          {held.length
            ? held.map(([label, names]) => `${label}: ${names.join(", ")}`).join(" · ")
            : "Skills, commands, agents and MCP servers: none"}
        </p>
      )}
    </section>
  );
}

/**
 * A hint while the OS keeps Hive from notifying (17.19: macOS's settings say "denied"), with a
 * link to them. Asked when the section opens and whenever the window comes back to the front,
 * as from System Settings.
 */
function NotificationsOff() {
  const denied = useHive((s) => s.notificationPermission === "denied");
  useEffect(() => {
    const check = () => void transport.checkNotifications();
    check();
    window.addEventListener("focus", check);
    return () => window.removeEventListener("focus", check);
  }, []);
  if (!denied) return null;
  return (
    <p className="field-help notifications-off" role="note">
      Notifications from Hive are turned off in System Settings: alerts show only in the inbox.{" "}
      <button
        type="button"
        className="link"
        onClick={() => void transport.openNotificationSettings()}
      >
        Open System Settings
      </button>
    </p>
  );
}

function Content({ section, query }: { section: Section; query: string }) {
  const modes = useHive((s) => s.appMode?.wsl === true);
  const windows = useHive((s) => s.appMode?.mode === "native");
  const mac = isMac();
  const fields = FIELDS.filter(
    (f) => (modes || !f.modes) && (windows || !f.windows) && (mac || !f.mac),
  );
  if (query) {
    const found = fields.filter((f) => f.label.toLowerCase().includes(query.toLowerCase()));
    if (found.length === 0) return <p className="field-help">No setting matches.</p>;
    return <Fields fields={found} />;
  }
  if (section === "Shortcuts") return <Shortcuts />;
  if (section === "About") return <About />;
  if (section === "Projects") return <ProjectScriptsSection />;
  const shown = <Fields fields={fields.filter((f) => f.section === section)} />;
  if (section === "Accounts") {
    return (
      <>
        <Accounts />
        <GhAccountsPart />
        <section className="field settings-group" aria-labelledby="sessions-title">
          <h3 id="sessions-title">Sessions</h3>
          {shown}
        </section>
        <SharedPlugin />
      </>
    );
  }
  if (section !== "Notifications") return shown;
  return (
    <>
      <NotificationsOff />
      {shown}
    </>
  );
}

/**
 * Ctrl+, (6.2): the service's settings by section, or every field matching the search. Each
 * change is sent at once (text and numbers once typing pauses) as the whole settings; the
 * service's refusal shows at the top. Terminals and the theme follow the stored settings.
 */
export function SettingsDialog() {
  const [section, setSection] = useState<Section>(() =>
    useHive.getState().modalProject ? "Projects" : "Terminal",
  );
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
