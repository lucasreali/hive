import { useState } from "react";
import { currentSpace, openModal, type SpaceEnv, useHive } from "../store";
import { transport } from "../transport";
import { CloseIcon } from "./icons";

const close = () => openModal(null);

/** The environment fields, in the dialog's order: what each sets in the space's terminals. */
const FIELDS: { key: keyof SpaceEnv; label: string; placeholder: string; help: string }[] = [
  {
    key: "claude_config_dir",
    label: "Claude config folder",
    placeholder: "/home/you/.claude-work",
    help: "CLAUDE_CONFIG_DIR: the space's Claude account, settings and sessions.",
  },
  {
    key: "git_name",
    label: "Git name",
    placeholder: "Your Name",
    help: "GIT_AUTHOR_NAME and GIT_COMMITTER_NAME.",
  },
  {
    key: "git_email",
    label: "Git email",
    placeholder: "you@work.example",
    help: "GIT_AUTHOR_EMAIL and GIT_COMMITTER_EMAIL.",
  },
  {
    key: "gh_config_dir",
    label: "GitHub CLI config folder",
    placeholder: "/home/you/.config/gh-work",
    help: "GH_CONFIG_DIR: the space's gh account.",
  },
];

/**
 * Creates a space ("new-space") or edits the current one ("edit-space"): its name and what its
 * new terminals get in their environment; an empty field leaves the user's own. The service
 * checks everything and answers `spaces` (the dialog closes) or why it refused (shown).
 */
export function SpaceDialog({ editing }: { editing: boolean }) {
  const space = useHive((s) => (editing ? currentSpace(s) : undefined));
  const error = useHive((s) => s.spaceError);
  const [name, setName] = useState(space?.name ?? "");
  const [env, setEnv] = useState<SpaceEnv>(
    space?.env ?? { claude_config_dir: null, git_name: null, git_email: null, gh_config_dir: null },
  );
  const title = editing ? "Edit space" : "New space";
  const empty = space?.projects.length === 0;
  return (
    <dialog
      className="dialog"
      aria-labelledby="space-title"
      ref={(dialog) => {
        if (dialog && !dialog.open) dialog.showModal();
      }}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (space) void transport.updateSpace(space.id, name, env);
          else void transport.createSpace(name, env);
        }}
      >
        <header>
          <h2 id="space-title">{title}</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          <div className="field">
            <label htmlFor="space-name">Name</label>
            <input
              id="space-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Work"
              autoComplete="off"
            />
          </div>
          {FIELDS.map((f) => (
            <div className="field" key={f.key}>
              <label htmlFor={`space-${f.key}`}>{f.label}</label>
              <input
                id={`space-${f.key}`}
                value={env[f.key] ?? ""}
                onChange={(e) => setEnv({ ...env, [f.key]: e.target.value || null })}
                placeholder={f.placeholder}
                spellCheck={false}
                autoComplete="off"
              />
              <p className="field-help">{f.help}</p>
            </div>
          ))}
          {error && (
            <span className="field-error" role="alert">
              {error}
            </span>
          )}
          <p className="field-help">
            Applies to terminals opened from now on. Agents of every space keep alerting.
          </p>
        </div>
        <footer>
          {space && (
            <button
              type="button"
              className="secondary space-delete"
              disabled={!empty}
              title={empty ? undefined : "Only a space without projects can be deleted"}
              onClick={() => void transport.deleteSpace(space.id)}
            >
              Delete space
            </button>
          )}
          <button type="button" className="secondary" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="primary">
            {editing ? "Save" : "Create space"}
          </button>
        </footer>
      </form>
    </dialog>
  );
}
