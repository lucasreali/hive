import { useEffect, useState } from "react";
import type { UninstallFailure, UninstallKind } from "../protocol";
import { openModal, useHive } from "../store";
import { transport } from "../transport";
import { closeWindow } from "../window";
import { CloseIcon } from "./icons";

// Uninstall Hive (23.5, D22.17): the app menu's and the palette's "Uninstall Hive…". The service
// lists what goes and removes it (#37); the dialog shows its list, the two boxes and the confirm.

/** The heading of each kind of item, in the order the service lists them. */
export const KIND_LABEL: Record<UninstallKind, string> = {
  data: "Settings, spaces and accounts",
  account: "Claude account folders Hive made",
  program: "The service and the hive shortcut",
  cache: "Caches and logs",
  app: "The app",
};

/** Hive is uninstalled: it quits, unless something could not go, which the dialog then lists. */
export function onUninstalled(failed: UninstallFailure[]): void {
  if (failed.length === 0) closeWindow();
}

const cancel = () => openModal(null);

/** Says how many `count` of `one` there are: "1 terminal", "2 terminals". */
const counted = (count: number, one: string) => `${count} ${one}${count === 1 ? "" : "s"}`;

export function UninstallDialog() {
  const [keepData, setKeepData] = useState(false);
  const [accounts, setAccounts] = useState(false);
  const [sent, setSent] = useState(false);
  const list = useHive((s) => s.uninstallList);
  const failed = useHive((s) => s.uninstallFailed);
  const terminals = useHive((s) => Object.keys(s.terminals).length);
  const agents = useHive((s) => Object.keys(s.agents).length);
  // Asked again on each box: the service decides what goes.
  useEffect(() => {
    void transport.listUninstall(keepData, accounts && !keepData);
  }, [keepData, accounts]);
  // A list from an earlier opening is not shown: what is there may have changed since.
  useEffect(() => useHive.setState({ uninstallList: null }), []);
  // The list for the boxes as they are now, not one still on its way.
  const current =
    list && list.keep_data === keepData && list.accounts === (accounts && !keepData) ? list : null;
  const confirm = () => {
    setSent(true);
    void transport.uninstall(keepData, accounts && !keepData);
  };
  const kinds = [...new Set(current?.items.map((i) => i.kind))];
  return (
    // A native modal dialog: the page behind is inert and Esc closes it.
    <dialog
      className="dialog uninstall"
      aria-labelledby="uninstall-title"
      ref={(dialog) => {
        if (dialog && !dialog.open) dialog.showModal();
      }}
      onCancel={(e) => {
        if (sent) e.preventDefault();
      }}
      onClose={cancel}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (failed) closeWindow();
          else confirm();
        }}
      >
        <header>
          <h2 id="uninstall-title">{failed ? "Hive is uninstalled" : "Uninstall Hive?"}</h2>
          {!sent && (
            <button type="button" className="ghost" title="Close (Esc)" onClick={cancel}>
              <CloseIcon />
            </button>
          )}
        </header>
        {failed ? (
          <div className="dialog-body">
            <p>These could not be removed; remove them by hand:</p>
            <ul className="uninstall-list" aria-label="Not removed">
              {failed.map((f) => (
                <li key={f.path} className="uninstall-item">
                  <code>{f.path}</code>
                  <span className="field-error">{f.error}</span>
                </li>
              ))}
            </ul>
          </div>
        ) : (
          <div className="dialog-body">
            {terminals > 0 && (
              <p className="field-warning">
                {counted(terminals, "terminal")} {terminals === 1 ? "is" : "are"} open
                {agents > 0 ? `, with ${counted(agents, "agent")}` : ""}: uninstalling closes them,
                as quitting does.
              </p>
            )}
            <p>These go to the Trash, so you can put them back:</p>
            {current === null ? (
              <p className="field-help">Listing…</p>
            ) : current.items.length === 0 ? (
              <p className="field-help">Nothing of Hive's is left to remove.</p>
            ) : (
              kinds.map((kind) => (
                <section key={kind} aria-label={KIND_LABEL[kind]}>
                  <h3 className="uninstall-kind">{KIND_LABEL[kind]}</h3>
                  <ul className="uninstall-list">
                    {current.items
                      .filter((i) => i.kind === kind)
                      .map((i) => (
                        <li key={i.path}>
                          <code>{i.path}</code>
                        </li>
                      ))}
                  </ul>
                </section>
              ))
            )}
            <label className="checkbox">
              <input
                type="checkbox"
                checked={keepData}
                disabled={sent}
                onChange={(e) => setKeepData(e.target.checked)}
              />
              Keep my settings, spaces and accounts
            </label>
            {(list?.account_folders.length ?? 0) > 0 && (
              <>
                <label className="checkbox">
                  <input
                    type="checkbox"
                    checked={accounts && !keepData}
                    disabled={sent || keepData}
                    onChange={(e) => setAccounts(e.target.checked)}
                  />
                  Also remove the Claude account folders Hive made
                </label>
                <p className="field-help">
                  They hold those accounts' sessions, settings and skills:{" "}
                  {list?.account_folders.join(", ")}.
                </p>
              </>
            )}
            <p className="field-help">
              Never touched: ~/.claude, ~/.claude.json, your projects, their worktrees and branches.
              To remove Claude Code's login too, run /logout in claude first.
            </p>
          </div>
        )}
        <footer>
          {failed ? (
            <button type="submit" className="primary">
              Quit
            </button>
          ) : (
            <>
              <button type="button" className="secondary" disabled={sent} onClick={cancel}>
                Cancel
              </button>
              <button
                type="submit"
                className="primary danger"
                disabled={sent || !current || current.items.length === 0}
              >
                {sent ? "Uninstalling…" : "Uninstall"}
              </button>
            </>
          )}
        </footer>
      </form>
    </dialog>
  );
}
