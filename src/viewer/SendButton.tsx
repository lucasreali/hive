import { type ReactNode, useCallback, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Agent } from "../protocol";
import { STATE_LABEL, StateIcon } from "../shell/icons";
import { ContextMenu } from "../shell/WorktreeMenu";
import { useHive } from "../store";
import { agentLabel, agentsIn, NO_AGENT, savedPick, savePick } from "./target";

/** One agent of the picker: its state's icon, its tab's title, its state and activity. */
function AgentItem({ agent, onPick }: { agent: Agent; onPick: () => void }) {
  const label = useHive((s) => agentLabel(s, agent));
  const status = useHive((s) => s.agentStates[agent.id]);
  const state = status?.state ?? "idle";
  const meta = [STATE_LABEL[state], status?.activity].filter(Boolean).join(" · ");
  return (
    <button type="button" role="menuitem" onClick={onPick}>
      <StateIcon state={state} />
      <span className="label">{label}</span>
      <span className="inbox-meta">{meta}</span>
    </button>
  );
}

/**
 * A button that sends to an agent of `worktree` (28.21): with one agent, to it; with several, a
 * small menu by the button lists them, the one last picked there first. Disabled with the reason
 * (`why`, else "No agent in this worktree") when nothing can be sent.
 */
export function SendButton(props: {
  worktree: string;
  why: string | null;
  title: string;
  label?: string;
  className: string;
  onSend: (terminal: number) => void;
  children: ReactNode;
}) {
  const { worktree, onSend } = props;
  const agents = useHive(useShallow((s) => agentsIn(s, worktree)));
  const why = props.why ?? (agents.length === 0 ? NO_AGENT : null);
  const [button, setButton] = useState<HTMLButtonElement | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; agents: Agent[] } | null>(null);
  const close = useCallback(() => setMenu(null), []);
  const several = agents.length > 1;
  const click = () => {
    if (menu) return close();
    const [only] = agents;
    if (!several && only) return onSend(only.terminal);
    const box = (button as HTMLButtonElement).getBoundingClientRect();
    const picked = savedPick(worktree);
    const first = agents.filter((a) => a.id === picked);
    const listed = [...first, ...agents.filter((a) => a.id !== picked)];
    setMenu({ x: box.left, y: box.bottom + 4, agents: listed });
  };
  const pick = (agent: Agent) => () => {
    close();
    savePick(worktree, agent.id);
    onSend(agent.terminal);
  };
  return (
    <>
      <button
        ref={setButton}
        type="button"
        className={props.className}
        aria-label={props.label}
        aria-haspopup={several ? "menu" : undefined}
        aria-expanded={several ? menu !== null : undefined}
        title={why ?? props.title}
        disabled={why !== null}
        onClick={click}
      >
        {props.children}
      </button>
      {menu && (
        <ContextMenu
          at={menu}
          label="Send to agent"
          onClose={close}
          anchor={button}
          className="send-menu"
        >
          {menu.agents.map((a) => (
            <AgentItem key={a.id} agent={a} onPick={pick(a)} />
          ))}
        </ContextMenu>
      )}
    </>
  );
}
