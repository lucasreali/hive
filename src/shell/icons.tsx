import {
  ChatCircleDotsIcon,
  CheckCircleIcon,
  CircleDashedIcon,
  CircleHalfIcon,
  CirclesThreeIcon,
  type Icon as PhosphorIcon,
  StopCircleIcon,
  XCircleIcon,
  XIcon,
} from "@phosphor-icons/react";
import type { AgentState } from "../store";

// Inline SVG icons from the prototype (docs/prototype/HiveApp.dc.html).

// The Hive logo (src-tauri/icons/icon.svg): a honeycomb with one lit cell.
const HEX = "0,-20 17.32,-10 17.32,10 0,20 -17.32,10 -17.32,-10";
const CELLS = [
  [-19.32, -33.46],
  [-38.64, 0],
  [0, 0],
  [38.64, 0],
  [-19.32, 33.46],
  [19.32, 33.46],
];

export const HiveIcon = () => (
  <svg width="16" height="16" viewBox="-60 -60 120 120" aria-hidden="true">
    {CELLS.map(([x, y]) => (
      <polygon key={`${x},${y}`} points={HEX} transform={`translate(${x} ${y})`} fill="#7D776C" />
    ))}
    <polygon points={HEX} transform="translate(19.32 -33.46)" fill="#F2B53C" />
  </svg>
);

export const MinimizeIcon = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
    <path d="M0 5.5h10" stroke="currentColor" />
  </svg>
);

export const MaximizeIcon = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
    <rect x=".5" y=".5" width="9" height="9" rx="1" fill="none" stroke="currentColor" />
  </svg>
);

export const WindowCloseIcon = () => <XIcon size={14} weight="light" aria-hidden="true" />;

export const CloseIcon = () => <XIcon size={12} weight="bold" aria-hidden="true" />;

export const PlusIcon = ({ size = 12 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 12 12" aria-hidden="true">
    <path d="M6 2v8M2 6h8" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
  </svg>
);

export const CheckIcon = () => (
  <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
    <path
      d="M2.5 6.3 4.8 8.5 9.5 3.5"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.4"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

export const PanelIcon = () => (
  <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
    <rect
      x="1.75"
      y="2.25"
      width="10.5"
      height="9.5"
      rx="1.5"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.1"
    />
    <path d="M8.75 2.25v9.5" stroke="currentColor" strokeWidth="1.1" />
  </svg>
);

export const ChevronIcon = ({ open }: { open: boolean }) => (
  <svg
    width="10"
    height="10"
    viewBox="0 0 10 10"
    aria-hidden="true"
    style={{ transform: open ? "rotate(90deg)" : undefined }}
  >
    <path
      d="M3.5 2 6.5 5 3.5 8"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

const FOLDER =
  "M1.75 4.2c0-.64.52-1.2 1.2-1.2h2.7l1.3 1.4h4.1c.66 0 1.2.54 1.2 1.2v5.2c0 .66-.54 1.2-1.2 1.2H2.95c-.66 0-1.2-.54-1.2-1.2z";

export const FolderIcon = () => (
  <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
    <path d={FOLDER} fill="none" stroke="currentColor" strokeWidth="1.1" strokeLinejoin="round" />
  </svg>
);

export const FileIcon = () => (
  <svg width="12" height="13" viewBox="0 0 12 14" aria-hidden="true">
    <g fill="none" stroke="currentColor" strokeWidth="1" strokeLinejoin="round">
      <path d="M2.5 1.5h4.8L10 4.2v8.3H2.5z" />
      <path d="M7 1.5v3h3" />
    </g>
  </svg>
);

/** The empty state's large "add folder" icon. */
export const AddFolderIcon = () => (
  <svg width="28" height="28" viewBox="0 0 14 14" aria-hidden="true">
    <path d={FOLDER} fill="none" stroke="currentColor" strokeWidth=".7" strokeLinejoin="round" />
    <path d="M7 6.3v3M5.5 7.8h3" stroke="currentColor" strokeWidth=".7" strokeLinecap="round" />
  </svg>
);

export const BranchIcon = () => (
  <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
    <g fill="none" stroke="currentColor" strokeWidth="1.1">
      <circle cx="3.2" cy="2.6" r="1.35" />
      <circle cx="3.2" cy="9.4" r="1.35" />
      <circle cx="8.8" cy="3.6" r="1.35" />
      <path d="M3.2 4v4M8.8 5c0 2.2-5.6 1.4-5.6 3" />
    </g>
  </svg>
);

/** The pending counter's bell. */
export const BellIcon = () => (
  <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
    <path d="M3 8.3V5.4a3 3 0 0 1 6 0v2.9l.9 1.1H2.1z" fill="currentColor" />
    <path d="M5 10.6h2" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
  </svg>
);

/** Row labels for each state (docs/ui-reference.md glossary). */
export const STATE_LABEL: Record<AgentState, string> = {
  waiting_permission: "waiting for permission",
  error: "error",
  waiting_you: "waiting for you",
  working: "working",
  with_subagents: "running subagents",
  idle: "idle",
  ended: "ended",
};

// Phosphor icons (outlined), each in its state's color token.
const STATE_ICON: Record<AgentState, PhosphorIcon> = {
  waiting_permission: CircleHalfIcon,
  error: XCircleIcon,
  waiting_you: ChatCircleDotsIcon,
  working: CircleDashedIcon,
  with_subagents: CirclesThreeIcon,
  idle: CheckCircleIcon,
  ended: StopCircleIcon,
};

/** A state's icon and color (`--state-*`), named for screen readers; working spins. */
export const StateIcon = ({ state }: { state: AgentState }) => {
  const Shape = STATE_ICON[state];
  return (
    <Shape
      className="state-icon"
      size={14}
      weight="bold"
      role="img"
      aria-label={STATE_LABEL[state]}
      alt={STATE_LABEL[state]}
      data-state={state}
    />
  );
};

// Not in the prototype: the sidebar's "Refresh worktrees" button (worktrees are not watched yet).
export const RefreshIcon = () => (
  <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
    <path
      d="M10 6a4 4 0 1 1-1.2-2.85M10 1.8v2.4H7.6"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.2"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

export const TerminalIcon = () => (
  <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
    <rect x="1" y="2" width="10" height="8" rx="1.5" fill="none" stroke="currentColor" />
    <path
      d="M3.5 5 5 6.2 3.5 7.4M6.2 7.6h2.2"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

/** A box with an arrow out of it: open elsewhere. */
export const ExternalIcon = () => (
  <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
    <path
      d="M5 2.5H3a1 1 0 0 0-1 1V9a1 1 0 0 0 1 1h5.5a1 1 0 0 0 1-1V7M7 2h3v3M10 2 5.5 6.5"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

export const SearchIcon = () => (
  <svg width="13" height="13" viewBox="0 0 14 14" aria-hidden="true">
    <g fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round">
      <circle cx="6" cy="6" r="4" />
      <path d="M9 9l3 3" />
    </g>
  </svg>
);
