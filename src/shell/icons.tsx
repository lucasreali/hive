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
  GitBranchIcon,
  ListChecksIcon,
  MagnifyingGlassIcon,
  MinusIcon,
  CheckIcon as PhosphorCheckIcon,
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
import type { AgentState } from "../store";

// The title bar's Hive logo: a honeycomb of seven hollow cells (the human's drawing), in the color
// of the text beside it.
const HIVE_LOGO =
  "M425.5,54.6 L333.0,1.2 L250.5,48.8 L168.0,1.2 L75.5,54.6 L75.5,149.3 L-6.5,196.6 L-6.5,303.4 L75.5,350.7 L75.5,446.4 L168.0,499.8 L250.5,452.2 L333.0,499.8 L425.5,446.4 L425.5,351.3 L508.5,303.4 L508.5,196.6 L425.5,148.7 Z M168.0,194.0 L93.5,151.0 L93.5,65.0 L168.0,22.0 L242.5,65.0 L242.5,151.0 Z M86.0,336.0 L11.5,293.0 L11.5,207.0 L86.0,164.0 L160.5,207.0 L160.5,293.0 Z M251.0,336.0 L176.5,293.0 L176.5,207.0 L251.0,164.0 L325.5,207.0 L325.5,293.0 Z M416.0,336.0 L341.5,293.0 L341.5,207.0 L416.0,164.0 L490.5,207.0 L490.5,293.0 Z M168.0,479.0 L93.5,436.0 L93.5,350.0 L168.0,307.0 L242.5,350.0 L242.5,436.0 Z M333.0,479.0 L258.5,436.0 L258.5,350.0 L333.0,307.0 L407.5,350.0 L407.5,436.0 Z";

export const HiveIcon = () => (
  <svg width="16" height="16" viewBox="-14 -7 531 515" aria-hidden="true">
    <path d={HIVE_LOGO} fillRule="evenodd" fill="currentColor" />
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
export const RefreshIcon = icon(ArrowClockwiseIcon);

export const TerminalIcon = icon(TerminalWindowIcon);

/** A box with an arrow out of it: open elsewhere. */
export const ExternalIcon = icon(ArrowSquareOutIcon);

export const SearchIcon = icon(MagnifyingGlassIcon);
