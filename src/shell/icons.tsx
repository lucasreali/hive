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
