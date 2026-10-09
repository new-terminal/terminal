# Glossary

The shared language for New Terminal. Docs, code, issues, and conversations all use these words with these meanings. When a term here changes, it changes everywhere.

Each entry gives the definition, then words to avoid (**Avoid**) and nearby terms it's easy to mix up (**Not**).

## The core idea

### Prompt

The one place you type, and the single interface where you provide your intent. You say what you want at the prompt, mention the projects and workspaces it's for, and agents do the work. Everything that needs you comes back to it. There is exactly one prompt, never one per project, window, pane, or agent. This is the core design constraint.

- **Avoid:** command line (you never type commands), pane of glass, main window, console, chat box.

### Intent

What you type at the prompt: what you want done, in your own words. Never a shell command; agents turn intent into commands.

- **Avoid:** command, query, message.

### Intent over location

The principle behind the prompt. You say what you want and name where it goes. You never navigate to where something lives first.

### Target

Where your next line goes, always shown in the prompt. A target is a project, a workspace, an agent, or a [thread](#thread) you're replying to. Only you change it: a mention, focusing on a workspace, or bringing up a question from the [attention queue](#attention-queue). It never changes on its own.

- **Avoid:** context, scope, active window.

### Mention

Naming a project, workspace, or agent in a request so the request goes there, for example `@api`. A request can mention several, and it goes to all of them. A mention also sets the [target](#target).

- **Avoid:** tag, address, target.

### Route

What the terminal does with a request: it sends the request to the projects and workspaces it mentions, wherever they live, and brings the results back to the prompt, labeled by where they came from.

- **Avoid:** dispatch, forward, proxy.

### Thread

A request you made plus everything that follows from it: the agent's progress, its questions, your replies, and the result. A thread belongs to one workspace and agent. Threads interleave in the scrollback, each labeled; they're structure, not separate windows or inputs.

- **Avoid:** conversation, chat, session.

### Reply

A line that answers a question in a [thread](#thread). Bring up the next question, or mention its source to answer it out of order. When the prompt is a reply, it shows the question.

### Focus

Setting the [target](#target) to a single workspace and showing it full screen, with its panes laid out. Focus never adds a second prompt. It's optional; everything a workspace does is also reachable without it.

- **Avoid:** open, enter, attach (attach means something else; see [remote attach](#remote-attach)).

## Things you work with

### Project

Something you work on, such as a codebase or a system. A project lives on your machine, on a remote machine, or in a cloud sandbox, and where it lives is a detail of the project. Projects last.

- **Avoid:** repo (a project can be a repository, but doesn't have to be), folder, directory.
- **Not:** [workspace](#workspace).

### Workspace

A task in progress. A workspace brings in the projects the task needs and holds its shells, agents, running processes, history, and diff. It isn't tied to a folder. Workspaces come and go.

- **Avoid:** session, worktree, branch, environment.
- **Not:** [project](#project). A project is what you work on; a workspace is the work.

### Agent

A long-running job that does work for you, with a terminal, a workspace, a [status](#status), and a set of [grants](#grant). Any agent that runs in a terminal counts, from any provider.

- **Avoid:** bot, assistant, AI, copilot.

### Default agent

The agent that handles requests that don't mention one. You choose it: Claude Code, Codex, the built-in agent, or any other.

### Built-in agent

The agent that ships with New Terminal, so the prompt works out of the box. You never have to keep it as your [default agent](#default-agent).

### Provider

A company or service that supplies an agent or a model. Any provider works.

### Adapter

A small piece that connects a provider's hosted agent to the [agent interface](#agent-interface), so it behaves like any other agent.

- **Avoid:** plugin, integration, connector.

## Where work runs

### Local

On your own machine, running as you.

### Sandbox

An isolated place to run an agent, so it doesn't run as you. A sandbox can be on your machine or in the cloud.

- **Not:** a workspace's isolated copy of a repository, which separates code, not permissions.

### Remote machine

A machine of yours that you reach over the network. Projects and agents on it work the same as local ones.

### Cloud

Sandboxes hosted by New Terminal or by a provider. Agents in the cloud keep working when your laptop is closed.

### Remote attach

Reaching your sessions from another machine or from your phone.

## Agents and you

### Status

What an agent is doing: `working`, `waiting`, `done`, or `failed`.

### Attention queue

Everything waiting on you, from every agent and workspace: permission requests, questions, and finished work. Items wait until you bring them up or answer them with a mention; the queue never takes over the prompt on its own.

- **Avoid:** inbox, notifications, alerts (a notification is how the queue reaches you when the app is in the background).

### Grant

A permission you give an agent beyond its default scope, such as reading other panes or starting more agents.

- **Avoid:** token, capability, role.

### Agent interface

How agents use the terminal directly. It has four verbs:

| Verb | Meaning |
| ---- | ------- |
| **Observe** | Read panes, blocks, and status, and subscribe to events. |
| **Act** | Run commands, send keystrokes, and manage panes. |
| **Delegate** | Start another agent in its own workspace. |
| **Ask** | Report status, request a grant, or ask you a question. |

Extensions use the same interface.

- **Avoid:** API, protocol, SDK (in user-facing text).

## The screen

### Pane

A region of the screen showing what an agent, its shell, or a program is doing. Panes show output; you never type into a pane. All input goes through the prompt.

- **Not:** the [prompt](#prompt). There is one prompt and many panes.

### Block

One command an agent ran, with its input, output, exit code, duration, and where it ran, kept together as a unit.

### Session

An agent or its shell, and the scrollback. Sessions outlive the window: they survive closing the app and rebooting.

- **Not:** [workspace](#workspace). A workspace holds sessions.

### Status bar

The single line at the bottom of the screen showing running jobs and what needs you.

### Leader key

The key that opens [completion](#completion).

### Completion

The prompt's fuzzy search over projects, workspaces, agents, past requests, project scripts, and scrollback, opened with the leader key. Picking a result does it. It's part of the prompt, not a separate palette.

- **Avoid:** command layer, command palette, launcher.

### Interactive program

A program that reads input as it runs, such as a REPL, a debugger, or a database shell. You never type into one: you ask, and an agent drives it in a pane while you watch. Full-screen programs like editors run outside New Terminal.

## Getting work merged

### Diff review

Reading an agent's changes and accepting, rejecting, or commenting on each hunk.

- **Avoid:** code review (that's what people do on a pull request).

### Merge readiness

One view of everything that has to be true before merging: git status, CI checks, review comments, open todos, and the diff.

### Project scripts

Three hooks a project declares for its workspaces:

| Script | When it runs |
| ------ | ------------ |
| **Setup** | When a workspace brings the project in. |
| **Run** | When an agent starts the project's app, watchers, or test loops. |
| **Archive** | When the workspace is archived. |

### Port block

The range of ports each workspace gets, so parallel workspaces can run the same app at once.

### Archive

The end of a workspace's life. Archived workspaces disappear from view and can be restored.

- **Avoid:** delete, close (nothing is lost).

## Everything else

### Extension

Something that adds to New Terminal, running on its own. A crashing extension can't take down the app.

- **Avoid:** plugin.

### Omakase

My approach to settings: I choose good defaults instead of offering a toggle for everything.
