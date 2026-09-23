# UI reference: design tokens and glossary

> **Proposal pending human approval — stands in for the missing `docs/prototype/README.md`.**
> Extracted by an agent from `docs/prototype/HiveApp.dc.html`, `Hive Protótipo.dc.html` and `StateIcon.dc.html` (read as source). Decisions #23 (dark theme, Zed / One Dark look), #32 (prototype is the reference), #34 (UI in English, one glossary) and #35 (shortcuts) apply. Where the prototype and #35 disagree on shortcuts, #35 wins. Once the human approves, this file (or its content moved to `docs/prototype/README.md`) is the vocabulary for every screen.

The tokens live as CSS custom properties in `src/styles.css`.

## Design tokens

### Surfaces and text

| Token | Value | Use (prototype label) |
|---|---|---|
| `--bg` | `#282C33` | Terminal / background ("Terminal / fundo"), active tab, text fields |
| `--panel` | `#2F343E` | Sidebar, tab bar, right panel, dialogs ("Painel") |
| `--bar` | `#3B414D` | Title bar and status bar ("Barra de título / status") |
| `--hover` | `#363C46` | Hover ("Hover"); also the subtle border |
| `--selected` | `#454A56` | Selected row, active icon button, scrollbar thumb ("Selecionado") |
| `--selected-hover` | `#4F5563` | Hover on a selected-colored button ("New worktree" chip) |
| `--border` | `#464B57` | Borders ("Borda") |
| `--border-subtle` | `#363C46` | Inner separators ("sutil") |
| `--tree-line` | `#4A505C` | Subagent tree lines, Claude Code box border |
| `--focus` | `#47679E` | Focus ring ("Foco"): `inset 0 0 0 1px` or `0 0 0 1px` |
| `--accent` | `#74ADE8` | Primary action ("Ação primária") |
| `--accent-hover` | `#8BBBEE` | Primary button hover |
| `--on-accent` | `#1F2329` | Text on the primary button |
| `--text` | `#DCE0E5` | Text ("Texto") |
| `--text-2` | `#A9AFBC` | Secondary text ("Secundário") |
| `--text-3` | `#878A98` | Tertiary text ("Terciário") |
| `--text-4` | `#5D636F` | Breadcrumb separators, diff line numbers, kbd borders |
| `--close-hover` | `#C42B1C` | Window close button hover (text `#FFFFFF`) |

Other values seen: canvas behind the prototype `#1B1E23` (not used in the app), disabled primary button `#3E4A5C` with `--text-3` text, dialog backdrop `rgba(18,20,24,0.55)`, dialog shadow `0 16px 40px rgba(0,0,0,0.45)`, picker shadow `0 12px 32px rgba(0,0,0,0.4)`, pending chip `rgba(222,193,132,0.10)` background (`0.18` hover) and `0.28` border, sidebar resize hover `rgba(71,103,158,0.6)`.

### Agent states (color + shape, `StateIcon.dc.html`)

| Token | Color | Shape | Urgency |
|---|---|---|---|
| `--state-permission` | `#DEC184` | Triangle with "!" | high · alert (bell) |
| `--state-error` | `#D07277` | Circle with X | high · alert (bell) |
| `--state-you` | `#E08A5A` | Ring with dot | medium |
| `--state-working` | `#74ADE8` | Dot pulsing every 2.4 s | low |
| `--state-subagents` | `#B477CF` | Three nodes | low |
| `--state-idle` | `#A1C181` | Circle with check | none |
| `--state-ended` | `#878A98` | Rounded square | none |

Git status colors in the files panel: modified `#DEC184`, added `#A1C181`, deleted `#D07277` (strikethrough). Diff backgrounds: added `rgba(161,193,129,0.12)`, removed `rgba(208,114,119,0.12)`, hunk header `rgba(116,173,232,0.08)` with `#74ADE8` text.

### Typography

IBM Plex Sans and IBM Plex Mono, weights 400/500/600, bundled with `@fontsource/*` (no Google Fonts).

| Token | Size | Use |
|---|---|---|
| `--fs-title` | 15px / 500 | Empty-state title |
| `--fs-body` | 13px, line-height 18px | Base UI text; 13 / 500 for project names, 13 / 600 dialog title |
| `--fs-small` | 12px | Labels, metadata, bars, hints |
| `--fs-mono` | 12.5px mono | Worktree and branch names, tabs, text fields |
| `--fs-meta` | 11px | Time since, row badges, mono metadata |
| `--fs-kbd` | 10.5px mono | Keyboard hints (`<kbd>`-like chips) |
| — | 13px mono, line-height 19px | Terminal text |
| — | 11.5px | Small buttons, segmented control, picker footer |

### Radii and sizes

| Token | Value | Use |
|---|---|---|
| `--radius-sm` | 3px | Kbd chips, checkbox, branch list rows |
| `--radius` | 4px | Buttons, text fields |
| `--radius-md` | 6px | Cards, Claude Code boxes |
| `--radius-lg` | 8px | Dialogs, worktree picker |
| `--bar-h` | 34px | Title bar, sidebar header, tab bar, right panel header |
| `--status-h` | 24px | Status bar |
| `--sidebar-w` | 264px | Sidebar width (resizable 200–440 in the prototype) |
| `--right-w` | 380px | Right panel width (resizable 260–720 in the prototype) |

Row heights: project 26px, worktree 24px, sub-worktree 22px, file row 22px, picker row 30px, branch row 24px. Indents: project 6px, worktree 20px, agent 40px, subagent 56px. Buttons: primary/secondary 28px high (empty-state primary 30px), ghost 24px, icon 26×24, window buttons 46px wide. Text fields 30px. Dialog 520px wide, 120px from the top; picker 560px wide, 44px from the top.

## Glossary (PT → EN)

### Core terms

| Portuguese | English | Notes |
|---|---|---|
| projeto | project | A folder inside WSL |
| worktree | worktree | Not translated |
| agente | agent | Rows show the agent as "Claude" |
| subagente | subagent | Row prefix "subagent:" |
| terminal | terminal | |
| branch | branch | |
| pendente / pendência | pending | An agent in a high-urgency state |
| painel direito | right panel | |
| barra lateral | sidebar | |
| barra de título | title bar | |
| barra de status | status bar | |
| aba | tab | |
| padrão / principal (branch) | default | Badge next to the default branch |

### Agent states

| Portuguese | English (label) |
|---|---|
| aguardando permissão | waiting for permission |
| erro | error |
| aguardando você | waiting for you |
| trabalhando | working |
| com subagentes | running subagents |
| ocioso | idle |
| encerrado | ended |

Labels are lowercase in rows (as in the prototype), sentence case in headings and legends.

### Title bar and status bar

| Portuguese | English |
|---|---|
| Minimizar | Minimize |
| Maximizar | Maximize |
| Fechar | Close |
| WSL: Ubuntu | WSL: Ubuntu |
| conectado | connected |
| (new) conectando | connecting |
| (new) versão incompatível | version mismatch |
| Distribuição WSL conectada (tooltip) | WSL connection |
| N agente ativo / N agentes ativos | 1 active agent / N active agents |

### Sidebar

| Portuguese | English |
|---|---|
| N pendente / N pendentes | 1 pending / N pending |
| Ir para o próximo agente pendente (F8) | Go to the next pending agent (F8) |
| Nenhuma pendência | Nothing pending |
| Projeto (button) | Project |
| Adicionar projeto (tooltip) | Add project (Ctrl+Shift+O) |
| Nenhum projeto | No projects |
| Nova worktree (Alt+N) | New worktree (Ctrl+Shift+N) |
| subagente: | subagent: |
| 14 min · 1 h · agora | 14 min · 1 h · now |
| Arraste para redimensionar | Drag to resize |

### Terminal area

| Portuguese | English |
|---|---|
| Novo terminal (Alt+T) | New terminal (Ctrl+Shift+T) |
| Fechar terminal | Close terminal |
| Arquivos e diff (Alt+B) | Files and diff (Ctrl+Shift+B) |

### Worktree picker (Ctrl+Shift+T)

| Portuguese | English |
|---|---|
| Abrir terminal na worktree… | Open a terminal in worktree… |
| Nenhuma worktree encontrada | No worktrees found |
| navegar | navigate |
| abrir terminal | open terminal |
| fechar | close |

### Empty state (screen 1e)

| Portuguese | English |
|---|---|
| Nenhum projeto aberto | No project open |
| Adicione um projeto para acompanhar os agentes que rodam nas worktrees dele. | Add a project to follow the agents running in its worktrees. |
| Adicionar projeto · Ctrl O | Add project · Ctrl+Shift+O |
| O projeto é uma pasta dentro do WSL, por exemplo: | A project is a folder inside WSL, for example: |
| /home/usuario/projetos/loja | /home/user/projects/shop |

### Right panel: files and diff (screen 1g, Stage 3)

| Portuguese | English |
|---|---|
| Arquivos e diff | Files and diff |
| Todos | All |
| Mostrar todos os arquivos | Show all files |
| Alterados | Changed |
| Mostrar só arquivos alterados | Show only changed files |
| Recolher (Alt+B) | Collapse (Ctrl+Shift+B) |
| N arquivo alterado / N arquivos alterados | 1 file changed / N files changed |
| Sem alterações | No changes |
| Nenhuma alteração nesta worktree. | No changes in this worktree. |
| Fechar diff | Close diff |
| Sem alterações neste arquivo. | No changes in this file. |
| Selecione um projeto ou agente para ver os arquivos. | Select a project or agent to see its files. |
| M / A / D | M / A / D (git status letters, unchanged) |

### New worktree dialog (screens 1c/1d)

| Portuguese | English |
|---|---|
| Nova worktree | New worktree |
| Fechar (Esc) | Close (Esc) |
| Projeto | Project |
| Nome da worktree | Worktree name |
| ex.: fix-carrinho | e.g. fix-cart |
| Use apenas letras minúsculas, números, hífen, ponto ou sublinhado. | Use only lowercase letters, digits, hyphens, dots or underscores. |
| Já existe uma worktree com esse nome. | A worktree with this name already exists. |
| Branch de origem | Base branch |
| Filtrar branches locais e remotas | Filter local and remote branches |
| Locais | Local |
| Remotas | Remote |
| principal | default |
| Nenhuma branch encontrada | No branches found |
| Abrir terminal na nova worktree | Open a terminal in the new worktree |
| Pasta: | Folder: |
| Branch: | Branch: |
| (a partir de main) | (from main) |
| \<nome\> | \<name\> |
| Cancelar · Esc | Cancel · Esc |
| Criar worktree · Enter | Create worktree · Enter |

The validation messages must match what `hive worktree create` prints (#33, TODO 1.6); if the CLI wording differs, the CLI wording wins and this table is updated.

### Style sheet labels (screen 1f, reference only)

Superfícies e texto → Surfaces and text · Estados do agente — cor + forma → Agent states: color + shape · Tipografia → Typography · Componentes base → Base components · Item da árvore → Tree item · Aba → Tab · Botão → Button · Campo de texto → Text field · primário · secundário · fantasma · ícone → primary · secondary · ghost · icon · padrão · foco · erro → default · focus · error · ativa · hover · inativa → active · hover · inactive.

Terminal content in the prototype (Claude Code output, sample code) is sample data and is not translated.

## Shortcuts (#35, not the prototype)

| Action | Shortcut | Prototype had |
|---|---|---|
| Next pending agent | F8 | F8 |
| Worktree picker → new terminal | Ctrl+Shift+T | Alt+T |
| New worktree | Ctrl+Shift+N | Alt+N |
| Files and diff panel | Ctrl+Shift+B | Alt+B |
| Add project | Ctrl+Shift+O | Ctrl+O |
| Copy / paste in the terminal | Ctrl+Shift+C / Ctrl+Shift+V | — |
| Move in the tree | ↑ ↓ ← → Enter | same |

Tooltips show the shortcut in parentheses, e.g. "Files and diff (Ctrl+Shift+B)"; buttons show it as a kbd chip (e.g. "Ctrl+Shift+O").
