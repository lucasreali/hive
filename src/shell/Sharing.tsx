import { useEffect, useState } from "react";
import type { Person, ShareRole, Sharing } from "../protocol";
import { type SharingView, useHive } from "../store";
import { transport } from "../transport";
import { Select } from "../ui/Select";
import { CloseIcon } from "./icons";

// Settings → Sharing (29.12, `docs/spike/multiplayer.md` §3): a remote Linux host's own Hive
// shared with teammates who log in to that host as their own users. Everything here is that
// host's service's: it sends what it has (`sharing`), checks a save and answers with the result
// or why it refused (`sharing_failed`). The UI only edits a draft and sends it whole.

const ROLES: { value: ShareRole; label: string }[] = [
  { value: "viewer", label: "Viewer" },
  { value: "collaborator", label: "Collaborator" },
];

/** A new row of the people list. */
const NEW_PERSON: Person = { login: "", display: "", git_name: "", git_email: "", role: "viewer" };

/**
 * The section: the remote host to share (only the user's own, never a joined Hive), and that
 * host's Sharing settings once its service sent them.
 */
export function SharingSettings() {
  const hosts = useHive((s) => s.settings.remote.hosts);
  const [picked, setPicked] = useState<string | null>(null);
  const host = picked !== null && hosts.includes(picked) ? picked : (hosts[0] ?? null);
  const connected = useHive((s) => host !== null && s.links[host]?.state === "connected");
  const view = useHive((s) => (host === null ? undefined : s.sharing[host]));
  useEffect(() => {
    if (host !== null && connected) void transport.getSharing(host);
  }, [host, connected]);
  return (
    <section className="field settings-group" aria-labelledby="sharing-title">
      <h3 id="sharing-title">Sharing</h3>
      <p className="field-help">
        Share a remote host's Hive with teammates who log in to it as their own users, in a Unix
        group you are in. They see only the spaces you share, and what they run, runs as you.
      </p>
      {host === null ? (
        <p className="field-help">Add a remote host in Hosts first: only its Hive can be shared.</p>
      ) : (
        <>
          {hosts.length > 1 && (
            <div className="field">
              <span id="sharing-host-label">Host</span>
              <Select
                aria-labelledby="sharing-host-label"
                value={host}
                options={hosts.map((h) => ({ value: h, label: h }))}
                onChange={setPicked}
              />
            </div>
          )}
          {!connected ? (
            <p className="field-help">{host} is not connected.</p>
          ) : view === undefined ? (
            <p className="field-help">Asking {host}…</p>
          ) : (
            // A new save starts the draft again from what the service saved; a refusal keeps it.
            <SharingForm key={`${host} ${JSON.stringify(view.sharing)}`} host={host} view={view} />
          )}
        </>
      )}
    </section>
  );
}

/** `host`'s Sharing settings as a draft, saved whole on Save. */
export function SharingForm({ host, view }: { host: string; view: SharingView }) {
  const [draft, setDraft] = useState<Sharing>(view.sharing);
  const change = (next: Partial<Sharing>) => setDraft({ ...draft, ...next });
  const person = (i: number, next: Partial<Person>) =>
    change({ people: draft.people.map((p, at) => (at === i ? { ...p, ...next } : p)) });
  const changed = JSON.stringify(draft) !== JSON.stringify(view.sharing);
  if (view.unavailable !== null) return <p className="field-help">{view.unavailable}.</p>;
  const groups = [
    { value: "", label: "Pick a group", disabled: true },
    ...view.groups.map((g) => ({ value: g, label: g })),
  ];
  return (
    <form
      className="sharing"
      onSubmit={(e) => {
        e.preventDefault();
        void transport.setSharing(host, draft);
      }}
    >
      <p className="field-help">
        Teammates join with Hosts → Join someone's Hive, giving your login name{" "}
        <code>{view.owner}</code>.
      </p>
      {view.error !== null && (
        <p className="field-error" role="alert">
          Not shared: {view.error}
        </p>
      )}
      <label className="checkbox">
        <input
          type="checkbox"
          checked={draft.on}
          onChange={(e) => change({ on: e.target.checked })}
        />
        Share this Hive
      </label>
      <div className="field">
        <span id="sharing-group-label">Unix group</span>
        <Select
          aria-labelledby="sharing-group-label"
          value={draft.group}
          options={groups}
          onChange={(group) => change({ group })}
        />
      </div>
      <fieldset className="field sharing-spaces">
        <legend>Spaces teammates see</legend>
        {view.spaces.map((space) => (
          <label className="checkbox" key={space.id}>
            <input
              type="checkbox"
              checked={draft.spaces.includes(space.id)}
              onChange={(e) =>
                change({
                  spaces: e.target.checked
                    ? [...draft.spaces, space.id]
                    : draft.spaces.filter((id) => id !== space.id),
                })
              }
            />
            {space.name}
          </label>
        ))}
      </fieldset>
      <fieldset className="field sharing-people">
        <legend>People</legend>
        {draft.people.length === 0 && <p className="field-help">Nobody can join yet.</p>}
        {draft.people.map((p, i) => (
          // The rows have no id of their own: a login may be typed or changed.
          // biome-ignore lint/suspicious/noArrayIndexKey: rows are edited in place.
          <div className="sharing-person" key={i}>
            <input
              className="sharing-login"
              aria-label={`Login name ${i + 1}`}
              placeholder="Login name"
              value={p.login}
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              onChange={(e) => person(i, { login: e.currentTarget.value })}
            />
            <input
              className="sharing-display"
              aria-label={`Display name ${i + 1}`}
              placeholder="Display name"
              value={p.display}
              onChange={(e) => person(i, { display: e.currentTarget.value })}
            />
            <input
              className="sharing-git-name"
              aria-label={`Git name ${i + 1}`}
              placeholder="Git name"
              value={p.git_name}
              onChange={(e) => person(i, { git_name: e.currentTarget.value })}
            />
            <input
              className="sharing-git-email"
              aria-label={`Git e-mail ${i + 1}`}
              placeholder="Git e-mail"
              value={p.git_email}
              spellCheck={false}
              autoCapitalize="off"
              onChange={(e) => person(i, { git_email: e.currentTarget.value })}
            />
            <Select
              className="sharing-role"
              aria-label={`Role ${i + 1}`}
              value={p.role}
              options={ROLES}
              onChange={(role) => person(i, { role: role as ShareRole })}
            />
            <button
              type="button"
              className="ghost sharing-remove"
              aria-label={`Remove ${p.login || `person ${i + 1}`}`}
              title="Remove"
              onClick={() => change({ people: draft.people.filter((_, at) => at !== i) })}
            >
              <CloseIcon />
            </button>
          </div>
        ))}
        <button
          type="button"
          className="secondary"
          onClick={() => change({ people: [...draft.people, NEW_PERSON] })}
        >
          Add person
        </button>
      </fieldset>
      {view.failed !== null && (
        <p className="field-error" role="alert">
          {view.failed}
        </p>
      )}
      <div className="sharing-actions">
        <button type="submit" className="primary" disabled={!changed}>
          Save
        </button>
      </div>
    </form>
  );
}
