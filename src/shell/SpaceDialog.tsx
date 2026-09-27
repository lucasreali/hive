import { useEffect, useState } from "react";
import { ask, currentSpace, type GhAccount, openModal, type SpaceEnv, useHive } from "../store";
import { transport } from "../transport";
import { Select } from "../ui/Select";
import { CloseIcon } from "./icons";

const close = () => openModal(null);

type TextKey = Exclude<keyof SpaceEnv, "gh_account">;

/** The environment fields, in the dialog's order: what each sets in the space's terminals. */
const FIELDS: { key: TextKey; label: string; placeholder: string; help: string }[] = [
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
    help: "GH_CONFIG_DIR: a separate gh config, where the account below is looked up.",
  },
];

const NO_ENV: SpaceEnv = {
  claude_config_dir: null,
  git_name: null,
  git_email: null,
  gh_config_dir: null,
  gh_account: null,
};

/** An account's value in the select; "" is gh's active account. */
const accountKey = (a: GhAccount) => `${a.login}@${a.host}`;
const accountName = (a: GhAccount) =>
  a.host === "github.com" ? a.login : `${a.login} on ${a.host}`;

/**
 * The space's GitHub account (9.30): one of the accounts logged in to `gh` (in the config folder
 * above, when set), or `gh`'s active account. The service lists them (logins only, never a
 * token) when the dialog opens and when the config folder field is left. "Make active in gh…"
 * switches `gh`'s own active account, for every shell on the machine: it asks first.
 */
function GhAccountField({ env, setEnv }: { env: SpaceEnv; setEnv: (env: SpaceEnv) => void }) {
  const answer = useHive((s) => s.ghAccounts);
  const modal = useHive((s) => s.modal);
  const dir = env.gh_config_dir;
  // An answer for another folder (the field changed since) is not this one's.
  const listed = answer?.gh_config_dir === dir ? answer : null;
  const accounts = listed?.accounts ?? [];
  const active = accounts.filter((a) => a.active).map(accountName);
  const chosen = env.gh_account;
  const known = chosen && accounts.find((a) => accountKey(a) === accountKey(chosen));
  const options = [
    { value: "", label: `gh's active account${active.length ? ` (${active.join(", ")})` : ""}` },
    ...accounts.map((a) => ({
      value: accountKey(a),
      label: `${accountName(a)}${a.logged_in ? "" : " (token invalid)"}`,
    })),
  ];
  if (chosen && !known) {
    options.push({ value: accountKey(chosen), label: `${accountName(chosen)} (not logged in)` });
  }
  const pick = (value: string) => {
    const account = accounts.find((a) => accountKey(a) === value);
    setEnv({ ...env, gh_account: account ? { host: account.host, login: account.login } : null });
  };
  const makeActive = (account: GhAccount) =>
    ask({
      title: "Switch gh's active account?",
      text: `${accountName(account)} becomes gh's active account on ${account.host} for every shell and repository on this machine, outside Hive too.`,
      action: "Switch",
      run: () => void transport.switchGhAccount(dir, account),
      back: modal,
    });
  return (
    <div className="field">
      <span id="space-gh-account-label">GitHub account</span>
      <div className="gh-account">
        <Select
          aria-labelledby="space-gh-account-label"
          value={chosen ? accountKey(chosen) : ""}
          options={options}
          onChange={pick}
        />
        {known && !known.active && (
          <button
            type="button"
            className="secondary"
            onClick={() => makeActive({ host: known.host, login: known.login })}
          >
            Make active in gh…
          </button>
        )}
      </div>
      {listed?.problem && (
        <p className="field-help" role="status">
          {listed.problem}
        </p>
      )}
      <p className="field-help">
        GH_TOKEN and GH_HOST: the account the space's terminals and Hive's gh calls use, without
        changing gh's active account.
      </p>
    </div>
  );
}

/**
 * Creates a space ("new-space") or edits the current one ("edit-space"): its name and what its
 * new terminals get in their environment; an empty field leaves the user's own. The service
 * checks everything and answers `spaces` (the dialog closes) or why it refused (shown).
 */
export function SpaceDialog({ editing }: { editing: boolean }) {
  const space = useHive((s) => (editing ? currentSpace(s) : undefined));
  const error = useHive((s) => s.spaceError);
  const [name, setName] = useState(space?.name ?? "");
  const [env, setEnv] = useState<SpaceEnv>({ ...NO_ENV, ...space?.env });
  const title = editing ? "Edit space" : "New space";
  const empty = space?.projects.length === 0;
  const listGh = (dir: string | null) => void transport.listGhAccounts(dir);
  // Listed when it opens, and again when the folder field is left (`onBlur` below).
  const [openedDir] = useState(env.gh_config_dir);
  useEffect(() => void transport.listGhAccounts(openedDir), [openedDir]);
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
                onBlur={f.key === "gh_config_dir" ? () => listGh(env.gh_config_dir) : undefined}
                placeholder={f.placeholder}
                spellCheck={false}
                autoComplete="off"
              />
              <p className="field-help">{f.help}</p>
            </div>
          ))}
          <GhAccountField env={env} setEnv={setEnv} />
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
