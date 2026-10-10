# Design

How New Terminal's promises turn into interaction: keys, syntax, flows, and mechanics. [VISION.md](VISION.md) says what New Terminal is and why, and rarely changes. This document changes as I build. Words are defined in [GLOSSARY.md](GLOSSARY.md).

## What the one-prompt constraint forces

Every row is a place where a usual design needs a second place to type, and how New Terminal does it through the one prompt instead.

| Problem | The usual answer | Under the constraint |
| ------- | ---------------- | -------------------- |
| Where does my request go? | Whichever window has focus | The prompt always shows the **target**. A mention changes it. |
| Working on several projects | A window or tab each | Mention them. Results come back labeled. |
| Running a command | Type it yourself | Say what you want. An agent runs the commands, and each one shows up as a block. |
| Talking to a specific agent | Its own chat box or terminal | Mention it at the prompt. |
| Answering an agent's question | Find its window | Questions wait in the attention queue. Bring up the next one with one key, or mention its source to answer any one. |
| Several conversations at once | A window per conversation | Each request is a **thread**. Threads interleave at the prompt, labeled, and the prompt always shows which one your next line answers. |
| Interactive programs (REPL, debugger, database shell) | A pane with its own input | Ask. An agent drives the program in a pane while you watch. |
| `ssh` | A terminal per server | A remote machine is a project. Mention it and the work happens there. |
| Full-screen programs (an editor, `top`) | They take over the window | You never type into them. Editors stay in your editor app. Panes replace monitors like `top`. |
| Finding something | A separate palette | The palette is the prompt's own completion. |
| Panes | Each has its own input | Panes are views. They show what agents are doing; you type only at the prompt. |
| Focusing on one workspace | A new window | Focus sets the prompt's target to that workspace. |

## The prompt

- **Target.** The prompt shows the target before you type: `api billing ›` means your next request goes to api and billing. A mention changes it, and the target sticks until you change it again.
- **Mentions.** `@name` mentions a project, a workspace, or an agent. Several mentions send one request to all of them: `@api @billing bump the shared client to v2 in both`.
- **Completion.** One key opens completion inline, Raycast-style: fuzzy search over projects, workspaces, agents, past requests, project scripts, and scrollback. Picking a result does it.
- **Focus.** Focusing on a workspace shows it full screen with its panes laid out, and sets the target to it. It never adds a second prompt.

## Interactive programs

You never type into a program; the prompt stays the one place you type.

- **Ask, and an agent drives it.** When work needs a REPL, a debugger, or a database shell, say what you want to know. An agent runs the program in a pane while you watch, and the answer comes back to the prompt.
- **Remote machines are projects.** There's no `ssh` session to manage. Mention the machine and the work happens there.
- **Full-screen programs stay outside.** Editors stay in your editor app. Panes replace monitors like `top`.

## Blocks

Every command an agent runs becomes a block: input, output, exit code, duration, and where it ran. Blocks are plain text with structure underneath. You can jump between them, collapse noisy output, copy just the output you need, or hand a failed block back to an agent with one key ("fix this").

## Diff review

Finished agent work opens as a diff: side-by-side or unified, syntax-highlighted, with ways to accept a hunk, reject it, comment back to the agent, or merge the branch, all from the keyboard.

## Workspaces

- **Starting.** A workspace starts empty, or from a new branch, an existing branch, a pull request, or an issue.
- **Isolated copies.** A workspace that includes a repository gets its own branch and its own copy, so parallel agents never step on each other. An isolated copy separates code, not permissions; for a real security boundary, the agent runs in a sandbox.
- **Project scripts.** A small file checked into the repository declares three hooks: *setup* (install dependencies, copy env files), *run* (start the dev server, watchers, test loops), and *archive* (clean up anything left behind).
- **Port blocks.** Each workspace gets its own block of ports, exposed as environment variables, so five agents can each run the app at once.
- **Merge readiness.** One view shows git status, CI checks, review comments, open todos, and the diff. Merging is blocked or flagged when something is still open.
- **Lifecycle.** Create → work → verify → review → open PR → merge → archive. Archived workspaces disappear from view and can be restored from history.

## The agent interface

Agents do all of the work, so they need the terminal more than anyone. Without the agent interface, an agent runs commands through a bare subprocess: no TTY, no shared view, no way to drive an interactive program. It can't watch a dev server that's already running, step through a debugger, answer a REPL prompt, or show you a test run as it happens. The agent interface gives agents the terminal you watch, through one protocol with a clear permission model.

Agents that use the interface report their status, permission requests, files touched, and cost. Agents that don't still work; they just get fewer affordances. The status bar, the attention queue, and diff review are all built on this interface: New Terminal's own features need no private API, so nobody else's does either.

| Verb | Examples |
| ---- | -------- |
| **Observe** | Read a pane's screen or scrollback. List panes, agents, and their status. Read blocks as structured data: command, output, exit code, duration, where it ran. Subscribe to events such as "command finished", "output matched a pattern", or "pane exited". |
| **Act** | Run a command in a pane and get back its finished block, not a scraped string. Send keystrokes to interactive programs (a REPL, a debugger, any TUI). Open, split, and close panes. Open a diff in review. |
| **Delegate** | Start another agent in its own workspace, locally or in the cloud, and wait for its result. |
| **Ask** | Report status (`working`, `waiting`, `done`, `failed`), request a permission, or ask you a question. Each one lands in the attention queue. |

Agents reach the same verbs from a shell, as MCP tools, or from any program. An agent that can only print text can still report its status.

```bash
term pane split --right --name dev
term run --pane dev "npm run dev" --wait-for "ready on"
term run --pane tests "npm test" --json   # returns the finished block as JSON
term send-keys --pane debugger "n" Enter
term attention ask "Migration drops a column. Proceed?" --options yes,no
```

### Safety

An agent that can drive a terminal can do real damage. The rules:

- **Scoped by default.** Each agent's permissions cover only the panes it created and its own workspace. Reading other panes or starting more agents is a separate grant you approve once, per agent or per project.
- **Visible always.** A pane being driven by an agent shows it in the pane header and cursor. Nothing happens in a pane you can't see happening.
- **You win.** Your requests always take priority. One key stops every agent at once.
- **Logged.** Every call an agent makes is recorded in the session, so you can replay what it did and why.

## The first version

The first version is the smallest New Terminal I can use to build New Terminal. It keeps the one prompt and leaves most of this document for later. [BUILDING.md](BUILDING.md) says how to build and run it.

- **Mentions lead the line.** A mention counts only at the start of a line, so `@tauri-apps/api` later in a request stays plain text. A line that is only a mention sets the target.
- **One target per request.** A line with two mentions fails, and the app sends nothing. Send one line per target.
- **Four lines go to the app, never to an agent.** Names use lowercase letters, digits, and hyphens, up to 32 characters. To send these words to an agent, start the line with a mention.
  - `add project <name> <path>` adds a project.
  - `new workspace <name> @<project>` creates a workspace on a git project.
  - `archive workspace <name>` archives a workspace.
  - `list` shows every project and workspace.
- **Reply keys.** When an agent waits on you, the status bar shows `needs you`. `Tab` on an empty prompt brings up the request, with everything it would run. `y` allows it, `n` denies it, and `Esc` puts it back for later.
- **Questions arrive as text.** An agent asks its question in its reply, with the options. You answer with a line at the prompt, the same as any request. A yes-or-no question takes a typed word, not one key.
- **Edits inside the target go ahead.** An agent edits files inside its project or workspace without asking. Anything else the agent asks permission for, such as a shell command, waits for your answer.
- **Setup files always ask.** An edit to a file that the agent's tools load as their own settings asks first, even inside the target. That is any path with a part that starts with `.`, such as `.git` or `.claude`, and files such as `CLAUDE.md` and `AGENTS.md`.
- **`⌘.` stops every agent.** It ends each agent's process and the commands it started in the background. If an agent ignores the stop, the app kills it and warns that those commands can still run. The next request to that target starts a new agent with no memory of earlier requests. Quitting stops every agent the same way.
- **Where workspaces live.** A workspace gets an isolated copy of its project at `~/.new-terminal/workspaces/<workspace>/<project>`, on its own branch, `nt/<workspace>`. Archiving removes the copy and keeps the branch. It refuses while the copy has changes that are not committed, or a commit that no branch holds.
- **Light or dark, with the system.** The window follows the system appearance and switches when it does. A request that waits on you is the one solid shape, and it inverts the ground.
