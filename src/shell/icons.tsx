import type { ReactNode } from "react";
import type { AgentState } from "../store";

// Inline SVG icons from the prototype (docs/prototype/HiveApp.dc.html).

export const HiveIcon = () => (
  <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
    <path
      d="M7 1.5 11.8 4.25v5.5L7 12.5 2.2 9.75v-5.5z"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.2"
      strokeLinejoin="round"
    />
    <path d="M7 5 9 6.1v2.3L7 9.5 5 8.4V6.1z" fill="currentColor" />
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

export const WindowCloseIcon = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
    <path d="M.5.5l9 9M9.5.5l-9 9" stroke="currentColor" />
  </svg>
);

export const CloseIcon = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
    <path d="M2 2l6 6M8 2 2 8" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
  </svg>
);

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

const STATE_SHAPE: Record<AgentState, ReactNode> = {
  waiting_permission: (
    <>
      <path
        d="M6 1.3 11.2 10.5H.8z"
        fill="var(--state-permission)"
        stroke="var(--state-permission)"
        strokeLinejoin="round"
      />
      <rect x="5.35" y="4.2" width="1.3" height="3.3" rx=".6" fill="var(--bg)" />
      <circle cx="6" cy="8.9" r=".75" fill="var(--bg)" />
    </>
  ),
  error: (
    <>
      <circle cx="6" cy="6" r="5.2" fill="var(--state-error)" />
      <path d="M4 4 8 8M8 4 4 8" stroke="var(--bg)" strokeWidth="1.5" strokeLinecap="round" />
    </>
  ),
  waiting_you: (
    <>
      <circle cx="6" cy="6" r="4.4" fill="none" stroke="var(--state-you)" strokeWidth="1.6" />
      <circle cx="6" cy="6" r="1.7" fill="var(--state-you)" />
    </>
  ),
  working: (
    <>
      <circle className="pulse" cx="6" cy="6" r="3.1" fill="var(--state-working)" />
      <circle cx="6" cy="6" r="3.1" fill="var(--state-working)" />
    </>
  ),
  with_subagents: (
    <>
      <path
        d="M6 3v3M6 6 2.8 9M6 6l3.2 3"
        fill="none"
        stroke="var(--state-subagents)"
        strokeWidth="1.2"
      />
      <circle cx="6" cy="2.6" r="2" fill="var(--state-subagents)" />
      <circle cx="2.6" cy="9.3" r="2" fill="var(--state-subagents)" />
      <circle cx="9.4" cy="9.3" r="2" fill="var(--state-subagents)" />
    </>
  ),
  idle: (
    <>
      <circle cx="6" cy="6" r="5.2" fill="var(--state-idle)" />
      <path
        d="M3.6 6.2 5.3 7.8 8.5 4.4"
        fill="none"
        stroke="var(--bg)"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </>
  ),
  ended: <rect x="2" y="2" width="8" height="8" rx="1.5" fill="var(--state-ended)" />,
};

/** `StateIcon.dc.html`: a state's color and shape, named for screen readers. */
export const StateIcon = ({ state }: { state: AgentState }) => (
  <svg
    className="state-icon"
    width="12"
    height="12"
    viewBox="0 0 12 12"
    role="img"
    aria-label={STATE_LABEL[state]}
    data-state={state}
  >
    <title>{STATE_LABEL[state]}</title>
    {STATE_SHAPE[state]}
  </svg>
);

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

export const SearchIcon = () => (
  <svg width="13" height="13" viewBox="0 0 14 14" aria-hidden="true">
    <g fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round">
      <circle cx="6" cy="6" r="4" />
      <path d="M9 9l3 3" />
    </g>
  </svg>
);
