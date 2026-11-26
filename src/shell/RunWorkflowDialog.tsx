import { useState } from "react";
import { runWorkflow, type Workflow, type WorkflowInput } from "../git";
import { findWorktree, openModal, useHive } from "../store";
import { Select } from "../ui/Select";
import { CloseIcon } from "./icons";
import { showModal } from "./WorktreeMenu";

const close = () => openModal(null);

/** Each input's value as the form starts: its default, a choice's first option, else empty. */
export const startValues = (workflow: Workflow | undefined): Record<string, string> =>
  Object.fromEntries(
    (workflow?.inputs ?? []).map((i) => [i.name, i.default ?? (i.options[0] || "")]),
  );

/** One input of the workflow, as GitHub's own form shows it. */
function InputField(props: {
  input: WorkflowInput;
  value: string;
  onChange: (value: string) => void;
}) {
  const { input, value, onChange } = props;
  const id = `workflow-input-${input.name}`;
  const label = `${input.name}${input.required ? " *" : ""}`;
  if (input.kind === "boolean") {
    return (
      <label className="checkbox" title={input.description || undefined}>
        <input
          type="checkbox"
          checked={value === "true"}
          onChange={(e) => onChange(String(e.target.checked))}
        />
        {label}
      </label>
    );
  }
  return (
    <div className="field">
      {input.kind === "choice" ? (
        <span id={`${id}-label`}>{label}</span>
      ) : (
        <label htmlFor={id}>{label}</label>
      )}
      {input.kind === "choice" ? (
        <Select
          aria-labelledby={`${id}-label`}
          value={value}
          options={input.options.map((o) => ({ value: o, label: o }))}
          onChange={onChange}
        />
      ) : (
        <input
          id={id}
          value={value}
          inputMode={input.kind === "number" ? "decimal" : undefined}
          onChange={(e) => onChange(e.target.value)}
          spellCheck={false}
          autoComplete="off"
        />
      )}
      {input.description && <span className="hint">{input.description}</span>}
    </div>
  );
}

/**
 * "Run workflow…" (27.6, D27.3) from the Actions view: the project's workflows that can be run
 * by hand (`workflow_dispatch`, as the shown worktree's `HEAD` has them), the branch (the
 * worktree's at first) and the workflow's inputs. Run asks the service (`gh workflow run`); the
 * dialog closes once started and the Actions view follows the new run; `gh`'s refusal shows here.
 */
export function RunWorkflowDialog() {
  const project = useHive((s) => s.modalProject);
  const w = useHive((s) => findWorktree(s.projects, s.modalWorktree));
  const listed = useHive((s) => s.workflows);
  const busy = useHive((s) => s.workflowBusy);
  const error = useHive((s) => s.workflowError);
  const [picked, setPicked] = useState<string | null>(null);
  const [branch, setBranch] = useState(w?.branch ?? "");
  const [values, setValues] = useState<Record<string, string>>({});
  if (!project || !w) return null;
  const workflows = listed?.worktree === w.path ? listed.workflows : null;
  const workflow = workflows?.find((f) => f.file === picked) ?? workflows?.[0];
  const shown = { ...startValues(workflow), ...values };
  const missing = workflow?.inputs.some((i) => i.required && !shown[i.name]?.trim());
  const pick = (file: string) => {
    setPicked(file);
    setValues({});
  };
  return (
    <dialog
      className="dialog"
      aria-labelledby="run-workflow-title"
      ref={showModal("button.primary")}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (!workflow) return;
          const inputs = workflow.inputs.map((i) => ({ name: i.name, value: shown[i.name] ?? "" }));
          runWorkflow(project, workflow.file, branch.trim(), inputs);
        }}
      >
        <header>
          <h2 id="run-workflow-title">Run workflow</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          {!workflows && !listed?.error && <p className="hint">Reading the workflows…</p>}
          {listed?.error && (
            <span className="field-error" role="alert">
              {listed.error}
            </span>
          )}
          {workflows?.length === 0 && (
            <p className="hint">
              No workflow of this repository can be run by hand (workflow_dispatch).
            </p>
          )}
          {workflow && workflows && (
            <>
              <div className="field">
                <span id="run-workflow-pick">Workflow</span>
                <Select
                  aria-labelledby="run-workflow-pick"
                  value={workflow.file}
                  options={workflows.map((f) => ({
                    value: f.file,
                    label: `${f.name} (${f.file})`,
                  }))}
                  onChange={pick}
                />
              </div>
              <div className="field">
                <label htmlFor="run-workflow-branch">Branch</label>
                <input
                  id="run-workflow-branch"
                  value={branch}
                  onChange={(e) => setBranch(e.target.value)}
                  spellCheck={false}
                  autoComplete="off"
                />
              </div>
              {workflow.inputs.map((input) => (
                <InputField
                  key={`${workflow.file}:${input.name}`}
                  input={input}
                  value={shown[input.name] ?? ""}
                  onChange={(value) => setValues((v) => ({ ...v, [input.name]: value }))}
                />
              ))}
            </>
          )}
          {error && (
            <span className="field-error" role="alert">
              {error}
            </span>
          )}
        </div>
        <footer>
          <button type="button" className="secondary" onClick={close}>
            Cancel
          </button>
          <button
            type="submit"
            className="primary"
            disabled={busy || !workflow || !branch.trim() || missing}
          >
            {busy ? "Starting…" : "Run"}
          </button>
        </footer>
      </form>
    </dialog>
  );
}
