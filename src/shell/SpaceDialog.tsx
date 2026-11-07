import { type Dispatch, type SetStateAction, useEffect, useState } from "react";
import type { GhAccount, GhAccounts, GhIdentity, SpaceEnv, SpaceGhAccount } from "../protocol";
import { ask, currentSpace, openModal, spaceAccount, useHive } from "../store";
import { transport } from "../transport";
import { Select } from "../ui/Select";
import { CloseIcon } from "./icons";

const close = () => openModal(null);

type TextKey = Exclude<keyof SpaceEnv, "gh_account" | "claude_account">;

/** The environment fields, in the dialog's order: what each sets in the space's terminals. */
const FIELDS: { key: TextKey; label: string; placeholder: string; help: string }[] = [
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
];

const NO_ENV: SpaceEnv = {
  git_name: null,
  git_email: null,
  gh_account: null,
  claude_account: null,
};

const accountKey = (a: GhAccount) => `${a.login}@${a.host}`;
/** A space's account's value in the select (its folder too, 24.13); "" is gh's active account. */
const spaceKey = (a: GhAccount, dir: string | null) => `${accountKey(a)} ${dir ?? ""}`;
export const accountName = (a: GhAccount) =>
  a.host === "github.com" ? a.login : `${a.login} on ${a.host}`;
/** A gh config folder by its last segment (`gh-work`), as the select names it. */
const folderName = (dir: string) => dir.split(/[\\/]/).pop() || dir;

/**
 * The accounts of every gh config folder (24.13): `gh`'s own (null), then the settings' extra
 * ones, each with its last `gh_accounts` answer (undefined until it came). Asked when shown and
 * whenever the folders change, never polled: `gh` checks each token with GitHub.
 */
export function useGhAccounts(): { dir: string | null; listed: GhAccounts | undefined }[] {
  const dirs = useHive((s) => s.settings.gh.config_dirs);
  const answers = useHive((s) => s.ghAccounts);
  const asked = dirs.join("\n");
  // biome-ignore lint/correctness/useExhaustiveDependencies: `asked` says when to ask again.
  useEffect(() => {
    for (const dir of [null, ...dirs]) void transport.listGhAccounts(dir);
  }, [asked]);
  return [null, ...dirs].map((dir) => ({ dir, listed: answers[dir ?? ""] }));
}

/**
 * The space's GitHub account (9.30): one of the accounts logged in to `gh`, in its own config or
 * in one of the gh config folders of Settings → Accounts (24.13), or `gh`'s active account. The
 * service lists them (logins only, never a token) when the dialog opens. "Make active in gh…"
 * switches `gh`'s own active account in the account's folder, for every shell on the machine: it
 * asks first.
 */
function GhAccountField({ env, setEnv }: { env: SpaceEnv; setEnv: (env: SpaceEnv) => void }) {
  const folders = useGhAccounts();
  const modal = useHive((s) => s.modal);
  const accounts = folders.flatMap(({ dir, listed }) =>
    (listed?.accounts ?? []).map((a) => ({ ...a, config_dir: dir })),
  );
  const active = accounts.filter((a) => a.active && a.config_dir === null).map(accountName);
  const chosen = env.gh_account;
  const key = (a: SpaceGhAccount) => spaceKey(a, a.config_dir);
  const known = chosen && accounts.find((a) => key(a) === key(chosen));
  const named = (a: SpaceGhAccount) =>
    `${accountName(a)}${a.config_dir === null ? "" : ` (${folderName(a.config_dir)})`}`;
  const options = [
    { value: "", label: `gh's active account${active.length ? ` (${active.join(", ")})` : ""}` },
    ...accounts.map((a) => ({
      value: key(a),
      label: `${named(a)}${a.logged_in ? "" : " (token invalid)"}`,
    })),
  ];
  if (chosen && !known) {
    options.push({ value: key(chosen), label: `${named(chosen)} (not logged in)` });
  }
  const pick = (value: string) => {
    const account = accounts.find((a) => key(a) === value);
    setEnv({
      ...env,
      gh_account: account
        ? { host: account.host, login: account.login, config_dir: account.config_dir }
        : null,
    });
  };
  const makeActive = (account: SpaceGhAccount) =>
    ask({
      title: "Switch gh's active account?",
      text: `${accountName(account)} becomes gh's active account on ${account.host} for every shell and repository on this machine, outside Hive too.`,
      action: "Switch",
      run: () =>
        void transport.switchGhAccount(account.config_dir, {
          host: account.host,
          login: account.login,
        }),
      back: modal,
    });
  const problems = folders.filter(({ listed }) => listed?.problem);
  return (
    <div className="field">
      <span id="space-gh-account-label">GitHub account</span>
      <div className="gh-account">
        <Select
          aria-labelledby="space-gh-account-label"
          value={chosen ? key(chosen) : ""}
          options={options}
          onChange={pick}
        />
        {known && !known.active && (
          <button type="button" className="secondary" onClick={() => makeActive(known)}>
            Make active in gh…
          </button>
        )}
      </div>
      {problems.map(({ dir, listed }) => (
        <p className="field-help" role="status" key={dir ?? ""}>
          {dir === null ? listed?.problem : `${dir}: ${listed?.problem}`}
        </p>
      ))}
      <p className="field-help">
        GH_TOKEN and GH_HOST, and GH_CONFIG_DIR for an account of another gh config folder: the
        account the space's terminals and Hive's gh calls use, without changing gh's active account.
        Folders are added in Settings → Accounts.
      </p>
    </div>
  );
}

/**
 * The space's Claude account (22.5, D22.15): one of the settings' accounts, kept as its folder so
 * a rename keeps it; "" is the default account.
 */
function ClaudeAccountField({ env, setEnv }: { env: SpaceEnv; setEnv: (env: SpaceEnv) => void }) {
  const { accounts, default_name } = useHive((s) => s.settings.claude);
  const options = [
    { value: "", label: default_name },
    ...accounts.map((a) => ({ value: a.config_dir, label: a.name })),
  ];
  return (
    <div className="field">
      <span id="space-claude-account-label">Claude account</span>
      <Select
        aria-labelledby="space-claude-account-label"
        value={env.claude_account ?? ""}
        options={options}
        onChange={(value) => setEnv({ ...env, claude_account: value || null })}
      />
      <p className="field-help">
        CLAUDE_CONFIG_DIR: the account the space's terminals run Claude as. Accounts are added in
        Settings → Accounts.
      </p>
    </div>
  );
}

/**
 * A space with a GitHub account but no git name or email (21.5, D22.8) commits as git's own
 * guess while it pushes as the account: says so, and "Use the GitHub account's" asks the service
 * for the account's login and noreply email, which fill the empty fields.
 */
function IdentityWarning({
  env,
  setEnv,
}: {
  env: SpaceEnv;
  setEnv: Dispatch<SetStateAction<SpaceEnv>>;
}) {
  const answer = useHive((s) => s.ghIdentity);
  // The answer there was when asked: only a newer one, for the same account, is this request's.
  const [asked, setAsked] = useState<{ account: GhAccount; before: GhIdentity | null } | null>(
    null,
  );
  const account = env.gh_account;
  // Asked for the account chosen now (another one chosen since: not asked).
  const asking =
    asked && account && accountKey(asked.account) === accountKey(account) ? asked : null;
  const mine =
    asking &&
    answer &&
    answer !== asking.before &&
    accountKey(answer.account) === accountKey(asking.account)
      ? answer
      : null;
  useEffect(() => {
    if (!mine || !("name" in mine)) return;
    const { name, email } = mine;
    setEnv((env) => ({
      ...env,
      git_name: env.git_name ?? name,
      git_email: env.git_email ?? email,
    }));
    setAsked(null);
  }, [mine, setEnv]);
  if (!account || (env.git_name && env.git_email)) return null;
  const pending = asking !== null && mine === null;
  const use = () => {
    setAsked({ account, before: useHive.getState().ghIdentity });
    void transport.getGhIdentity(account.config_dir, { host: account.host, login: account.login });
  };
  return (
    <div className="field">
      <p className="field-warning">
        Commits will use git's own guess: this space has a GitHub account but no git{" "}
        {env.git_name ? "email" : env.git_email ? "name" : "name or email"}.
      </p>
      <div>
        <button type="button" className="secondary" disabled={pending} onClick={use}>
          Use the GitHub account's
        </button>
      </div>
      {mine && "message" in mine && (
        <span className="field-error" role="alert">
          {mine.message}
        </span>
      )}
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
  // A Claude account no account has any more is the default one, saved as such.
  const account = useHive((s) => spaceAccount(s, space));
  const [env, setEnv] = useState<SpaceEnv>({ ...NO_ENV, ...space?.env, claude_account: account });
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
          <ClaudeAccountField env={env} setEnv={setEnv} />
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
          <GhAccountField env={env} setEnv={setEnv} />
          <IdentityWarning env={env} setEnv={setEnv} />
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
