# Hive — Documento de Decisões do Projeto

> Versão 3.5. Chat com o Claude no app, estados revisados, configurações, espaços, CI e as decisões das Etapas 5 a 8 (2026-09-27). Substitui a versão 3.4.

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

**Contexto sobre mim:** sou desenvolvedor, uso Windows com WSL (e também um Mac com Apple Silicon), meus projetos ficam no sistema de arquivos do WSL, uso **fish** como shell no WSL, tenho experiência com **React**, uso muito worktrees e costumo trabalhar com um agente que recebe várias tarefas e distribui cada uma para um subagente. Rodo **muitos agentes em paralelo, sem limite definido**: todos precisam funcionar. Este é um projeto pessoal, feito para minha satisfação como desenvolvedor, não um produto comercial.

**Situação atual:** as Etapas 0 a 8 estão implementadas e publicadas no GitHub Releases (v0.1.0 a v0.4.0; Windows + WSL e macOS). O desenvolvimento continua **incremental e guiado** (seção "Regras de desenvolvimento"): cada etapa nova nasce de pedidos meus no `TODO.md`, e os agentes seguem uma tarefa por vez, com checkpoints de revisão humana. Pendentes: o relatório do spike de hooks (1.12), as gravações que confirmam os estados (7.6) e o chat (7.3), o `claude` nativo no macOS (5.12) e as capturas do protótipo (ponto 13).

**Primeiro passo sugerido:** leia o documento inteiro, aponte qualquer inconsistência ou risco e depois me ajude a planejar a próxima etapa.

---

## 🏷️ Nome

**Hive** *(decidido)*. Existe outro projeto com a mesma ideia e o mesmo nome (FedorenkoCodes).

**Comando da CLI: `hive`** *(decidido)*. Um binário só, com subcomandos em inglês. Colide em tese com o Apache Hive e com o projeto do FedorenkoCodes, mas hooks e app sempre chamam o binário por **caminho absoluto**, então uma colisão futura afeta só o uso manual.

---

## 🧭 Conceito central

O Hive é **igual ao Orca neste ponto**: tem **terminais embutidos**, e eu rodo o `claude` interativo dentro deles. O Hive **observa** os agentes abertos nos seus terminais e mostra o estado de cada um; **nos terminais ele não controla e não conversa** com os agentes: toda interação acontece no próprio terminal. Além dos terminais, o Hive tem **abas de chat** (7.3, #40): nelas eu converso com o `claude` pelo próprio app, e o app é a interface do agente, inclusive das permissões, perguntas e aprovação de plano. O chat roda o binário oficial em modo headless (`claude -p` com `stream-json`), sem o pacote do Agent SDK e sem Node. Por conveniência, o Hive **pode iniciar** um `claude` digitando o comando num terminal novo (nova worktree com "Start claude", "+" de novo chat, retomar ou bifurcar uma sessão do histórico, reabrir as sessões que estavam abertas quando o app fechou) (Etapa 4, 4.7/4.8/4.11/4.12).

Consequências:

- **Sem o pacote do Claude Agent SDK e sem Node.** Terminais usam o `claude` interativo; o chat usa o `claude -p` com o protocolo `stream-json`, com o meu login. Hoje os dois consomem os limites da assinatura; o `-p` segue a regra de cobrança do SDK (#45).
- O Hive é um **companheiro do terminal**, não um substituto.
- O **estado ao vivo** (barra lateral, pendências, notificações) vem só das sessões abertas **nos terminais e chats do Hive**. O **histórico de sessões** (painel direito) lista também as sessões rodadas fora do Hive nos projetos seguidos, com um estado lido do log (4.11/4.12).
- **Terminais, chats e agentes vivem enquanto o app estiver aberto.** Fechar o app (ou um crash) encerra todos; ao reabrir, as sessões do `claude` que estavam abertas são retomadas (`claude --resume`, 4.12), e um chat volta como chat (#44).
- O Hive **pode editar arquivos** (Etapa 3b). Nos terminais nunca age sobre o agente; no chat, só repassa o que eu digito e clico.

---

## 🚫 Requisitos obrigatórios

1. **App desktop.**
2. **Conexão 100% funcional com WSL** no Windows: agentes, git e projetos rodando dentro do WSL. **No macOS**, tudo roda nativo (#39).
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
3. **Subagentes na barra lateral** *(decidido)*: indentados sob o agente pai, com estado próprio; quando o subagente tem worktree própria, **a worktree vira a linha pai e o subagente fica indentado sob ela** (Projeto → Worktree → Agente em todos os níveis; decisão de 2026-09-24, tarefa 5.4, que substitui "a worktree indentada sob o subagente"). Base técnica: hooks `SubagentStart`/`SubagentStop` e hooks `WorktreeCreate`/`WorktreeRemove`.
4. **Estado propagado + contador de pendências** *(decidido)*: projeto ou worktree recolhido mostra o **ícone do estado mais urgente** dentro dele, sem o sino do protótipo. **Pendente = qualquer 🟡, 🔴 ou 🟠** (checkpoint 2); o serviço decide e manda `pending`/`urgency`. Na barra de título, um **sino** (4.14) com **um só selo laranja**: o número de agentes pendentes que eu ainda não vi; **abrir o sino zera o selo** (8.5), e o agente volta a contar quando fica pendente de novo ou passa a outro estado pendente. O clique abre a **caixa de entrada** (6.5): primeiro os agentes pendentes, depois o histórico de alertas (até 100, só em memória); clicar num item vai ao agente. **F8** vai ao próximo pendente. Compensa a ausência do kanban.
5. **Notificação** *(decidido)*: som ao entrar em 🟡, 🟠 ou 🔴; notificação do sistema operacional quando o agente termina (🔵/🟣 → 🟠). **Volume 0–100 % nas configurações, 0 = mudo** (6.1). Exceção: com a janela do Hive em foco e o terminal daquele agente à vista, não há notificação do sistema nem o agente entra no contador de pendências (já foi visto; 5.6). Uma interrupção minha (Esc/Ctrl+C, recusa de um diálogo) vai para 🟠 sem som, pendência nem notificação (7.6). Agentes de todos os espaços continuam alertando (#48). O app informa ao serviço qual agente está à vista e se a janela tem foco; o serviço continua decidindo `pending` (#37).
6. **Edição de arquivos** *(decidido)*: editar no visualizador, salvando pelo serviço com verificação de versão; sem LSP na v1; diff somente leitura *(Etapa 3b)*. Criar, renomear e mover arquivos, criar pastas e vários arquivos abertos em abas (#56).

### ✨ Inspirado em outros apps

1. **Edição ao vivo, estilo Zed**: ver qual arquivo o agente está alterando, no momento em que acontece. *(Fase 2)*
2. **Histórico de sessões, estilo Orca/opcode** *(feito na Etapa 4, 4.11)*: lê `~/.claude/projects/` (ou o `CLAUDE_CONFIG_DIR` do espaço, #48) dos projetos seguidos; retomar, bifurcar, abrir como chat, copiar comando/ID, abrir log, apagar. Uma sessão só conta como rodando quando um `claude` roda exatamente aquele id (8.16, #20).
3. **Convenção de worktrees do Claude Code**: `.claude/worktrees/<nome>/` com branch `worktree-<nome>`, igual ao `claude -w <nome>`.
4. **Seleção de trecho para o prompt, estilo plugin do Herdr** *(decidido)*: selecionar código no visualizador ou no diff e **escrever só a referência no terminal ativo**. Ex.: `@src/checkout/validators.ts (linhas 44–46)`.
5. **Linguagem visual do Zed** *(decidido)*: tema escuro inicial, interface densa e minimalista, One Dark como referência de cores.

### 📦 Esperado (útil, mas não é diferencial)

1. **Integração com a CLI do GitHub (`gh`)**: PRs, CI e revisões. *(Fase 2)*

### ⏸️ Fase 2 (pós-v1)

1. **Kanban** do ponto de vista do usuário, como **tela separada** do terminal. Colunas: 🚨 Precisa de mim (aguardando permissão, erro) · 👀 Revisar (aguardando você) · ⚙️ Trabalhando (trabalhando, com subagentes) · 💤 Parados (ocioso, encerrado).
2. **Interações ricas nos terminais**: responder permissões do `claude` interativo pelo app (hook `PermissionRequest`), perguntas de múltipla escolha como botões, mockups em markdown. *No chat (7.3) isso já existe.*
3. Edição ao vivo, integração com `gh`.

---

## 🗺️ Etapas de desenvolvimento

| Etapa | Entrega | Telas |
|---|---|---|
| **0. Fundação** | Serviço no WSL com a vida do app, Unix socket, bridge via `wsl.exe`, protocolo em frames com handshake de versão, CLI `hive` (criar/remover worktree, receber eventos de hooks), `claude` embrulhado com integração fish. **Spike de hooks** com o modo gravador (`hive hook --record`) | — |
| **1. Terminal + um agente** | Listar projetos do WSL, criar worktree (com branch de origem, replicando o `.worktreeinclude`), terminal embutido com abas, detecção da sessão do Claude. **Critério de saída:** teste de carga com 20 terminais reproduzindo saída real do Claude, com o terminal em foco fluido | Esqueleto (barra lateral + terminais + barra de status) · modal nova worktree · estado vazio |
| **2. Estado dos agentes** | Barra lateral com estados (mapeamento revisado), propagação, contador de pendências, notificações, reconciliação por silêncio do PTY | Barra lateral completa |
| **3. Ver o que o agente fez** | Árvore de arquivos com diff em tempo real, seleção de trecho → terminal. **3b:** edição com verificação de versão | Painel de árvore + diff + editor |
| **4. Subagentes e worktrees** | Hooks `WorktreeCreate`/`WorktreeRemove` (reaproveitam o comando da Etapa 1), subagentes vinculados ao pai na barra lateral | Subagentes indentados |
| **5. Pastas e macOS** (v0.1.x) | Navegador de pastas ao adicionar projeto, macOS (#39), portões pesados no CI (D6) | — |
| **6. Configurações e observação** (v0.2.0) | Configurações, paleta, atividade, caixa de entrada, saúde das worktrees, comentários de revisão, scripts e portas, uso, conversa do subagente, terminais divididos, `hive badge`, espaços | Configurações · paleta · espaços |
| **7. Chat e estados** (v0.3.x) | Chat no app (#40–#46), estados revisados, criar/renomear arquivos, menu "+", ícones Phosphor, fonte Hive Mono, mutação em shards | Chat |
| **8. Refinamentos** (v0.4.0) | Janela de contexto real, reordenar agentes e abas, mover arquivos e criar pastas, modo Auto, seletor de modelo, Markdown, títulos e menções no chat, vários arquivos abertos, sessão rodando pelo id, sem diálogos do navegador | — |
| **Fase 2** | Kanban, interações ricas nos terminais, edição ao vivo, `gh` | Kanban (tela separada) |

**v1 = Etapas 0 a 4.**

**Resolvido na Etapa 1 (1.0):** o app Windows é compilado no WSL com `cargo xwin` e roda por `scripts/win-dev.sh`; a interface é testada no navegador com Playwright contra um transporte simulado.

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
- **Estados com cor + forma** (acessível sem depender só da cor): permissão `#DEC184` triângulo · erro `#D07277` círculo com X · aguardando você `#E08A5A` anel com ponto · trabalhando `#74ADE8` ponto pulsando 2,4 s · com subagentes `#B477CF` três nós · ocioso `#A1C181` círculo com check · encerrado `#878A98` quadrado. Urgência alta mostra também o sino de pendência. No app, as cores ficam e as formas são ícones do Phosphor (#54).
- **Tipografia**: IBM Plex Sans na interface; terminal e editor com **Hive Mono** (#55), no lugar do IBM Plex Mono do protótipo. O protótipo carrega do Google Fonts; o app **embute as fontes** (desktop offline, CSP do Tauri).
- **Atalhos**: seguem a #35 (lista completa lá); setas/Enter na árvore. O protótipo usa Alt e Ctrl+O, que colidem com o fish e com o Claude Code.
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
| D4 | **Git**: commits pequenos direto na `main`, no padrão Conventional Commits (`type(scope): description`); um branch `task/<id>-<slug>` a partir da `main` só para tarefa complexa ou em paralelo, cada agente na sua worktree. **Ao concluir** (portões e CI verdes, `TODO.md` marcado, relatório feito) o agente faz `git merge main` no seu branch, resolve ele mesmo os conflitos preservando o trabalho dos outros e roda os portões de novo. A integração na `main` é `--ff-only`: quem trabalha sozinho no checkout principal integra o próprio branch; branches feitos em worktree são integrados pelo orquestrador (`.claude/skills/stage/`), e se a `main` andou a tarefa volta ao agente para outro merge. O branch é apagado depois. **Push só de branches `task/*`, pelo `scripts/ci.sh push`** (dispara o CI, D6); `main` e tags só eu. Nunca force, rebase nem reescrita de histórico | Tudo que o agente fez fica na `main`, sem branches pendurados; paralelismo sem perder o histórico linear; o CI testa antes do merge; a revisão acontece nos checkpoints |
| D5 | **O agente nunca muda uma decisão deste documento**; se algo estiver errado ou faltando, para e pergunta | O documento é a fonte da verdade |
| D6 | **Portões pesados no GitHub Actions** (5.11, decisão de 2026-09-25): `llvm-cov`, mutação e e2e rodam no CI a cada push do branch da tarefa (`ci.yml`, `macos.yml`); localmente só os rápidos (fmt, clippy, testes do crate tocado, lint/typecheck/test do frontend). A tarefa só entra na `main` com o CI verde na ponta do branch. **Mutação em shards** (7.1): todo mutante do diff, em 1 a 20 shards de ~25 mutantes; toda noite, a `main` inteira, com uma issue para os sobreviventes | Portões pesados em paralelo na minha máquina reiniciaram a VM do WSL; o repositório é público e os runners são gratuitos; tempo quase constante qualquer que seja o tamanho da mudança, sem enfraquecer o portão |


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

**Fluxo do chat (#40):** o app manda `chat_send` → o serviço escreve no stdin do `claude -p` (pipes, `stream-json`) → lê o stdout, traduz em mensagens tipadas (`chat_entries`, `chat_request`, `chat_status`) e avisa o app; os hooks do chat seguem o fluxo de observação.

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
| 1 | **Claude Code interativo em terminal embutido**; observação via hooks + sessões em `~/.claude/projects/`. O chat (#40) é a exceção: `claude -p` headless | Igual ao Orca; sem dependência do SDK (cobrança do chat na #45) |
| 2 | **Arquitetura com adaptadores** | Outros agentes entram sem reescrever o app |
| 3 | **App em duas partes**: interface no Windows + serviço no WSL | Trabalho pesado perto dos arquivos; evita a ponte Windows↔WSL, lenta e ruim para detectar mudanças |
| 4 | **Projetos no sistema de arquivos do WSL** | Melhor cenário de performance |
| 5 | **Rust + Tauri** no app desktop | Usa o WebView2 do Windows em vez de embutir Chromium |
| 6 | **Serviço no WSL 100% Rust** | Sem SDK, o Node deixou de ser necessário |
| 7 | **O Hive é o dono das worktrees**, seguindo a convenção do Claude | Padroniza a criação e a remoção |
| 8 | **Descoberta via `git worktree list`** | Enxerga também worktrees criadas fora do Hive (aparecem como worktrees, sem agente vinculado) |
| 9 | **CLI e serviço no mesmo binário Rust `hive`**; a CLI é cliente do serviço | Mesma base; eventos aparecem no app em tempo real |
| 10 | **Subcomandos em inglês**: `worktree create/list/remove`, `hook <event>` (com modo `--record`), `badge <texto>` / `badge --clear` (6.12), `bridge`, `daemon`. O `badge` só põe um rótulo (até 40 caracteres, sem caracteres de controle) na aba e na linha do agente do terminal de `HIVE_TERMINAL_ID`; some quando o terminal sai | A CLI não cria nem controla agentes; idioma conforme D1 |
| 12 | **Sem merge automático** | Integração fica comigo |
| 13 | **Git via executável `git` do WSL** | A crate `gix` não gerencia worktrees |
| 14 | **Serviço no WSL ouvindo em Unix socket, com a mesma vida do app**: o `hive bridge` sobe o serviço (`setsid`, lockfile) se o socket não existir; o app conecta via `wsl.exe hive bridge` (stdio ↔ socket); a CLI usa o socket direto. Sem rede. | A CLI dos hooks precisa de um canal independente do stdio do app; sem portas nem firewall |
| 15 | **Hooks `WorktreeCreate` e `WorktreeRemove` apontando para a CLI**, cobrindo `claude -w` e subagentes com `isolation: worktree`, **apenas nos terminais do Hive** | Toda worktree passa pelo Hive, aparece sob quem a criou e some quando é removida |
| 17 | **Modelo interno independente do Claude** | Permite outros provedores |
| 18 | **Terminal embutido**: PTY no serviço do WSL, xterm.js na interface; **terminais e agentes encerram quando o app fecha, inclusive em crash** (o serviço mata o grupo de processos de cada PTY); confirmação ao fechar se houver agente 🔵, 🟣, 🟡 ou 🟠 (🟣 incluído no checkpoint 2: subagentes trabalhando também se perdem; desligável em `agents.confirm_close`, 6.1). Chats seguem a mesma regra | Simplicidade; sem reconexão nem snapshot de tela |
| 19 | **`HIVE_TERMINAL_ID`** (herdada pelos hooks) liga agente ↔ aba; a posição na hierarquia vem do `cwd` do payload | Um `cd` no terminal não coloca o agente na worktree errada |
| 20 | **O estado ao vivo vem só das sessões abertas nos terminais e chats do Hive**; o histórico de sessões (4.11) também lista as rodadas fora do Hive nos projetos seguidos, sem retomá-las nem apagá-las enquanto rodam. **Rodando = um `claude` conhecido roda aquele id de sessão** (8.16): os do Hive pelos hooks; os de fora pelo registro `<config>/sessions/<pid>.json` do Claude, por `--session-id`/`--resume` na linha de comando ou pelo `.jsonl` aberto; um processo cuja sessão não se sabe não marca nada. Sessão rodando fora do Hive: Resume, Open as Chat e Delete desligados, sem aviso | Escopo claro; nada muda no uso externo; o palpite pela pasta (4.12) bloqueava sessões já encerradas |
| 21 | **Hooks injetados só nos terminais do Hive**: um `claude` embrulhado no `PATH` desses terminais chama o real com `--settings <hooks-do-hive>` | Configuração global intocada; o `--settings` **mescla** hooks, então os hooks pessoais continuam valendo |
| 22 | **Barra lateral como visão principal de estado**: Projeto → Worktree → Agente → Subagentes, estado propagado; a worktree própria de um subagente é a linha pai dele (Projeto → Worktree → Agente em todos os níveis, 5.4) | Acompanhar tudo sem sair do terminal |
| 23 | **Tema escuro inicial, linguagem visual do Zed** (One Dark como referência); tema claro One Light opcional nas configurações (6.1) | Preferência estética; interface densa para uso intenso |
| 24 | **Protocolo em frames binários** `[tipo][canal][tamanho][payload]`: controle em JSON, terminal em bytes crus; handshake de versão; frames de controle com prioridade sobre os de terminal; formato isolado no crate `hive-protocol`. Entre o Rust do Tauri e a WebView, um `Channel` do Tauri por terminal (eventos do Tauri não servem para alto volume) | Performance sem perder a legibilidade do controle; reversível |
| 25 | **`claude` embrulhado = script `sh` em `~/.local/share/hive/bin`**, colocado no início do `PATH` via `fish -C 'set -gx PATH …'`, depois da config do usuário; nunca `fish_add_path` sem flag (vazaria para o fish fora do Hive via variável universal); proteção contra recursão (`HIVE_WRAPPED=1`); o serviço avisa quando detecta `claude` no PTY sem eventos de hook | Funciona com fish sem tocar no `config.fish`; falha visível em vez de silenciosa |
| 26 | **Binário único `hive`** em `~/.local/share/hive/bin`, com atalho em `~/.local/bin` para uso fora do Hive; hooks e app usam o caminho absoluto | Mesmo código; imune a colisão de nome |
| 27 | **Hooks de observação síncronos**, com `timeout` curto no hook e timeout interno de ~200 ms na CLI, que sempre sai com 0 (exceto `WorktreeCreate`) | Mantém a ordem dos eventos por sessão; o `async` evitaria bloqueio, mas pode inverter eventos e fazer o estado piscar |
| 28 | **Terminais ilimitados**: WebGL só nos terminais visíveis; histórico limitado e configurável por terminal; teste de carga como critério de saída da Etapa 1; se falhar, emulador headless no serviço com snapshot ao exibir a aba | Todos precisam funcionar sem degradar o terminal em foco |
| 29 | **Distribuição pelo GitHub Releases** (4.17–4.19): uma tag `v*` gera no GitHub Actions o instalador Windows (NSIS) e o do macOS, assinados para o updater (minisign), com o `hive` do serviço **dentro do instalador**; ao conectar, o bridge copia esse `hive` para `~/.local/share/hive/bin/hive` quando difere e o executa (`cargo install` só em desenvolvimento). Ao abrir, o app consulta o `latest.json` da última release e mostra **"Update to vX" na barra de título**; o clique pergunta antes se há agentes rodando, instala e reinicia. O handshake continua comparando versões e **bloqueia com aviso** se divergirem | App e serviço sempre na mesma versão sem passo manual; sem erro silencioso |
| 30 | **React + React Compiler**; estado dos agentes em store externo com assinatura por agente (`useSyncExternalStore` ou seletores do Zustand); xterm.js gerenciado fora do React; listas grandes virtualizadas (TanStack Virtual) | Experiência prévia; as regras neutralizam o custo de re-renderização |
| 31 | **CodeMirror 6** para visualizar, editar e ver diff (`@codemirror/merge`); salvamento pelo serviço (temporário + rename) com verificação de versão; recarga automática com buffer limpo; aviso de conflito com buffer sujo; selo "agente trabalhando aqui"; botão "abrir no editor externo" | Leve, renderiza só o visível, fácil de deixar com cara de Zed; conflito com o agente nunca é silencioso |
| 32 | **Protótipo do Claude Design como referência** de aparência, layout e interação (`docs/prototype/`), protegido como o `hive.md` | Os agentes implementam o que foi desenhado, sem reinventar a interface |
| 33 | **Validação do nome de worktree idêntica na CLI e na interface** (`^[a-z0-9][a-z0-9._-]*$` + nome existente); `worktree create` aceita branch de origem local ou remota | Interface e CLI nunca discordam; a regra já entra na Etapa 0 |
| 34 | **Interface em inglês** (D1); o protótipo, em português, vale como referência visual e de comportamento; os textos seguem o glossário em `docs/prototype/README.md` | Consistência com a N9; um só vocabulário em todas as telas |
| 35 | **Atalhos do app com Ctrl+Shift+letra**, capturados mesmo com o foco no terminal: T novo terminal, N nova worktree, B painel lateral, O adicionar projeto, L referência do trecho (3.4), P paleta de comandos (6.3), D dividir terminal (6.11), M comentário de revisão (6.7); C/V copiar e colar no terminal. Exceções fora do padrão: **Ctrl+,** configurações (6.2) e **F8** próximo pendente. O resto vai intocado para o terminal. **No macOS, Cmd no lugar de Ctrl** (Cmd+Shift+letra, Cmd+,; Cmd+C/Cmd+V no terminal), e o Ctrl vai para o shell (#39). **Teclas locais não são atalhos do app**: as do compositor do chat (#58) e Alt+↑/↓ numa linha de agente focada (#57). Atalhos do navegador desligados (#53) | Alt+B/Alt+T colidem com o fish e Ctrl+O com o Claude Code; Ctrl+Shift é a convenção do Windows Terminal |
| 36 | **Pacotes só pela CLI**: Rust com `cargo add -p` / `cargo remove` (sem `[workspace.dependencies]`, que o `cargo add` não suporta); frontend **só com bun** (`bun add`, `bun add -d`, `bun remove`, `bunx`, `bun create`; nada de npm, pnpm, yarn ou npx). Nunca editar à mão as seções de dependências nem os lockfiles; portões `cargo check --locked` e `bun install --frozen-lockfile` | O gerenciador escolhe versões reais e mantém manifesto e lockfile coerentes; evita versões inventadas pelo agente |
| 37 | **Toda a lógica em Rust; o frontend só apresenta.** Protocolo, PTYs, worktrees, git, hooks, estados dos agentes, observação e gravação de arquivos ficam no serviço. React/TypeScript renderiza o que o serviço manda, guarda estado de interface (seleção, painéis, diálogos, abas) e envia ações. **Bun** é gerenciador de pacotes, executor de scripts e test runner (`bun test` + `happy-dom`) do frontend; o app distribuído não tem runtime Bun nem Node | Uma única fonte de regras (sem duplicar lógica em duas linguagens); o Tauri empacota a interface como arquivos estáticos na WebView2 |
| 38 | **Sem biblioteca de rotas** (nem TanStack Router nem React Router). O Hive é uma janela só (barra lateral, terminais, painel, diálogos); o que está visível é estado de interface no store (ex.: `view`, `modal`, `rightPanel`). Rever só se, na Fase 2, surgirem várias telas com parâmetros | Num app desktop não há barra de endereço, links diretos nem botão voltar, que é o que um router resolve; seria dependência e complexidade sem uso |
| 39 | **macOS (Apple Silicon) como segunda plataforma** (Etapa 5): app e serviço `hive` rodam nativos, sem WSL; assinatura ad-hoc, sem conta Apple Developer (na primeira abertura é preciso liberar em Privacidade e Segurança); terminais abrem o shell de login do usuário (`$SHELL`) com o bin do Hive primeiro no `PATH`; processos via `libproc`, arquivos via `notify` (FSEvents); botões nativos da janela; textos sem WSL/Explorer. No Windows, o seletor `[Windows \| WSL]` ao adicionar projeto aceita pastas do Windows (`/mnt/c`, mais lentas e sem atualização ao vivo) | Uso o Hive também num Mac; mesmo código Rust, com as diferenças isoladas por plataforma |
| 40 | **Chat no app (7.3)**: aba de chat ao lado dos terminais (o "Agent" do "+", 7.5); o serviço roda o `claude` real em modo headless (`-p --input-format stream-json --output-format stream-json --verbose --replay-user-messages --permission-prompt-tool stdio --settings <hooks do Hive>`), por **pipes, sem PTY**, num grupo de processos próprio; o chat usa o mesmo espaço de canais dos terminais (`HIVE_TERMINAL_ID` = canal do chat) e morre com o app como os terminais (#18); acha o `claude` pelo `PATH` do shell de login do usuário (7.15); nunca `--bare` | Pedido meu de 2026-09-25; o `stream-json` é o que o SDK usa por baixo, sem precisar do pacote nem de Node; os hooks, a barra lateral, o contador e as notificações funcionam iguais |
| 41 | **O app é o host de permissões do chat**: pedidos de permissão, perguntas (`AskUserQuestion`) e aprovação de plano (`ExitPlanMode`) viram cartões na conversa; nada roda sem clique; fechar, cancelar ou erro = negar; o serviço só aceita resposta para um pedido pendente daquele chat, uma vez. Modos: Default (inicial), Accept edits, Plan e **Auto** (8.4; só para modelo com modo automático, 8.9; se o Claude recusa, mostra a mensagem e fica no modo atual); **`bypassPermissions` nunca**, sem "Always allow". Um cartão só toma o foco quando nada editável o tem (8.10) | Com o chat, o Hive passa a ser o prompt de permissão; um erro aqui executaria comandos sem meu consentimento |
| 42 | **Estados do chat vêm do stream**, além dos hooks: pedido pendente → 🟡, `result` de sucesso → 🟠, `result` de erro ou saída sem `result` → 🔴, fim do processo → ⚫; a regra do silêncio do PTY não vale para o chat | O stream diz exatamente quando o agente espera por mim; pergunta e fim de turno deixam de parecer iguais |
| 43 | **Saída do chat é entrada não confiável**: linha ≤ 16 MiB, texto por entrada cortado em 64 KiB, plano em 256 KiB; imagens enviadas por mim: até 10 por mensagem e 3 MiB de base64 juntas (cabem num frame), tipo conferido pelo conteúdo; prompts e saídas de ferramentas fora dos logs no nível padrão. Respostas do Claude em **Markdown sanitizado** (`react-markdown` + `remark-gfm`, 8.6): HTML cru vira texto, nenhuma imagem da rede, links só `http(s)`/`mailto`, abertos fora do app; minhas mensagens em texto puro | Mesmas regras de hooks e mensagens do socket (linha de base do código) |
| 44 | **Retomada do chat**: o `session_id` do `system/init` fica no chat; reabrir usa `--resume <id>`; o histórico vem do arquivo da sessão (leitor da 6.10); o `open-sessions.json` (4.12) guarda o tipo (terminal ou chat) e o chat volta como chat; "Open as Chat" no histórico de sessões | Mesmo comportamento dos terminais depois de reiniciar o app |
| 45 | **Cobrança do chat**: usa o meu login do `claude` (o Hive nunca lê credenciais); o cabeçalho avisa quando o `apiKeySource` indica chave de API em vez da assinatura; o `total_cost_usd` (estimativa) não é mostrado como valor cobrado | Hoje o `claude -p` consome os limites da assinatura (nota da Anthropic de 16/06/2026); a separação anunciada foi pausada e pode voltar |
| 46 | **Pastas não confiáveis**: chat só em projetos adicionados ao Hive; no primeiro chat de cada projeto, o Hive pede confirmação (lembrada no arquivo de configurações do Hive) | O `-p` não mostra o diálogo de confiança e já roda hooks e servidores MCP do projeto |
| 47 | **Configurações num arquivo do serviço** (6.1/6.2): `$XDG_CONFIG_HOME/hive/settings.json` (0600, gravação atômica, tamanho limitado; chave desconhecida ignorada, valor fora da faixa recusado com mensagem): fonte, tamanho, histórico e cursor do terminal, tema, volume 0–100 (0 = mudo), silêncio 2–60 s, confirmação ao fechar, branch de origem padrão, scripts por projeto (#49). O diálogo (Ctrl+,) aplica ao vivo e abre o arquivo no editor | Lógica e validação em Rust (#37); um arquivo que eu também posso editar à mão |
| 48 | **Espaços** (6.14): um espaço agrupa **projetos e uma identidade opcional** para os seus terminais e chats (`CLAUDE_CONFIG_DIR`, nome/e-mail do git, `GH_CONFIG_DIR`), passada como variáveis de ambiente validadas; um projeto pertence a um espaço só; o `projects.json` antigo vira o espaço "Default"; só se apaga um espaço vazio. Barra lateral e histórico mostram o espaço atual; **agentes de todos os espaços continuam vivos e alertando** (sino, F8, som e notificação nomeiam o espaço; ir ao agente troca de espaço) | Sessões de trabalho e pessoais estavam misturadas; cada conta do Claude, git e `gh` no seu espaço |
| 49 | **Scripts de projeto e portas** (6.8): `setup`, `run` (vários, com nome) e `archive` por projeto, **só nas configurações do Hive, nunca lidos do repositório**. Setup roda num terminal próprio depois de criar a worktree; Run pelo menu da worktree ou pela paleta; Archive roda antes de remover a worktree (`sh -c`, 60 s; falha cancela a remoção, exceto se forçada). Cada worktree ganha um bloco fixo de 10 portas em 20000–29999 (`ports.json`); os terminais recebem `HIVE_PORT`, `HIVE_WORKTREE_PATH` e `HIVE_ROOT_PATH` | Um repositório clonado não roda código sozinho; servidores de worktrees diferentes não disputam portas |
| 50 | **Conversa do subagente** (6.10): clicar num subagente mostra o transcript dele **somente leitura** no lugar dos terminais (sem thinking nem resultados de ferramentas); nada é digitado no Claude; só arquivos dentro da pasta de projetos do Claude | Ver o que o subagente faz sem abrir o log; nos terminais o Hive continua observador |
| 51 | **Tokens e contexto só dos transcripts** (6.9): "ctx N%" na linha do agente e tokens nos cartões de sessão; sem limites de 5 h/semana, a statusline do usuário fica intocada. **Janela = a janela real do modelo** (8.1): no chat, o `contextWindow` do `modelUsage`; nos terminais, a janela aprendida por modelo nos chats; senão 1M para `[1m]` e para as famílias Opus/Sonnet/Fable/Mythos 5, 200k para o resto; passou de 200k, pelo menos 1M | Hooks não trazem o modelo; a heurística antiga mostrava 26% numa sessão de 1M com 5% |
| 52 | **Mais observação** (Etapa 6): atividade atual e tempo no estado por agente ("3m · Editing src/x.ts", 6.4); saúde de cada worktree (mudanças, ↑/↓ contra a branch principal, "merged") e "Remove merged worktrees…" (6.6, sem merge automático, #12); comentários de revisão no diff, enviados ao terminal como texto colado, sem Enter (6.7); paleta de comandos (6.3); dois terminais lado a lado (6.11) | Ideias de Orca, Conductor, Zed, Warp e Wave; nada age sobre o agente |
| 53 | **Sem atalhos nem diálogos do navegador**: na WebView2 os atalhos do navegador ficam desligados (Ctrl+P, F5/Ctrl+R, Ctrl+F, F12…; zoom desligado) e o menu de contexto padrão só aparece em campos de texto e no editor (6.0); `alert`/`confirm`/`prompt` nunca, só diálogos do Hive, garantido por regra de lint (8.20) | Numa janela de app, imprimir, recarregar ou um "tauri.localhost diz" quebram o app ou a aparência |
| 54 | **Ícones só do Phosphor** (`@phosphor-icons/react`, regular 14 px, `currentColor`) para estados e sistema (7.6/7.7); `@react-symbols/icons` só para arquivos e pastas, monocromático (5.13) | Um só estilo; nenhum SVG desenhado à mão além do logo |
| 55 | **Fonte do terminal e do editor: Hive Mono** (7.11): IBM Plex Mono com as ligaduras do Fira Code (Ligaturizer, renomeada pela OFL) e **Symbols Nerd Font** como reserva para ícones; embutidas; ligaduras só no renderizador WebGL. A interface continua em IBM Plex Sans | Mesmo visual do Plex Mono, com ligaduras e ícones Nerd Font |
| 56 | **Arquivos pelo serviço** (7.4/8.3): criar, renomear, mover (arrastar na árvore Files para uma pasta ou a raiz) e criar pasta; nome de um componente ≤ 255 bytes, dentro da worktree, **nunca sobrescreve**, nada dentro de `.git`. A árvore Files abre o arquivo como texto editável; a aba Diff abre o diff (7.2). **Vários arquivos abertos**, cada um na sua aba, com edição, seleção e rolagem próprias (8.21) | Regras e segurança em Rust (#37); um arquivo movido para `.git` poderia virar hook |
| 57 | **Abas e ordem são estado de interface** (#37): o "+" da barra de abas abre Terminal · Agent (chat) · New file (7.5); abas de terminal, chat e arquivo na ordem em que abriram, reordenáveis por arrastar, ordem no `localStorage` por worktree (8.21); agentes reordenáveis na barra lateral só dentro da própria worktree, por arrastar ou Alt+↑/↓, ordem no `localStorage` pelo id da sessão (8.2); F8 e atalhos seguem a ordem mostrada. Arrastar e soltar nativo do HTML5: o do Tauri fica desligado (`dragDropEnabled: false`) e arquivos ou links soltos de fora são recusados | Sem dependência nova; no WebView2 o arrastar do Tauri engolia o do HTML5; um drop externo levaria a WebView para fora do app |
| 58 | **Compositor e cabeçalho do chat** (7.16, Etapa 8): caixa estilo Zed; teclas do terminal do Claude dentro do compositor (Enter envia, Esc para, Shift+Tab cicla os modos, ↑/↓ histórico das minhas mensagens do chat, Ctrl+C copia, interrompe ou limpa conforme o estado; `/` comandos); `@caminho` lista arquivos e pastas da worktree e vai como digitado, o Claude expande (8.12); seletor de modelo com a lista do `initialize` e `set_model`, só valores da lista (8.9); rascunho por chat só em memória (texto, imagens, cursor, rolagem; some ao fechar o chat, 8.14); título = primeiro prompt, depois o `/rename` (o `stream-json` não gera `ai-title`, 8.11) | O mesmo uso do terminal do Claude dentro do app; modelo e título decididos no serviço (#37) |

### Detalhes importantes dos hooks de worktree (verificado)

- `WorktreeCreate` dispara para `--worktree`, subagentes com `isolation: worktree` e sessões em segundo plano.
- **Substitui completamente** a criação padrão. O `.worktreeinclude` deixa de ser processado, então **o Hive precisa copiar os arquivos ignorados pelo git** (`.env` etc.). Isso já acontece no comando `criar worktree` da Etapa 1.
- Qualquer saída diferente de 0 aborta a criação. A CLI devolve o **caminho da worktree no stdout**; qualquer saída extra quebra a criação.
- `WorktreeRemove` dispara no fim da sessão, no fim de um subagente e ao apagar uma sessão em segundo plano. Saída diferente de 0 faz a remoção falhar se o diretório ainda existir.
- Existe um bug aberto (ago/2026) de subagente com hook `WorktreeCreate` dando erro de isolamento. Testar no spike (tarefa 1.12).
- Se um projeto tiver um hook `WorktreeCreate` próprio, ele e o do Hive vão disputar a criação. O Hive deve detectar e avisar.

### Mapeamento de estados (hooks do Claude Code → estado visual)

Revisado na 7.6. Os três 🟡 têm a mesma cor e urgência e se distinguem pelo ícone.

| Estado | Origem | Urgência |
|---|---|---|
| 🟢 Ocioso | SessionStart (`startup`, `resume`, `clear`, `fork`) | nenhuma |
| 🔵 Trabalhando | UserPromptSubmit, PreToolUse (exceto AskUserQuestion e ExitPlanMode), PostToolUse, PostToolUseFailure, SubagentStart, PreCompact (atividade "Compacting"); Notification `elicitation_complete`, `elicitation_response`, `quota_auto_resume_fired` | baixa |
| 🟡 Aguardando permissão | PermissionRequest (exceto AskUserQuestion e ExitPlanMode); Notification `permission_prompt` | alta 🚨 |
| 🟡 Aguardando aprovação do plano | PreToolUse/PermissionRequest `ExitPlanMode` | alta 🚨 |
| 🟡 Aguardando resposta | PreToolUse/PermissionRequest `AskUserQuestion`; Elicitation; Notification `elicitation_dialog`, `elicitation_url_dialog`, `agent_needs_input` | alta 🚨 |
| 🟠 Aguardando você | Stop; Notification `idle_prompt`, `quota_auto_resume_stale`, `quota_auto_resume_disabled`; interrupção (regra 2) | média |
| 🔴 Erro | StopFailure | alta 🚨 |
| 🟣 Com subagentes | SubagentStart / SubagentStop | baixa |
| ⚫ Encerrado | SessionEnd | nenhuma |

**Regras:**

1. **O mais urgente vence**, da maior para a menor urgência: permissão > plano > resposta > erro > aguardando você > com subagentes > trabalhando > ocioso > encerrado. Um subagente em qualquer 🟡 põe o pai nesse 🟡.
2. **Interrupção:** o `Stop` não dispara quando eu interrompo (Esc/Ctrl+C) nem quando recuso uma permissão, pergunta ou plano. Enquanto o agente está 🔵, 🟣 ou 🟡, o serviço lê a cada segundo as linhas novas do transcript; se a última mensagem da conversa principal é `[Request interrupted by user…]` ou a recusa de uma ferramenta, o agente e seus subagentes em 🔵/🟡 vão para 🟠 **sem pendência, som, item na caixa de entrada nem notificação** (campo `interrupted`), até o estado mudar. Reserva: 🔵 com o PTY `agents.silence_secs` em silêncio (5 s por padrão) vai para 🟠, com alerta. **Os três 🟡 não decaem por silêncio.**
3. **Compactação:** `PreCompact` mostra 🔵 "Compacting"; `PostCompact` devolve o estado e a atividade de antes; o `SessionStart` com `source: "compact"` de uma sessão conhecida não muda nada (estado, subagentes, tokens).
4. **Lembrete de permissão:** o `Notification permission_prompt` (~6 s depois de qualquer diálogo) não transforma uma pergunta ou um plano em permissão.
5. **Tarefas em segundo plano:** `Stop` com `background_tasks` não vazio fica em 🟠 com a atividade "N background tasks" (o próximo `UserPromptSubmit` limpa). Um subagente que termina o turno esperando uma tarefa que ele lançou continua listado em 🔵 (5.10) até ela sair do `background_tasks`, até o `WorktreeRemove` da worktree dele ou o fim da sessão.
6. **Hooks registrados** (#27, síncronos): os 12 de antes mais `PreCompact`, `PostCompact` e `Elicitation`.
7. **Chat:** o estado vem do stream (#42); a regra do silêncio não vale.

> Hooks de subagente trazem `agent_id` e `agent_type` no payload. O `cwd` segue o Claude para dentro da worktree. `Stop`/`SubagentStop` trazem `background_tasks` (Claude Code 2.1.282).

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
| Conceito central | "O Hive não controla e não conversa com os agentes" valendo para todo o app | O chat (#40) conversa com o `claude`; os terminais continuam só observados. A linha "SDK (antiga #1)" continua revogada: o chat não usa o pacote do SDK nem Node, mas o risco de cobrança não foi eliminado (#45, risco 11) |
| D4 (versão 3.4) | Um branch por tarefa e nunca push | Tarefas simples vão direto na `main`; o push de `task/*` passou a disparar o CI (D6) |
| — | Todos os portões rodando na minha máquina | Rodar os pesados em paralelo reiniciava a VM do WSL; foram para o CI (D6) |
| — | Notificação sempre, sem botão de mudo (lista, item 5) | Volume 0–100 % nas configurações, 0 = mudo (6.1) |
| — | Clique no sino leva ao próximo pendente (4.14) | O clique abre a caixa de entrada (6.5); F8 continua indo ao próximo |
| — | Ponto azul de não lidos na caixa de entrada (6.5) | Um só selo laranja com os pendentes ainda não vistos, zerado ao abrir o sino (8.5) |
| — | Um único 🟡 e silêncio do PTY levando 🔵/🟡 a 🟠 | Uma pergunta parecia um turno terminado; agora três 🟡 que não decaem por silêncio e interrupção lida do transcript (7.6) |
| — | Janela de contexto de 200k até o contexto passar dela (6.9) | Mostrava 26% numa sessão de 1M que estava em 5%; agora a janela real do modelo (#51) |
| — | Sessão "rodando" quando há um `claude` na mesma pasta (4.12) | Qualquer outro `claude` na pasta bloqueava sessões encerradas; agora pelo id da sessão (8.16, #20) |
| — | Chat em texto puro (padrão da 7.3) | Markdown sanitizado (8.6, #43) |
| — | IBM Plex Mono no terminal e no editor | Sem ligaduras nem ícones Nerd Font; trocada pela Hive Mono (7.11, #55) |

---

## ⚠️ Riscos

1. **Fidelidade e desempenho do terminal embutido**: a interface do Claude Code redesenha muito; com muitos agentes ativos, a interpretação dos bytes na thread da WebView pode degradar o terminal em foco. Mitigação: WebGL só nos visíveis, teste de carga na Etapa 1 e plano B com emulador headless (#28).
2. **O embrulho do `claude` depende da interface da CLI do Claude** (flag `--settings`) **e da ordem do `PATH`**. Mitigação: embrulho mínimo testado a cada atualização do Claude Code; `fish -C`; aviso quando um `claude` roda no PTY sem eventos de hook.
3. **Hooks de worktree**: `WorktreeCreate` substitui a criação padrão (o Hive replica o `.worktreeinclude`) e tem bug aberto com subagentes; `WorktreeRemove` precisa manter a barra lateral em sincronia.
4. **Vincular a worktree ao subagente certo**: hooks de subagente trazem `agent_id`/`agent_type`; falta confirmar se o `WorktreeCreate` já traz esses campos.
5. **Interface lenta mesmo com Tauri**: diffs grandes, árvores grandes e muitas atualizações em tempo real exigem virtualização e store com assinatura por agente (#30).
6. **Estado importante escondido na barra lateral**: sem o kanban, um agente pendente pode ficar num projeto recolhido. Mitigação: estado propagado + contador de pendências.
7. **Conflito de edição com o agente**: eu e o agente alterando o mesmo arquivo. Mitigação: verificação de versão ao salvar, aviso de buffer sujo, selo de agente ativo.
8. **Estado preso após interrupção**: o `Stop` não dispara em Esc/Ctrl+C. Mitigação: interrupção lida do transcript (7.6), com o silêncio do PTY como reserva para 🔵.
9. **Agentes morrem com o app**: fechar sem querer, crash ou atualização do Hive interrompe tarefas em andamento; processos que o agente deixou em segundo plano com `nohup`/`setsid` podem sobreviver ao encerramento. Mitigação: confirmação ao fechar; `claude --resume` recupera a conversa.
10. **Protocolo de controle do `stream-json` não documentado na referência da CLI** (`--permission-prompt-tool stdio`, `control_request` `initialize`/`can_use_tool`/`interrupt`/`set_permission_mode`/`set_model`): vem do SDK de código aberto e pode mudar numa versão do Claude Code. Mitigação: detectar recursos pelo `capabilities` do `system/init`; testes com gravações reais; regravar a cada atualização.
11. **A separação da cobrança do `claude -p`/SDK** (anunciada e pausada em 15/06/2026) **pode voltar**: o chat passaria a consumir outro crédito, diferente dos terminais. Mitigação: chat opcional; terminais intactos; plano B: interações ricas no `claude` interativo via hooks (Fase 2).
12. **Fontes não documentadas do Claude Code**: o registro `<config>/sessions/<pid>.json` (8.16) e o campo `background_tasks` (5.10) foram achados no binário, não na documentação. Mitigação: reservas (linha de comando e arquivo aberto; um `Stop` sem o campo volta ao comportamento antigo) e conferir a cada atualização.

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
| Conductor · Warp · Wave · ccusage | Revisão da Etapa 6: configurações, paleta, espaços, scripts, uso |
| [Modo headless do Claude Code](https://code.claude.com/docs/en/headless) · SDK Python de código aberto | Protocolo `stream-json` do chat (#40); estudo em `docs/spike/chat.md` |
| Phosphor Icons | Ícones da interface (#54) |

---

## ❓ Pontos em aberto

| # | Ponto | Situação |
|---|---|---|
| 7 | **Identificar o subagente dono de uma worktree** a partir do payload dos hooks | Resolver no spike com `hive hook --record` (movido da Etapa 0 para a tarefa 1.12 do `TODO.md`; gravações ainda pendentes) |
| 9 | **Revisão do protótipo de alta fidelidade** da Etapa 1 | Protótipo recebido e incorporado (#32, #33); pendência no ponto 13 (o 11 virou a #34 e o 12 a #35) |
| 13 | **Referência visual para os agentes**: sem o `support.js`, os agentes não renderizam o protótipo; capturas PNG das telas 1a–1g (exportadas do Claude Design) permitiriam comparar a implementação com o desenho | Capturas ainda não exportadas |
| 14 | **`claude` nativo no macOS não é detectado** (5.12): o processo leva o nome da versão (`<x.y.z>`), então não há aviso de terminal sem hooks nem sessão marcada como rodando | Ler o argv[0] pelo `KERN_PROCARGS2`; precisa de `libc` como dependência direta (aguarda minha aprovação) |
| 15 | **Endurecimento do Tauri** (revisão de segurança da 8.6): sem bloqueio de navegação e de novas janelas no builder, permissão `opener:allow-open-path` ampla (`**`), CSP nula | A decidir |

**A verificar no spike de hooks (tarefa 1.12, movida da Etapa 0):**

- Se o `WorktreeCreate` disparado por um subagente traz `agent_id`/`agent_type`.
- Se o spinner do Claude continua escrevendo no terminal durante ferramentas longas (reserva da regra 2 do mapeamento).
- O bug do `WorktreeCreate` com subagentes.
- Se o Claude Code recusa editar um arquivo alterado desde a última leitura e o relê (lado do agente no conflito de edição).
- Estados (7.6, `docs/spike/agent-states.md`): quais hooks disparam para AskUserQuestion/ExitPlanMode, as linhas exatas de recusa no transcript, a ordem da compactação e o que acorda um `Stop` com tarefas em segundo plano; o `background_tasks` da 5.10 numa sessão real.
- Chat (7.3): os cenários de gravação da Etapa 7 (`scripts/spike/record-chat.py`); os da Etapa 8 já foram gravados (claude 2.1.283).

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
| Gestão de consumo de IA | **Parcial**: tokens e % de contexto por agente, lidos dos transcripts (#51); sem limites de 5 h/semana |
| Alocação de RAM/processamento para agentes | Pós-v1; custo alto (cgroups no WSL), ganho duvidoso |
| Orquestração de agentes | **Descartado**: conflita com o Hive como observador; a orquestração é feita pelos subagentes do Claude (o chat, #40, é uma conversa conduzida por mim, não orquestração) |
| Notificação (som + SO) | **Incorporado** à Etapa 2 (som só em 🟡, 🟠 e 🔴); sem alerta para o agente à vista com a janela em foco (5.6); volume com mudo (6.1), caixa de entrada (6.5), selo de não vistos (8.5) |
| Edição de arquivos | **Incorporado** à Etapa 3b |
