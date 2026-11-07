import { ArrowUpIcon, GitBranchIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import { useHive } from "../store";
import { transport } from "../transport";
import { Select } from "../ui/Select";
import { FolderIcon, ICON, NewFolderIcon } from "./icons";

/** How long typing must pause before the folder is listed. */
export const LIST_DELAY_MS = 150;

/** Stable, so it focuses the name once when shown, not again on every render. */
const focusOnMount = (el: HTMLInputElement | null) => el?.focus();

const SIDES = [
  { value: "windows", label: "Windows" },
  { value: "wsl", label: "WSL" },
];

/**
 * The service's listing for `path` on that side (the home folder's while `path` is empty), or
 * null while it is for another path.
 */
export const useFolderListing = (path: string, windows = false) =>
  useHive((s) =>
    s.dirs?.windows === windows && (s.dirs.path === path || path === "") ? s.dirs : null,
  );

export type FolderFieldProps = {
  id: string;
  /** Without a `<label>` of its own. */
  "aria-label"?: string;
  value: string;
  onChange: (path: string) => void;
  placeholder: string;
  /** Windows paths (the WSL service's Windows side); with `onWindows`, a WSL | Windows select. */
  windows?: boolean;
  onWindows?: (windows: boolean) => void;
  /** Lists hidden folders too; otherwise only when the last name starts with `.`. */
  hidden?: boolean;
  /** Fills the empty field with the home folder once listed, until the user types. */
  home?: boolean;
  invalid?: boolean;
  onBlur?: () => void;
  /** A "New folder" button (23.2): a folder made in the typed one, which the field then enters. */
  newFolder?: boolean;
};

/**
 * A folder typed or browsed: under the field, the subfolders of the typed folder (listed by
 * the service once typing pauses) filtered by the text after its last separator; a click enters
 * one, ↑ goes up. With the service on Windows itself, every path is a Windows one (`\` or `/`).
 */
export function FolderField(props: FolderFieldProps) {
  const { id, value, onChange, windows = false, onWindows, hidden = false } = props;
  const wsl = useHive((s) => s.connection.status === "connected" && s.connection.distro !== null);
  // Windows paths: the WSL service's Windows side, or every path of the service on Windows.
  const backslash = useHive((s) => windows || s.appMode?.mode === "native");
  const [fill, setFill] = useState(props.home === true);
  const input = useRef<HTMLInputElement>(null);
  const listing = useFolderListing(value, windows);
  useEffect(() => {
    const later = setTimeout(() => void transport.listDirs(value, windows, hidden), LIST_DELAY_MS);
    return () => clearTimeout(later);
  }, [value, windows, hidden]);
  useEffect(() => {
    if (fill && listing && value === "") {
      onChange(listing.path);
      setFill(false);
    }
  }, [fill, listing, value, onChange]);

  const typed = value || listing?.path || "";
  const cut = Math.max(typed.lastIndexOf("/"), backslash ? typed.lastIndexOf("\\") : -1) + 1;
  const filter = typed.slice(cut).toLowerCase();
  const shown = listing?.dirs.filter((d) => d.name.toLowerCase().includes(filter)) ?? [];
  const go = (to: string) => {
    onChange(to);
    setFill(false);
    input.current?.focus();
  };
  // "New folder": its name while typed (null: closed), and the request last sent.
  const [naming, setNaming] = useState<string | null>(null);
  const [asked, setAsked] = useState<{ parent: string; name: string } | null>(null);
  const answer = useHive((s) => s.newDir);
  const answered = asked && answer?.parent === asked.parent && answer.name === asked.name;
  // The folder typed (ending with a separator), as the service has it.
  const parent = cut === typed.length && listing?.path === typed ? listing.linux_path : null;
  const sep = backslash ? "\\" : "/";
  const made = answered && answer.error === null;
  useEffect(() => {
    if (!made || !asked) return;
    setAsked(null);
    setNaming(null);
    onChange(`${typed}${asked.name}${sep}`);
    setFill(false);
    input.current?.focus();
  }, [made, asked, typed, sep, onChange]);
  const nameError = answered ? answer.error : null;
  const closeNaming = () => {
    setNaming(null);
    setAsked(null);
    input.current?.focus();
  };
  return (
    <div className="folder-picker">
      <div className="folder-path">
        {wsl && onWindows && (
          <Select
            aria-label="Folder kind"
            value={windows ? "windows" : "wsl"}
            options={SIDES}
            onChange={(side) => {
              onWindows(side === "windows");
              onChange("");
              setFill(true);
            }}
          />
        )}
        <input
          id={id}
          aria-label={props["aria-label"]}
          ref={input}
          value={value}
          onChange={(e) => {
            onChange(e.target.value);
            setFill(false);
          }}
          onBlur={props.onBlur}
          placeholder={props.placeholder}
          spellCheck={false}
          autoComplete="off"
          aria-invalid={props.invalid}
        />
        {props.newFolder && (
          <button
            type="button"
            className="ghost folder-new"
            title={parent || naming !== null ? "New folder" : "Type or pick a folder first"}
            aria-label="New folder"
            aria-pressed={naming !== null}
            disabled={!parent && naming === null}
            // Keeps the focus in the name, so its blur does not close the row this click toggles.
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => (naming === null ? setNaming("") : closeNaming())}
          >
            <NewFolderIcon />
          </button>
        )}
      </div>
      <ul className="folder-list" aria-label="Folders">
        {naming !== null && (
          <li className="folder-naming">
            <div className="folder-row">
              <FolderIcon />
              <input
                aria-label="New folder name"
                className="folder-name"
                value={naming}
                ref={focusOnMount}
                onChange={(e) => {
                  setNaming(e.target.value);
                  setAsked(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    // Only the name closes, not the dialog.
                    e.preventDefault();
                    e.stopPropagation();
                    closeNaming();
                  } else if (e.key === "Enter") {
                    e.preventDefault();
                    if (!parent) return;
                    setAsked({ parent, name: naming });
                    void transport.createDir(parent, naming);
                  }
                }}
                // Leaving it with nothing typed closes it; a typed name stays.
                onBlur={() => naming === "" && setNaming(null)}
                placeholder="Folder name, then Enter"
                spellCheck={false}
                autoComplete="off"
                aria-invalid={nameError !== null}
              />
            </div>
            {nameError && (
              <span className="field-error" role="alert">
                {nameError}
              </span>
            )}
          </li>
        )}
        {listing?.parent && (
          <li>
            <button
              type="button"
              tabIndex={-1}
              className="folder-row"
              title="Up a level"
              onClick={() => go(listing.parent as string)}
            >
              <ArrowUpIcon {...ICON} />
              <span className="folder-name">{listing.parent}</span>
            </button>
          </li>
        )}
        {shown.map((d) => (
          <li key={d.name}>
            <button
              type="button"
              tabIndex={-1}
              className="folder-row"
              onClick={() => go(`${typed.slice(0, cut)}${d.name}${backslash ? "\\" : "/"}`)}
            >
              {d.git ? <GitBranchIcon size={14} aria-label="Repository" /> : <FolderIcon />}
              <span className="folder-name">{d.name}</span>
            </button>
          </li>
        ))}
        {listing && shown.length === 0 && (
          <li className="folder-empty">{listing.error ?? "No folders here"}</li>
        )}
      </ul>
    </div>
  );
}
