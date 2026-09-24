# Hive — Documento de Decisões do Projeto

> Versão 3.3. Execução noturna autônoma abandonada; desenvolvimento incremental guiado pelo `TODO.md`. Substitui a versão 3.2.

---

## 🤖 Prompt de inicialização (leia primeiro)

Você vai me ajudar a **continuar o refinamento do projeto Hive**, uma ferramenta desktop para acompanhar visualmente meus agentes de IA (Claude Code). A ideação e duas rodadas de refinamento já foram feitas; tudo que foi decidido está neste documento. Seu papel é refinar, questionar e aprofundar, não recomeçar do zero.

**Como quero que você se comporte:**

1. **Não concorde comigo sempre.** Se uma ideia minha for ruim, arriscada ou tiver uma alternativa melhor, diga claramente. Concordar por educação não me ajuda.
2. **Sempre avalie os dois lados da moeda.** Para cada decisão relevante, apresente prós e contras, inclusive das decisões já marcadas como "decidido", se você enxergar um problema real nelas.
3. **Seja direto e organizado.** Respostas curtas, bem estruturadas, sem blocos enormes de texto. Prefiro tabelas e listas curtas a parágrafos longos.
4. **Uma pergunta por vez.** Quando precisar de uma resposta minha, faça uma pergunta de cada vez.
5. **Mantenha a lista atualizada.** Ao final de cada resposta em que algo for decidido ou mudar, mostre as linhas alteradas das listas deste documento, no mesmo formato.
6. **Verifique informações que mudam rápido.** Claude Code, hooks, flags de CLI e ferramentas mudam com frequência; confirme antes de afirmar.
7. **Não reproponha decisões revogadas** (seção "Decisões revogadas") sem um motivo novo.

**Contexto sobre mim:** sou desenvolvedor, uso Windows com WSL, meus projetos ficam no sistema de arquivos do WSL, uso **fish** como shell no WSL, tenho experiência com **React**, uso muito worktrees e costumo trabalhar com um agente que recebe várias tarefas e distribui cada uma para um subagente. Rodo **muitos agentes em paralelo, sem limite definido**: todos precisam funcionar. Este é um projeto pessoal, feito para minha satisfação como desenvolvedor, não um produto comercial.

**Situação atual:** todas as decisões das Etapas 0 a 4 foram tomadas. O desenvolvimento é **incremental e guiado** (seção "Regras de desenvolvimento"): o agente segue o `TODO.md` do repositório, uma tarefa por vez, com checkpoints de revisão humana. O protótipo da Etapa 1 está em `docs/prototype/`; falta exportar as capturas das telas (ponto 13).

**Primeiro passo sugerido:** leia o documento inteiro, aponte qualquer inconsistência ou risco e depois me ajude a revisar o protótipo da Etapa 1 ou a planejar a implementação da Etapa 0.

---

## 🏷️ Nome

**Hive** *(decidido)*. Existe outro projeto com a mesma ideia e o mesmo nome (FedorenkoCodes).

**Comando da CLI: `hive`** *(decidido)*. Um binário só, com subcomandos em inglês. Colide em tese com o Apache Hive e com o projeto do FedorenkoCodes, mas hooks e app sempre chamam o binário por **caminho absoluto**, então uma colisão futura afeta só o uso manual.

---

## 🧭 Conceito central

O Hive é **igual ao Orca neste ponto**: tem **terminais embutidos**, e eu rodo o `claude` interativo dentro deles. O Hive **apenas observa** os agentes abertos nos seus terminais e mostra o estado de cada um. Ele **não inicia, não controla e não conversa** com os agentes. Toda interação com o agente acontece no terminal.

Consequências:

- **Sem Claude Agent SDK.** O uso é o `claude` normal, dentro dos limites da assinatura.
- O Hive é um **companheiro do terminal**, não um substituto.
- Só aparecem no Hive as sessões abertas **nos terminais do Hive**. Sessões no Windows Terminal ficam fora.
- **Terminais e agentes vivem enquanto o app estiver aberto.** Fechar o app (ou um crash) encerra todos.
- O Hive **pode editar arquivos** (Etapa 3b), mas nunca age sobre o agente.

---

## 🚫 Requisitos obrigatórios

1. **App desktop.**
2. **Conexão 100% funcional com WSL**: agentes, git e projetos rodando dentro do WSL.
3. **Performance**: app leve e rápido, **com qualquer número de terminais abertos**. Motivação: o Orca (Electron + TypeScript) é pesado na minha máquina.
4. **Worktrees**: cada agente trabalha isolado na sua própria git worktree.
5. **CLI de worktrees**: toda worktree **criada nos terminais do Hive** (pelo app, pelo `claude -w` ou por subagentes) passa pela CLI do Hive. **A CLI não cria agentes.**
6. **Sistema adaptado à customização**: desenvolvido inicialmente para o Claude Code, com código orientado a reuso para suportar outros provedores. Cada adaptador é **um tradutor de hooks/eventos** do provedor para o modelo interno.

---

## 📋 Lista do projeto

### ✅ Manter (inspirado no Orca)

1. **Estado dos agentes na barra lateral** *(visão principal)*: hierarquia **Projeto → Worktree → Agente → Subagentes**, com indentação, como o Orca faz com subagentes.
2. **Terminais embutidos** com abas; os agentes abertos neles são listados no app.
3. **Árvore de arquivos com diff do git**: ver o que o agente criou, alterou ou apagou.

### 🔧 Melhorar / mudar

1. **Árvore de arquivos em tempo real** *(decidido)*: atualiza conforme o agente trabalha.
2. **Escolha da branch de origem** ao criar uma worktree.
3. **Subagentes na barra lateral** *(decidido)*: indentados sob o agente pai, com estado próprio; quando o subagente tem worktree própria, ela aparece indentada sob ele (e não se repete no nível do projeto). Base técnica: hooks `SubagentStart`/`SubagentStop` e hooks `WorktreeCreate`/`WorktreeRemove`.
4. **Estado propagado + contador de pendências** *(decidido)*: projeto ou worktree recolhido mostra o estado mais urgente dentro dele; contador "🚨 N pendentes" no topo leva ao próximo agente que precisa de ação. **Pendente = 🟡, 🔴 ou 🟠** (checkpoint 2); o serviço decide e manda `pending`/`urgency`. Nó recolhido mostra o **ícone do estado mais urgente**, sem o sino do protótipo. Compensa a ausência do kanban.
5. **Notificação** *(decidido)*: som ao entrar em 🟡, 🟠 ou 🔴; notificação do sistema operacional quando o agente termina (🔵/🟣 → 🟠). **Sempre**, mesmo com a janela em foco, sem botão de mudo (checkpoint 2; rever se incomodar).
6. **Edição de arquivos** *(decidido)*: editar no visualizador, salvando pelo serviço com verificação de versão; sem LSP na v1; diff somente leitura. *(Etapa 3b)*

### ✨ Inspirado em outros apps

1. **Edição ao vivo, estilo Zed**: ver qual arquivo o agente está alterando, no momento em que acontece. *(Fase 2)*
2. **Histórico de sessões, estilo opcode**: ler `~/.claude/projects/` para ver conversas antigas. *(Fase 2)*
3. **Convenção de worktrees do Claude Code**: `.claude/worktrees/<nome>/` com branch `worktree-<nome>`, igual ao `claude -w <nome>`.
4. **Seleção de trecho para o prompt, estilo plugin do Herdr** *(decidido)*: selecionar código no visualizador ou no diff e **escrever só a referência no terminal ativo**. Ex.: `@src/checkout/validators.ts (linhas 44–46)`.
5. **Linguagem visual do Zed** *(decidido)*: tema escuro inicial, interface densa e minimalista, One Dark como referência de cores.

### 📦 Esperado (útil, mas não é diferencial)

1. **Integração com a CLI do GitHub (`gh`)**: PRs, CI e revisões. *(Fase 2)*

### ⏸️ Fase 2 (pós-v1)

1. **Kanban** do ponto de vista do usuário, como **tela separada** do terminal. Colunas: 🚨 Precisa de mim (aguardando permissão, erro) · 👀 Revisar (aguardando você) · ⚙️ Trabalhando (trabalhando, com subagentes) · 💤 Parados (ocioso, encerrado).
2. **Interações ricas**: responder permissões pelo app (possível via hook `PermissionRequest`, a verificar), perguntas de múltipla escolha como botões, mockups em markdown renderizados.
3. Edição ao vivo, histórico de sessões, integração com `gh`.

---

## 🗺️ Etapas de desenvolvimento

| Etapa | Entrega | Telas |
|---|---|---|
| **0. Fundação** | Serviço no WSL com a vida do app, Unix socket, bridge via `wsl.exe`, protocolo em frames com handshake de versão, CLI `hive` (criar/remover worktree, receber eventos de hooks), `claude` embrulhado com integração fish. **Spike de hooks** com o modo gravador (`hive hook --record`) | — |
| **1. Terminal + um agente** | Listar projetos do WSL, criar worktree (com branch de origem, replicando o `.worktreeinclude`), terminal embutido com abas, detecção da sessão do Claude. **Critério de saída:** teste de carga com 20 terminais reproduzindo saída real do Claude, com o terminal em foco fluido | Esqueleto (barra lateral + terminais + barra de status) · modal nova worktree · estado vazio |
| **2. Estado dos agentes** | Barra lateral com estados (mapeamento revisado), propagação, contador de pendências, notificações, reconciliação por silêncio do PTY | Barra lateral completa |
| **3. Ver o que o agente fez** | Árvore de arquivos com diff em tempo real, seleção de trecho → terminal. **3b:** edição com verificação de versão | Painel de árvore + diff + editor |
| **4. Subagentes e worktrees** | Hooks `WorktreeCreate`/`WorktreeRemove` (reaproveitam o comando da Etapa 1), subagentes vinculados ao pai na barra lateral | Subagentes indentados |
| **Fase 2** | Kanban, interações ricas, edição ao vivo, histórico, `gh` | Kanban (tela separada) |

**v1 = Etapas 0 a 4.**

**Pendente para a Etapa 1:** decidir como o app Windows é compilado e executado durante o desenvolvimento (o código fica no WSL, o app roda no Windows), e ter um transporte simulado para a interface ser testada no navegador com Playwright.

---

## 🎨 Protótipo (Etapa 1)

Protótipo de alta fidelidade feito no Claude Design, guardado em `docs/prototype/stage-1/` (somente leitura para os agentes, com `README.md` em inglês). Os arquivos dependem do runtime do Claude Design (`support.js`, fora do repositório): os agentes os leem como código-fonte, não como página.

| Id | Cena | Tela |
|---|---|---|
| 1a | `principal` | Janela principal, agente com subagentes em foco |
| 1b | `permissao` | Agente aguardando permissão (respondida no próprio terminal) |
| 1g | `arquivos` | Painel direito: arquivos e diff (Etapa 3) |
| 1c | `modal` | Nova worktree |
| 1d | `modal-erro` | Nova worktree com erro de validação |
| 1e | `vazio` | Estado vazio |
| 1f | — | Folha de estilo: tokens, ícones de estado, tipografia, componentes base |

**O que o protótipo fixa:**

- **Paleta** estilo Zed/One Dark: fundo `#282C33`, painel `#2F343E`, barras `#3B414D`, borda `#464B57`, foco `#47679E`, ação `#74ADE8`, texto `#DCE0E5`/`#A9AFBC`/`#878A98`.
- **Estados com cor + forma** (acessível sem depender só da cor): permissão `#DEC184` triângulo · erro `#D07277` círculo com X · aguardando você `#E08A5A` anel com ponto · trabalhando `#74ADE8` ponto pulsando 2,4 s · com subagentes `#B477CF` três nós · ocioso `#A1C181` círculo com check · encerrado `#878A98` quadrado. Urgência alta mostra também o sino de pendência.
- **Tipografia**: IBM Plex Sans e IBM Plex Mono. O protótipo carrega do Google Fonts; o app precisa **embutir as fontes** (desktop offline, CSP do Tauri).
- **Atalhos** (ajustados pela #35): F8 próximo pendente · Ctrl+Shift+T seletor de worktree → novo terminal · Ctrl+Shift+N nova worktree · Ctrl+Shift+B painel de arquivos · Ctrl+Shift+O adicionar projeto · Ctrl+Shift+C/V copiar e colar no terminal · setas/Enter na árvore. O protótipo usa Alt e Ctrl+O, que colidem com o fish e com o Claude Code.
- **Nova worktree**: nome validado com `^[a-z0-9][a-z0-9._-]*$` e contra nomes existentes; branch de origem entre locais **e remotas**, com filtro; opção "abrir terminal na nova worktree".
- **Barra de status** com a distribuição WSL e o estado da conexão.
- **Idioma**: os textos do protótipo estão em português, mas a interface é em inglês (#34), seguindo o glossário do `docs/prototype/README.md`.

---

## 🧪 Regras de desenvolvimento

O desenvolvimento é **incremental e guiado**: o agente segue o `TODO.md` do repositório, uma tarefa por vez, e para em cada checkpoint para revisão humana.

| # | Regra | Motivo |
|---|---|---|
| D1 | **Tudo em inglês**: código, comentários, commits, docs do repositório, CLI, interface. Só este documento fica em português | Padrão do ecossistema; interface em inglês (#34) |
| D2 | **100% de cobertura de linhas + teste de mutação** (`cargo llvm-cov`, `cargo mutants` sem mutante sobrevivente no código alterado); no frontend, 100% de linhas no `bun test`. Exceções só por arquivo, listadas com justificativa em `COVERAGE_EXCLUSIONS.md` e aprovadas pelo humano | Cobertura total com prova de que os testes verificam comportamento |
| D3 | **Portões por tarefa**: `cargo fmt --check`, `clippy -D warnings` (sem `unwrap`/`expect` fora de testes), testes, `cargo deny`, `cargo machete`, `cargo check --locked`; frontend: lint, typecheck, `bun test --coverage`, `bun install --frozen-lockfile` | Qualidade constante, sem acumular dívida |
| D4 | **Git**: um branch por tarefa (`task/<id>-<slug>`) criado a partir da `main`, commits pequenos no padrão Conventional Commits (`type(scope): description`). Tarefas independentes rodam **em paralelo**, cada agente na sua worktree; **ao concluir** (portões verdes, `TODO.md` marcado, relatório feito) o agente faz `git merge main` no seu branch, resolve ele mesmo os conflitos preservando o trabalho dos outros e roda os portões de novo. A integração na `main` é sempre `--ff-only`: quem trabalha sozinho no checkout principal integra o próprio branch; branches feitos em worktree são integrados pelo orquestrador (`.claude/skills/stage/`), e se a `main` andou a tarefa volta ao agente para outro merge. O branch é apagado depois. Nunca push, force, rebase nem reescrita de histórico | Tudo que o agente fez fica na `main`, sem branches pendurados; paralelismo sem perder o histórico linear; a revisão acontece nos checkpoints |
| D5 | **O agente nunca muda uma decisão deste documento**; se algo estiver errado ou faltando, para e pergunta | O documento é a fonte da verdade |


---

## 🏗️ Arquitetura

```
Windows                                    WSL (Linux)
┌──────────────────────────┐               ┌────────────────────────────────────────┐
│ App desktop (Tauri)      │  stdio do     │ Serviço `hive daemon` (Rust)           │
│ React: barra lateral,    │  wsl.exe      │ • vive enquanto o app estiver aberto   │
│ árvore, editor, diff     │ ◄──────────►  │ • ouve num Unix socket                 │
│ xterm.js (fora do React) │  hive bridge  │ • abre os PTYs e repassa os bytes      │
└──────────────────────────┘  (frames      │ • gerencia worktrees (git CLI)         │
                               binários)   │ • observa e grava arquivos             │
                                           │ • mantém o estado dos agentes          │
                                           └───────────────▲────────────────────────┘
                                                           │ Unix socket
                                           ┌───────────────┴────────────────────────┐
                                           │ `hive hook` (mesmo binário)            │
                                           │ • chamado pelos hooks do Claude        │
                                           │ • cria/remove worktrees                │
                                           └───────────────▲────────────────────────┘
                                                           │ hooks
                                           ┌───────────────┴────────────────────────┐
                                           │ Terminal do Hive (PTY, fish -C)        │
                                           │ HIVE_TERMINAL_ID=...                   │
                                           │ `claude` embrulhado → claude real      │
                                           │   com --settings <hooks-do-hive>       │
                                           └────────────────────────────────────────┘
```

**Ciclo de vida:** o app abre → roda `wsl.exe hive bridge` → o bridge sobe o `hive daemon` (`setsid` + lockfile) se o socket não existir → handshake de versão (bloqueia com aviso se divergir) → o app abre terminais. Quando o app fecha ou cai, o bridge morre, e o serviço mata o **grupo de processos** de cada PTY e encerra.

**Fluxo de observação:** o `claude` roda no terminal do Hive → dispara hooks → os hooks chamam `hive hook` por caminho absoluto (herdando `HIVE_TERMINAL_ID`) → a CLI envia o evento ao serviço pelo socket → o serviço atualiza o estado e avisa o app.

**Camadas internas:**

```
Interface (barra lateral, terminais, árvore, editor, diff)
        ↓
Modelo interno (estados, eventos, arquivos alterados) — independente do agente
        ↓
Adaptador Claude Code (hooks + sessões em ~/.claude/projects/)  ← v1
Adaptadores futuros (Codex, Gemini...)
```

---

## 💡 Decisões técnicas

| # | Decisão | Motivo |
|---|---|---|
| 1 | **Claude Code interativo em terminal embutido**; observação via hooks + sessões em `~/.claude/projects/` | Igual ao Orca; sem dependência nem cobrança do SDK |
| 2 | **Arquitetura com adaptadores** | Outros agentes entram sem reescrever o app |
| 3 | **App em duas partes**: interface no Windows + serviço no WSL | Trabalho pesado perto dos arquivos; evita a ponte Windows↔WSL, lenta e ruim para detectar mudanças |
| 4 | **Projetos no sistema de arquivos do WSL** | Melhor cenário de performance |
| 5 | **Rust + Tauri** no app desktop | Usa o WebView2 do Windows em vez de embutir Chromium |
| 6 | **Serviço no WSL 100% Rust** | Sem SDK, o Node deixou de ser necessário |
| 7 | **O Hive é o dono das worktrees**, seguindo a convenção do Claude | Padroniza a criação e a remoção |
| 8 | **Descoberta via `git worktree list`** | Enxerga também worktrees criadas fora do Hive (aparecem como worktrees, sem agente vinculado) |
| 9 | **CLI e serviço no mesmo binário Rust `hive`**; a CLI é cliente do serviço | Mesma base; eventos aparecem no app em tempo real |
| 10 | **Subcomandos em inglês**: `worktree create/list/remove`, `hook <event>` (com modo `--record`), `bridge`, `daemon` | A CLI não cria nem controla agentes; idioma conforme D1 |
| 12 | **Sem merge automático** | Integração fica comigo |
| 13 | **Git via executável `git` do WSL** | A crate `gix` não gerencia worktrees |
| 14 | **Serviço no WSL ouvindo em Unix socket, com a mesma vida do app**: o `hive bridge` sobe o serviço (`setsid`, lockfile) se o socket não existir; o app conecta via `wsl.exe hive bridge` (stdio ↔ socket); a CLI usa o socket direto. Sem rede. | A CLI dos hooks precisa de um canal independente do stdio do app; sem portas nem firewall |
| 15 | **Hooks `WorktreeCreate` e `WorktreeRemove` apontando para a CLI**, cobrindo `claude -w` e subagentes com `isolation: worktree`, **apenas nos terminais do Hive** | Toda worktree passa pelo Hive, aparece sob quem a criou e some quando é removida |
| 17 | **Modelo interno independente do Claude** | Permite outros provedores |
| 18 | **Terminal embutido**: PTY no serviço do WSL, xterm.js na interface; **terminais e agentes encerram quando o app fecha, inclusive em crash** (o serviço mata o grupo de processos de cada PTY); confirmação ao fechar se houver agente 🔵, 🟣, 🟡 ou 🟠 (🟣 incluído no checkpoint 2: subagentes trabalhando também se perdem) | Simplicidade; sem reconexão nem snapshot de tela |
| 19 | **`HIVE_TERMINAL_ID`** (herdada pelos hooks) liga agente ↔ aba; a posição na hierarquia vem do `cwd` do payload | Um `cd` no terminal não coloca o agente na worktree errada |
| 20 | **O Hive mostra apenas sessões abertas nos seus próprios terminais** | Escopo claro; nada muda no uso externo |
| 21 | **Hooks injetados só nos terminais do Hive**: um `claude` embrulhado no `PATH` desses terminais chama o real com `--settings <hooks-do-hive>` | Configuração global intocada; o `--settings` **mescla** hooks, então os hooks pessoais continuam valendo |
| 22 | **Barra lateral como visão principal de estado**: Projeto → Worktree → Agente → Subagentes (indentados, com worktree própria quando houver), estado propagado | Acompanhar tudo sem sair do terminal |
| 23 | **Tema escuro inicial, linguagem visual do Zed** (One Dark como referência) | Preferência estética; interface densa para uso intenso |
| 24 | **Protocolo em frames binários** `[tipo][canal][tamanho][payload]`: controle em JSON, terminal em bytes crus; handshake de versão; frames de controle com prioridade sobre os de terminal; formato isolado no crate `hive-protocol`. Entre o Rust do Tauri e a WebView, um `Channel` do Tauri por terminal (eventos do Tauri não servem para alto volume) | Performance sem perder a legibilidade do controle; reversível |
| 25 | **`claude` embrulhado = script `sh` em `~/.local/share/hive/bin`**, colocado no início do `PATH` via `fish -C 'set -gx PATH …'`, depois da config do usuário; nunca `fish_add_path` sem flag (vazaria para o fish fora do Hive via variável universal); proteção contra recursão (`HIVE_WRAPPED=1`); o serviço avisa quando detecta `claude` no PTY sem eventos de hook | Funciona com fish sem tocar no `config.fish`; falha visível em vez de silenciosa |
| 26 | **Binário único `hive`** em `~/.local/share/hive/bin`, com atalho em `~/.local/bin` para uso fora do Hive; hooks e app usam o caminho absoluto | Mesmo código; imune a colisão de nome |
| 27 | **Hooks de observação síncronos**, com `timeout` curto no hook e timeout interno de ~200 ms na CLI, que sempre sai com 0 (exceto `WorktreeCreate`) | Mantém a ordem dos eventos por sessão; o `async` evitaria bloqueio, mas pode inverter eventos e fazer o estado piscar |
| 28 | **Terminais ilimitados**: WebGL só nos terminais visíveis; histórico limitado e configurável por terminal; teste de carga como critério de saída da Etapa 1; se falhar, emulador headless no serviço com snapshot ao exibir a aba | Todos precisam funcionar sem degradar o terminal em foco |
| 29 | **Binário Linux compilado por mim no WSL** (`cargo install`); o handshake compara versões e **bloqueia com aviso** se app e serviço divergirem | Zero infraestrutura na v1; sem erro silencioso |
| 30 | **React + React Compiler**; estado dos agentes em store externo com assinatura por agente (`useSyncExternalStore` ou seletores do Zustand); xterm.js gerenciado fora do React; listas grandes virtualizadas (TanStack Virtual) | Experiência prévia; as regras neutralizam o custo de re-renderização |
| 31 | **CodeMirror 6** para visualizar, editar e ver diff (`@codemirror/merge`); salvamento pelo serviço (temporário + rename) com verificação de versão; recarga automática com buffer limpo; aviso de conflito com buffer sujo; selo "agente trabalhando aqui"; botão "abrir no editor externo" | Leve, renderiza só o visível, fácil de deixar com cara de Zed; conflito com o agente nunca é silencioso |
| 32 | **Protótipo do Claude Design como referência** de aparência, layout e interação (`docs/prototype/`), protegido como o `hive.md` | Os agentes implementam o que foi desenhado, sem reinventar a interface |
| 33 | **Validação do nome de worktree idêntica na CLI e na interface** (`^[a-z0-9][a-z0-9._-]*$` + nome existente); `worktree create` aceita branch de origem local ou remota | Interface e CLI nunca discordam; a regra já entra na Etapa 0 |
| 34 | **Interface em inglês** (D1); o protótipo, em português, vale como referência visual e de comportamento; os textos seguem o glossário em `docs/prototype/README.md` | Consistência com a N9; um só vocabulário em todas as telas |
| 35 | **Atalhos do app com Ctrl+Shift+letra** (T novo terminal, N nova worktree, B painel de arquivos, O adicionar projeto; C/V copiar e colar no terminal), capturados mesmo com o foco no terminal; **F8** continua; o resto vai intocado para o terminal | Alt+B/Alt+T colidem com o fish e Ctrl+O com o Claude Code; Ctrl+Shift é a convenção do Windows Terminal |
| 36 | **Pacotes só pela CLI**: Rust com `cargo add -p` / `cargo remove` (sem `[workspace.dependencies]`, que o `cargo add` não suporta); frontend **só com bun** (`bun add`, `bun add -d`, `bun remove`, `bunx`, `bun create`; nada de npm, pnpm, yarn ou npx). Nunca editar à mão as seções de dependências nem os lockfiles; portões `cargo check --locked` e `bun install --frozen-lockfile` | O gerenciador escolhe versões reais e mantém manifesto e lockfile coerentes; evita versões inventadas pelo agente |
| 37 | **Toda a lógica em Rust; o frontend só apresenta.** Protocolo, PTYs, worktrees, git, hooks, estados dos agentes, observação e gravação de arquivos ficam no serviço. React/TypeScript renderiza o que o serviço manda, guarda estado de interface (seleção, painéis, diálogos, abas) e envia ações. **Bun** é gerenciador de pacotes, executor de scripts e test runner (`bun test` + `happy-dom`) do frontend; o app distribuído não tem runtime Bun nem Node | Uma única fonte de regras (sem duplicar lógica em duas linguagens); o Tauri empacota a interface como arquivos estáticos na WebView2 |
| 38 | **Sem biblioteca de rotas** (nem TanStack Router nem React Router). O Hive é uma janela só (barra lateral, terminais, painel, diálogos); o que está visível é estado de interface no store (ex.: `view`, `modal`, `rightPanel`). Rever só se, na Fase 2, surgirem várias telas com parâmetros | Num app desktop não há barra de endereço, links diretos nem botão voltar, que é o que um router resolve; seria dependência e complexidade sem uso |

### Detalhes importantes dos hooks de worktree (verificado)

- `WorktreeCreate` dispara para `--worktree`, subagentes com `isolation: worktree` e sessões em segundo plano.
- **Substitui completamente** a criação padrão. O `.worktreeinclude` deixa de ser processado, então **o Hive precisa copiar os arquivos ignorados pelo git** (`.env` etc.). Isso já acontece no comando `criar worktree` da Etapa 1.
- Qualquer saída diferente de 0 aborta a criação. A CLI devolve o **caminho da worktree no stdout**; qualquer saída extra quebra a criação.
- `WorktreeRemove` dispara no fim da sessão, no fim de um subagente e ao apagar uma sessão em segundo plano. Saída diferente de 0 faz a remoção falhar se o diretório ainda existir.
- Existe um bug aberto (ago/2026) de subagente com hook `WorktreeCreate` dando erro de isolamento. Testar no spike (tarefa 1.12).
- Se um projeto tiver um hook `WorktreeCreate` próprio, ele e o do Hive vão disputar a criação. O Hive deve detectar e avisar.

### Mapeamento de estados (hooks do Claude Code → estado visual)

| Estado | Origem | Urgência |
|---|---|---|
| 🟢 Ocioso | SessionStart | nenhuma |
| 🔵 Trabalhando | UserPromptSubmit, PreToolUse, PostToolUse, PostToolUseFailure | baixa |
| 🟡 Aguardando permissão/resposta | PermissionRequest; Notification `permission_prompt`, `elicitation_dialog` | alta 🚨 |
| 🟠 Aguardando você | Stop; Notification `idle_prompt`, `agent_needs_input`; silêncio do PTY (regra 2) | média |
| 🔴 Erro | StopFailure | alta 🚨 |
| 🟣 Com subagentes | SubagentStart / SubagentStop | baixa |
| ⚫ Encerrado | SessionEnd | nenhuma |

**Regras:**

1. **O mais urgente vence:** um subagente em 🟡 põe o pai em 🟡, não em 🟣.
2. **Reconciliação da interrupção:** o `Stop` não dispara quando o usuário interrompe (Esc/Ctrl+C). Se o agente está em 🔵 ou 🟡 e o PTY fica alguns segundos sem saída, ele vai para 🟠. Funciona porque o spinner do Claude escreve no terminal continuamente enquanto trabalha. *A verificar no spike:* se o spinner continua animando durante ferramentas longas e durante a espera de permissão. Silêncio implementado: 5 s (Etapa 2).

> Hooks de subagente trazem `agent_id` e `agent_type` no payload. O `cwd` segue o Claude para dentro da worktree.

---

## 🗑️ Decisões revogadas (não repropor sem motivo novo)

| Antiga | O que era | Por que saiu |
|---|---|---|
| SDK (antiga #1) | Rodar agentes via Claude Agent SDK | Hive virou observador com terminal embutido; elimina o risco de cobrança do SDK |
| Node (antiga #6) | Mini serviço Node para o SDK | Sem SDK |
| #11 | Resumo gerado pelo serviço para um agente gateway | A CLI não orquestra agentes; o "gateway" era só um padrão de uso meu, feito com subagentes do Claude |
| #16 | Ativar `settingSources` no SDK | Sem SDK |
| — | CLI criando/iniciando agentes, comandos `status`, `esperar`, `resumo` | A CLI só cria worktrees |
| — | Hierarquia de agentes independentes criados via CLI | Substituída pelos subagentes indentados |
| — | Kanban como visão principal | Movido para a Fase 2; a barra lateral é a visão principal |
| — | Hooks globais em `~/.claude/settings.json` | Afetariam o `claude` fora do Hive e quebrariam o `claude -w` externo |
| — | Stdio do `wsl.exe` como único canal | A CLI dos hooks não teria como conectar |
| — | Terminais persistentes com o app fechado (estilo tmux) | Custo alto (reconexão, emulador headless, WSL desligando sozinho sem terminal aberto) para pouco uso real |
| — | Qualquer falha de ferramenta (`PostToolUseFailure`) como erro 🚨 | Falha de ferramenta é rotina e o agente se recupera sozinho; inflaria o contador com falso alarme |
| — | Cobertura só no relatório | Substituída por 100% de linhas + mutação (D2) |
| — | Execução noturna autônoma com orquestração de agentes (N1–N17: distro dedicada, watchdog, 12 subagentes, portões automáticos) | Complexa demais para configurar e acompanhar; substituída pelo desenvolvimento incremental guiado pelo `TODO.md` |
| — | pnpm como gerenciador do frontend (#36 original) | Trocado por bun por preferência; como o JS é só a interface, a diferença prática é pequena |
| D4 original | O agente nunca fazia merge na `main`; o humano integrava cada branch | Muitos branches encadeados para integrar à mão; o agente passou a integrar ao concluir cada tarefa |
| — | Som a cada mudança de estado | Toca quando eu mesmo envio o prompt; com vários agentes vira ruído |

---

## ⚠️ Riscos

1. **Fidelidade e desempenho do terminal embutido**: a interface do Claude Code redesenha muito; com muitos agentes ativos, a interpretação dos bytes na thread da WebView pode degradar o terminal em foco. Mitigação: WebGL só nos visíveis, teste de carga na Etapa 1 e plano B com emulador headless (#28).
2. **O embrulho do `claude` depende da interface da CLI do Claude** (flag `--settings`) **e da ordem do `PATH`**. Mitigação: embrulho mínimo testado a cada atualização do Claude Code; `fish -C`; aviso quando um `claude` roda no PTY sem eventos de hook.
3. **Hooks de worktree**: `WorktreeCreate` substitui a criação padrão (o Hive replica o `.worktreeinclude`) e tem bug aberto com subagentes; `WorktreeRemove` precisa manter a barra lateral em sincronia.
4. **Vincular a worktree ao subagente certo**: hooks de subagente trazem `agent_id`/`agent_type`; falta confirmar se o `WorktreeCreate` já traz esses campos.
5. **Interface lenta mesmo com Tauri**: diffs grandes, árvores grandes e muitas atualizações em tempo real exigem virtualização e store com assinatura por agente (#30).
6. **Estado importante escondido na barra lateral**: sem o kanban, um agente pendente pode ficar num projeto recolhido. Mitigação: estado propagado + contador de pendências.
7. **Conflito de edição com o agente**: eu e o agente alterando o mesmo arquivo. Mitigação: verificação de versão ao salvar, aviso de buffer sujo, selo de agente ativo.
8. **Estado preso após interrupção**: o `Stop` não dispara em Esc/Ctrl+C. Mitigação: reconciliação por silêncio do PTY, que depende do comportamento do spinner (a verificar).
9. **Agentes morrem com o app**: fechar sem querer, crash ou atualização do Hive interrompe tarefas em andamento; processos que o agente deixou em segundo plano com `nohup`/`setsid` podem sobreviver ao encerramento. Mitigação: confirmação ao fechar; `claude --resume` recupera a conversa.

---

## 🔍 Referências para estudar

| Referência | Por quê |
|---|---|
| [Orca](https://github.com/stablyai/orca) | Inspiração principal (MIT): terminais embutidos, lista de agentes e subagentes |
| [opcode](https://github.com/winfunc/opcode) | Mesma stack: Tauri 2 + Claude Code |
| Hive (FedorenkoCodes) | Mesma ideia; ver como resolveram |
| [Zed](https://zed.dev) · [tema One](https://github.com/zed-industries/zed/blob/main/assets/themes/one/one.json) | Linguagem visual; edição ao vivo |
| Plugin do Herdr | Seleção de trecho para o prompt |
| [Docs de worktrees do Claude Code](https://code.claude.com/docs/en/worktrees) | `--worktree`, subagentes, `.worktreeinclude`, hooks |
| [Docs de configurações do Claude Code](https://code.claude.com/docs/en/settings) | Precedência e mesclagem do `--settings` |
| [Docs de hooks do Claude Code](https://code.claude.com/docs/en/hooks) | Nomes, payloads, `WorktreeCreate`/`WorktreeRemove`, `Notification` por tipo |
| [Invocação do fish](https://fishshell.com/docs/current/cmds/fish.html) · [`fish_add_path`](https://fishshell.com/docs/current/cmds/fish_add_path.html) | `-C` depois da config; armadilha da variável universal |
| [Issue do WSL #13416](https://github.com/microsoft/wsl/issues/13416) | WSL desliga sem terminal aberto, mesmo com systemd (motivo da revogação da persistência) |
| Protótipo (`docs/prototype/`) | Aparência, layout, estados e atalhos da interface |
| xterm.js | Terminal embutido |
| CodeMirror 6 + `@codemirror/merge` | Editor e diff |
| React Compiler · TanStack Virtual · canais do Tauri 2 | Interface com atualizações frequentes |

---

## ❓ Pontos em aberto

| # | Ponto | Situação |
|---|---|---|
| 7 | **Identificar o subagente dono de uma worktree** a partir do payload dos hooks | Resolver no spike com `hive hook --record` (movido da Etapa 0 para a tarefa 1.12 do `TODO.md`; gravações ainda pendentes) |
| 9 | **Revisão do protótipo de alta fidelidade** da Etapa 1 | Protótipo recebido e incorporado (#32, #33); pendência no ponto 13 (o 11 virou a #34 e o 12 a #35) |
| 13 | **Referência visual para os agentes**: sem o `support.js`, os agentes não renderizam o protótipo; capturas PNG das telas 1a–1g (exportadas do Claude Design) permitiriam comparar a implementação com o desenho | Exportar as capturas antes da noite da Etapa 1 |

**A verificar no spike de hooks (tarefa 1.12, movida da Etapa 0):**

- Se o `WorktreeCreate` disparado por um subagente traz `agent_id`/`agent_type`.
- Se o spinner do Claude continua escrevendo no terminal durante ferramentas longas (regra 2 do mapeamento).
- Se o Claude continua escrevendo no terminal enquanto espera uma permissão (🟡); se não, a regra 2 leva uma permissão sem resposta para 🟠 após 5 s.
- O bug do `WorktreeCreate` com subagentes.
- Se o Claude Code recusa editar um arquivo alterado desde a última leitura e o relê (lado do agente no conflito de edição).

### Resolvidos na sessão 2

| Ponto | Decisão |
|---|---|
| 1. Quem inicia o serviço | #14 (o bridge sobe; serviço com a vida do app) e #18 |
| 2. Protocolo | #24 |
| 3. Framework da interface | #30 |
| 4. Nome do comando da CLI | #26 |
| 5. Visualizador de arquivos e diff | #31 |
| 6. Como o `claude` embrulhado funciona | #25 |
| 8. Limites de histórico e terminais | #28 |
| 10. Versão do binário Linux no WSL | #29 |

---

## 📝 Adições manuais (situação)

| Item | Situação |
|---|---|
| Gestão de consumo de IA | Pós-v1; a analisar (possível leitura de uso nas sessões) |
| Alocação de RAM/processamento para agentes | Pós-v1; custo alto (cgroups no WSL), ganho duvidoso |
| Orquestração de agentes | **Descartado**: conflita com o Hive como observador; a orquestração é feita pelos subagentes do Claude |
| Notificação (som + SO) | **Incorporado** à Etapa 2 (som só em 🟡, 🟠 e 🔴) |
| Edição de arquivos | **Incorporado** à Etapa 3b |
