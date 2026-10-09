# Vision

I started with one constraint: **there is only one prompt.**

Not one per project, per window, per pane, or per agent. One. Every time a feature seemed to need a second place to type, I had to find a way to do it through the first. Those answers became the product: mentions instead of windows, a prompt that always shows where your next request goes, and agents that do the work and bring their questions back to you. See [The constraint](#the-constraint-one-prompt).

The prompt is the single interface where you provide your intent. Agents do all of the work. You stop spending cycles on switching windows and deciding where to type what.

New Terminal is an experience for directing agents, not for editing files. It's built for people who direct agents more than they edit files.

> **Every project. Every agent. One terminal.**
>
> One prompt. Agents do the work.
>
> You don't go to the work. The work comes to you.

## A new way of working

### The old way

Developer tools are organized around *places*. You `cd` into a folder. You open a window per project, a tab per server, a terminal per agent. To work on something, you first go to where it lives, then type the commands yourself.

That was fine when one person did one thing at a time. It breaks when you're running agents. Now there are five streams of work across three projects, two machines, and a cloud sandbox, and every one of them lives somewhere different. You become the router: switching windows, re-finding the right tab, checking which agent is waiting, copying context from one place to another.

Every switch costs a little thought. Which window is api in? Is this terminal the server or the agent? Where do I type this? Did I just type it into the wrong one? None of that is the work, but it eats the attention the work needs. The developer is the glue, and the glue is tired.

### The new way

Work is organized around *intent*. You stay at one prompt and say what you want and where it should go. "Run the tests in api." "Bump the shared client in api and billing." "What's fix-auth waiting on?" Each request goes to the right project or workspace, wherever it lives. Agents turn it into commands and do the work, and the results come back to you.

There's no shell environment to set up: no dotfiles, no `PATH`, no version managers on every machine. Projects declare what they need, and agents set it up.

You stop spending cycles on switching windows and deciding where to type what. You spend them on what you want done.

| | The old way | The new way |
| --- | --- | --- |
| **Where you work** | A window or folder per project | One prompt for everything |
| **What you type** | Commands | Intent |
| **Setting up** | A shell, dotfiles, `PATH`, and version managers on every machine | Nothing to set up: projects declare it, agents run it |
| **Who does the work** | You, one command at a time | Agents, in parallel |
| **How work starts** | Navigate there, then act | Name it, and the request goes there |
| **Where things live** | Your machine, unless you switch tools | Local, remote, or cloud; a detail of the project |
| **Knowing what needs you** | Check every window | It comes to you, at the same prompt |
| **What you think about** | Which window, which tab, where to type | Only what you want done |
| **Your role** | Operator of each tool | Director of all the work |

## The constraint: one prompt

There is exactly one prompt. Not one per project, per window, per pane, or per agent. One.

You type intent there and nothing else. You never type a shell command; agents do that.

This is the core design constraint, and every other decision follows from it. When a feature seems to need a second place to type, I find a way to do it through the first. The constraint is how I keep the promise of the new way: you never decide where to type, because there is only one place.

[DESIGN.md](DESIGN.md#what-the-one-prompt-constraint-forces) works through what the constraint forces, case by case.

## The bet

A terminal was originally the human's end of a line to a computer doing work somewhere else. You typed what you wanted, and the machine did it. Over time, terminals shrank into a window running a shell on your own laptop.

New Terminal goes back to the original idea. The machines doing the work are now agents, spread across your laptop, your servers, and the cloud. The prompt is your end of the line to all of them.

This is a terminal where:

- **Intent beats location.** You name what you want to work on. You never have to go to where it lives.
- **There is exactly one prompt.** Every project, workspace, and agent is reachable from it, and everything that needs you comes back to it.
- **You give intent; agents do the work.** You never type a command. Agents run them, and you watch, steer, and review.
- **Agents are processes, not personalities.** An agent is a long-running job with a terminal, a workspace, a status, and a permission scope. The app manages agents the way an OS manages processes.

## Principles

### 1. Intent, not location

No cycles spent on switching windows or deciding where to type what.

- **Name it, don't navigate to it.** Mention a project, a workspace, or an agent by name and the request goes there. Mention several and it goes to all of them.
- **Location is a detail.** A project can be a folder on your laptop, a remote machine, or a cloud sandbox. You work with all of them the same way.
- **Everything comes back to you.** Results, questions, and finished work arrive at the one prompt, labeled by where they came from.
- **Focus is optional.** Focusing on a workspace sets the prompt's target to it, for depth. It never adds a second prompt.

### 2. Fast and interactive

Speed is a core promise. Everything responds instantly, everything works from the keyboard, and a slowdown is a bug.

### 3. Omakase

The chef chooses. I ship one carefully tuned experience instead of a thousand toggles.

- One font that I ship and tune, one default theme pair (light/dark) that follows the OS, one keymap.
- Sensible integration out of the box: git awareness, worktrees, notifications, inline images, clickable links, true color.
- Configuration exists, in one plain text file, for the things people legitimately need to change: font, size, theme, keybindings, default agent. If a setting exists mostly to settle an argument, I don't add it.
- No plugin marketplace. [Extensions](#extensibility) use a small, stable API.

### 4. Cross-platform, natively

macOS, Linux, and Windows are first-class. A [companion app](#on-your-phone) runs on iOS and Android.

Native means native. No web app in a window, no lowest-common-denominator look. On macOS it feels like a Mac app. On Linux it respects the user's desktop. On Windows it feels at home next to Windows Terminal.

### 5. Open source, honestly

New Terminal is open source under a permissive license. Not open core with the good parts ripped out: the whole local experience, agents included, is free forever.

Premium features are things that cost me money to run or that only make sense for teams. The test for any paid feature: **"Would a solo developer on a plane miss it?"** If yes, it's free.

## The experience

### One prompt

You open the app. You see a prompt, a cursor, and a one-line status bar. No project picker, no file tree, no folder to choose first, no shell, and no wall of windows.

The prompt is where you work across everything. Say what you want, and mention a workspace or project to send it there. Mention two and it goes to both. Agents do the work. Their commands, questions, and finished work come back to the same place, labeled by where they came from.

```
› @api run the tests
┌ api ─ fix-auth ────────────────────────────────────────────────────┐
│ $ npm test                                                         │
│   ✓ 214 passing (3.2s)                                             │
└────────────────────────────────────────────────────────────────────┘
› @api @billing bump the shared client to v2 in both
┌ api ─ agent ● working ─────────────────────────────────────────────┐
│ Editing package.json  +1 −1                                        │
│ Running npm test                                                   │
└────────────────────────────────────────────────────────────────────┘
┌ billing ─ agent ● working ─────────────────────────────────────────┐
│ Editing package.json  +1 −1                                        │
└────────────────────────────────────────────────────────────────────┘
? fix-auth    Allow `git commit`?               [y] yes  [n] no
✓ perf-audit  3 N+1 queries found. Diff ready.  [r] review
api billing › █

 4 workspaces │ 3 agents │ 1 needs you │ ⌃␣ commands            12:04
```

You typed "run the tests". An agent ran `npm test`. That's the whole division of labor.

The prompt always shows the **target**: where your next request goes. Nothing is ever typed into the wrong place, because there's only one place to type.

You *can* focus on a single workspace when you want depth. It never adds a second prompt, and you never have to. Working across three projects shouldn't mean juggling three windows.

### The agent behind the prompt

Every request goes to an agent. You choose which one handles requests by default: Claude Code, Codex, or any other. A built-in agent ships with the app, so the prompt works the moment you open it, and you never have to keep using it. Mention a different agent to hand it one request.

### Agents

Agents do all of the work, in parallel. You direct it from the prompt. The idea of agents as first-class panes comes from cmux; I push it further.

- **Any agent, any provider, no lock-in.** Claude Code, Codex, Cursor, OpenCode, and whatever ships next run as normal processes in normal panes, against whichever model provider you choose. You keep the agents you like. They just run better here than anywhere else.
- **Local or cloud, same experience.** An agent running on your laptop and one running in a [cloud sandbox](#local-and-cloud-agents) look and behave the same: a pane, a status, a workspace, a diff.
- **Agents use the terminal directly.** They work in the panes you watch, with only the permissions you grant. How that works is in [DESIGN.md](DESIGN.md#the-agent-interface).
- **One workspace per task.** Starting a task creates an isolated [workspace](#projects-and-workspaces) with its own branch, so parallel agents never step on each other.
- **The attention queue.** The real bottleneck in agentic work is the human. Every agent's permission requests, questions, and finished diffs wait in one queue, and you answer them at the prompt. Native OS notifications fire when the app is in the background. You never have to poll.

### Projects and workspaces

Two names you can mention from the prompt:

- A **project** is something you work on: a codebase or a system. It can live on your laptop, on a remote machine, or in a cloud sandbox. Projects last.
- A **workspace** is a task in progress. It brings in the projects it needs and holds the task's agents, their shells and running processes, its history, and its diff. Workspaces come and go.

The workspace as the unit of work comes from Conductor. I take it further: a workspace isn't tied to a folder, and you never have to open one to use it.

- **Not tied to a filesystem.** A workspace doesn't live in a directory, and it starts empty. You bring in what the task needs.
- **Bring in any project.** A workspace can bring in several projects, such as your local checkout and the staging server you're debugging against.
- **Everything for one task in one place.** You can reach a workspace by name from the prompt, or focus on it. Either way, its agents and history come with it.
- **Isolated when there's a repository.** Parallel agents never step on each other, in files or in ports.
- **Ready to work in.** A project declares how to set up, run, and clean up its workspaces, so agents start working instead of debugging setup.
- **Merge when it's ready.** Everything that has to be true before merging shows up in one place.

An isolated copy separates code, not permissions. Agents working on your local machine still run as you. When you need a real security boundary, run the agent in a [sandbox](#local-and-cloud-agents). The mechanics are in [DESIGN.md](DESIGN.md#workspaces).

### Local and cloud agents

Where an agent runs is a setting, not a different product. These are the same systems a workspace can bring in. Every agent, wherever it runs, is a session with a pane, a status, a workspace, and a diff, and it shows up in the same attention queue.

| Runs on | How it connects | Cost |
| ------- | --------------- | ---- |
| **Your machine** | A process in a local pane | Free |
| **A local sandbox** | A container or VM on your machine, for agents you don't want running as you | Free |
| **Your own remote machine** | SSH or a dev container; the session runs there and streams to your panes | Free |
| **A provider's cloud** | Adapters for agent providers' hosted runs (for example, cloud tasks from coding-agent vendors) | Free here; you pay the provider |
| **New Terminal's cloud** | Hosted sandboxes that keep running with your laptop closed | Premium |

- **Any provider.** An adapter maps a provider's agent onto the agent interface. The adapter spec is open, so providers and the community can write their own.
- **Any model.** Agents that let you choose a model can use any provider's API key or a local model. Your keys go straight to the provider, never through the app, and no special endpoints are required.
- **Move work between them.** Start a task locally, send it to the cloud when you close the lid, and pull it back to review the diff at your desk.

### Sessions that survive

Every pane outlives the window. Close the window, quit the app, reboot: your agents, their shells, and their scrollback come back exactly where they were.

### On your phone

Agents keep working when you walk away. The mobile app is how you keep directing them.

- **The same prompt.** Say what you want from your phone, mention a project or workspace, and the request goes there, exactly as it does on your desk. Same look.
- **The attention queue in your pocket.** Push notifications when an agent needs you. Approve a permission, answer a question, or say "keep going" from the lock screen.
- **Review on the go.** Read an agent's summary and diff, comment back, or merge when checks are green.
- **Watch any pane.** Open a live view of any agent's session on your desktop or in the cloud.
- **Private by default.** Your phone connects to your sessions end-to-end encrypted. Only the connection is relayed, never your code.

Because you only ever type intent, the phone loses nothing: no shell to squeeze onto glass, no keys it lacks. Unblocking five agents from a coffee line is the same as doing it at your desk.

## Extensibility

Extensions follow two rules:

1. **Extensions run on their own,** using the same interface as agents. Any language. A crashing extension can't take down the app.
2. **Small surface, stable contract.** Ten APIs I keep forever beat a hundred I regret.

## Non-goals

- **Not a shell.** You never type commands; agents do. When you want a raw shell, any other terminal is right there.
- **Not a window manager.** You shouldn't need a window per project, a tab per server, or a terminal per agent.
- **Not an editor or an IDE.** New Terminal is for directing agents, not editing files. No file tree, no editor, no debugger UI. When you want to edit by hand, your editor is right there in its own app.
- **Not a chat app.** There's one prompt for everything, not a chat window per agent.
- **Not infinitely configurable.** If you want to tune every pixel, there are excellent terminals for that. I'd rather be opinionated and right.
- **Not a web app.** No Electron, no cloud dependency for core use, no account required.
- **Not a walled garden.** It never requires its own model, cloud, or agent.

## Inspirations, and what I take from each

| Project | What I take |
| ------- | ------------ |
| **Ghostty** | Truly native on every platform. Speed and correctness as non-negotiables. Zero-config defaults that are actually good. |
| **Warp** | Proof that a terminal can be rethought, not just made faster. Output grouped into blocks. A real text editor for the input line. Agents built into the terminal, with room for third-party ones. |
| **Raycast** | One keystroke to any action. Fuzzy everything. Speed as a brand promise. |
| **cmux** | Agents deserve their own panes, status, and notifications. The terminal should know when an agent is waiting for you. |
| **Claude Code** | The agent belongs in the terminal. Permission prompts, tool calls, and diffs can all be rendered as text without losing anything. |
| **Codex** | Parallel, sandboxed agent tasks that produce reviewable diffs. Local and cloud execution behind one interface. |
| **Conductor** | The workspace as the unit of work. Setup, run, and archive scripts. Per-workspace ports. A single merge-readiness view. A clear lifecycle from task to archive. |
