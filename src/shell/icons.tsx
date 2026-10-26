import {
  ArrowClockwiseIcon,
  ArrowSquareOutIcon,
  CaretRightIcon,
  ChatCircleDotsIcon,
  CheckCircleIcon,
  CircleDashedIcon,
  CirclesThreeIcon,
  FolderPlusIcon,
  FolderSimpleIcon,
  FoldersIcon,
  GitBranchIcon,
  ListChecksIcon,
  MagnifyingGlassIcon,
  MinusIcon,
  CheckIcon as PhosphorCheckIcon,
  EyeIcon as PhosphorEyeIcon,
  FileIcon as PhosphorFileIcon,
  type Icon as PhosphorIcon,
  PlusIcon as PhosphorPlusIcon,
  QuestionIcon,
  ShieldWarningIcon,
  SidebarSimpleIcon,
  SquareIcon,
  StopCircleIcon,
  TerminalWindowIcon,
  XCircleIcon,
  XIcon,
} from "@phosphor-icons/react";
import type { AgentState } from "../protocol";

// The title bar's Hive logo: the app icon's honeycomb, six cells and the lit one (the cell of an
// agent that needs you), in the color of the text beside it.
const HIVE_CELLS =
  "M5.29,1.48A0.48,0.48 0 0 1 5.77,1.48L7.35,2.39A0.48,0.48 0 0 1 7.59,2.81L7.59,4.63A0.48,0.48 0 0 1 7.35,5.05L5.77,5.96A0.48,0.48 0 0 1 5.29,5.96L3.71,5.05A0.48,0.48 0 0 1 3.47,4.63L3.47,2.81A0.48,0.48 0 0 1 3.71,2.39ZM2.82,5.76A0.48,0.48 0 0 1 3.30,5.76L4.88,6.67A0.48,0.48 0 0 1 5.12,7.09L5.12,8.91A0.48,0.48 0 0 1 4.88,9.33L3.30,10.24A0.48,0.48 0 0 1 2.82,10.24L1.24,9.33A0.48,0.48 0 0 1 1.00,8.91L1.00,7.09A0.48,0.48 0 0 1 1.24,6.67ZM7.76,5.76A0.48,0.48 0 0 1 8.24,5.76L9.82,6.67A0.48,0.48 0 0 1 10.06,7.09L10.06,8.91A0.48,0.48 0 0 1 9.82,9.33L8.24,10.24A0.48,0.48 0 0 1 7.76,10.24L6.18,9.33A0.48,0.48 0 0 1 5.94,8.91L5.94,7.09A0.48,0.48 0 0 1 6.18,6.67ZM12.70,5.76A0.48,0.48 0 0 1 13.18,5.76L14.76,6.67A0.48,0.48 0 0 1 15.00,7.09L15.00,8.91A0.48,0.48 0 0 1 14.76,9.33L13.18,10.24A0.48,0.48 0 0 1 12.70,10.24L11.12,9.33A0.48,0.48 0 0 1 10.88,8.91L10.88,7.09A0.48,0.48 0 0 1 11.12,6.67ZM5.29,10.04A0.48,0.48 0 0 1 5.77,10.04L7.35,10.95A0.48,0.48 0 0 1 7.59,11.37L7.59,13.19A0.48,0.48 0 0 1 7.35,13.61L5.77,14.52A0.48,0.48 0 0 1 5.29,14.52L3.71,13.61A0.48,0.48 0 0 1 3.47,13.19L3.47,11.37A0.48,0.48 0 0 1 3.71,10.95ZM10.23,10.04A0.48,0.48 0 0 1 10.71,10.04L12.29,10.95A0.48,0.48 0 0 1 12.53,11.37L12.53,13.19A0.48,0.48 0 0 1 12.29,13.61L10.71,14.52A0.48,0.48 0 0 1 10.23,14.52L8.65,13.61A0.48,0.48 0 0 1 8.41,13.19L8.41,11.37A0.48,0.48 0 0 1 8.65,10.95Z";
const HIVE_LIT =
  "M10.23,1.48A0.48,0.48 0 0 1 10.71,1.48L12.29,2.39A0.48,0.48 0 0 1 12.53,2.81L12.53,4.63A0.48,0.48 0 0 1 12.29,5.05L10.71,5.96A0.48,0.48 0 0 1 10.23,5.96L8.65,5.05A0.48,0.48 0 0 1 8.41,4.63L8.41,2.81A0.48,0.48 0 0 1 8.65,2.39Z";

export const HiveIcon = () => (
  <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
    <path d={HIVE_CELLS} fill="currentColor" fillOpacity={0.5} />
    <path d={HIVE_LIT} fill="currentColor" />
  </svg>
);

// Every icon is Phosphor (7.7): regular weight at 14 px in currentColor, hidden from screen
// readers (the button or row around it carries the name). Exceptions (docs/ui-reference.md):
// chevrons 10 px bold, plus/check beside text 12 px, the dialogs' close cross 15 px bold, window
// buttons light, the empty state's 28 px icon. File tree files and folders use @react-symbols/icons.

/** Props of a decorative icon at the app's size: `<CopyIcon {...ICON} />`. */
export const ICON = { size: 14, "aria-hidden": true } as const;

const icon = (Shape: PhosphorIcon) => () => <Shape {...ICON} />;

export const MinimizeIcon = () => <MinusIcon size={14} weight="light" aria-hidden="true" />;

export const MaximizeIcon = () => <SquareIcon size={12} weight="light" aria-hidden="true" />;

export const WindowCloseIcon = () => <XIcon size={14} weight="light" aria-hidden="true" />;

export const CloseIcon = () => <XIcon size={15} weight="bold" aria-hidden="true" />;

export const PlusIcon = ({ size = 12 }: { size?: number }) => (
  <PhosphorPlusIcon size={size} aria-hidden="true" />
);

export const CheckIcon = () => <PhosphorCheckIcon size={12} aria-hidden="true" />;

// The right panel's toggle: Phosphor's sidebar, mirrored so the panel is on the right.
export const PanelIcon = () => <SidebarSimpleIcon {...ICON} mirrored />;

export const ChevronIcon = ({ open }: { open: boolean }) => (
  <CaretRightIcon
    size={10}
    weight="bold"
    aria-hidden="true"
    style={{ transform: open ? "rotate(90deg)" : undefined }}
  />
);

export const FolderIcon = icon(FolderSimpleIcon);

/** A group (14.1): a folder of projects. */
export const GroupIcon = icon(FoldersIcon);

export const FileIcon = icon(PhosphorFileIcon);

/** The empty state's large "add folder" icon. */
export const AddFolderIcon = () => <FolderPlusIcon size={28} weight="light" aria-hidden="true" />;

export const BranchIcon = icon(GitBranchIcon);

/** Row labels for each state (docs/ui-reference.md glossary). */
export const STATE_LABEL: Record<AgentState, string> = {
  waiting_permission: "waiting for permission",
  waiting_plan: "waiting for plan approval",
  waiting_answer: "waiting for your answer",
  error: "error",
  waiting_you: "waiting for you",
  working: "working",
  with_subagents: "running subagents",
  idle: "idle",
  ended: "ended",
};

// Phosphor icons (outlined), each in its state's color token.
const STATE_ICON: Record<AgentState, PhosphorIcon> = {
  waiting_permission: ShieldWarningIcon,
  waiting_plan: ListChecksIcon,
  waiting_answer: QuestionIcon,
  error: XCircleIcon,
  waiting_you: ChatCircleDotsIcon,
  working: CircleDashedIcon,
  with_subagents: CirclesThreeIcon,
  idle: CheckCircleIcon,
  ended: StopCircleIcon,
};

/**
 * A state's icon and color (`--state-*`), named for screen readers; working spins. A `muted` one
 * (a 🟠 already seen, 15.6) is drawn in the muted text color and named so.
 */
export const StateIcon = ({ state, muted }: { state: AgentState; muted?: boolean }) => {
  const Shape = STATE_ICON[state];
  const label = muted ? `${STATE_LABEL[state]}, seen` : STATE_LABEL[state];
  return (
    <Shape
      className="state-icon"
      size={14}
      weight="bold"
      role="img"
      aria-label={label}
      alt={label}
      data-state={state}
      data-muted={muted || undefined}
    />
  );
};

// Not in the prototype: the sidebar's "Refresh worktrees" button (worktrees are not watched yet).
export const RefreshIcon = icon(ArrowClockwiseIcon);

export const TerminalIcon = icon(TerminalWindowIcon);

/** A box with an arrow out of it: open elsewhere. */
export const ExternalIcon = icon(ArrowSquareOutIcon);

export const EyeIcon = icon(PhosphorEyeIcon);

export const SearchIcon = icon(MagnifyingGlassIcon);
