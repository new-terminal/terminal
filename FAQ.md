# FAQ

Short answers to the questions people ask first. The full picture is in [VISION.md](VISION.md).

## The basics

### What is this?

An experience for directing agents, not for editing files. It gives you one prompt where you say what you want, across all your projects. Agents do all of the work. Instead of opening a window per project and typing commands yourself, you name the project or workspace and the request goes there. Results, questions, and finished work come back to the same prompt. Projects can be on your laptop, on a remote machine, or in the cloud.

It runs on macOS, Linux, and Windows.

### Who is it for?

People who direct agents more than they edit files. If you spend your day editing files by hand, an editor is the better tool. If you spend it starting, steering, and reviewing agents across several projects, it's built for you.

### What's different about the way I'd work?

With most tools, you go to the work: `cd` into a folder, open a window per project, type the commands, and check each terminal to see which agent needs you. Here the work comes to you. You stay at one prompt, say what you want and where, and agents do it. You direct the work instead of running each tool by hand.

The payoff is focus. You stop spending cycles on switching windows and deciding where to type what, and spend them on what you want done. See [A new way of working](VISION.md#a-new-way-of-working).

### What's it called?

New Terminal. The website lives at [terminal.new](https://terminal.new). Agents drive it with a command-line tool called `term`.

### Why "Terminal" in the name if I never type commands?

A terminal was originally the human's end of a line to a computer doing work somewhere else. New Terminal goes back to that meaning. The machines doing the work are now agents, on your laptop, your servers, and the cloud, and the prompt is your end of the line to all of them. That's what's new. See [The bet](VISION.md#the-bet).

### Can I type shell commands?

No. You type intent, and agents turn it into commands. Every command an agent runs shows up as a block you can read, copy, or hand back with "fix this". When you want a raw shell, any other terminal is right there.

### Do I have to switch between windows to work on several projects?

No. Everything happens at one prompt. Mention a workspace or project by name and your request goes there; mention several and it goes to all of them. Results and questions from every workspace come back to the same place. You can focus on a single workspace, but you never have to. See [One prompt](VISION.md#one-prompt).

### Why only one prompt?

Because it's the only way to never decide where to type. It's the core design constraint: there is exactly one prompt, and every feature works through it. The prompt always shows where your next request goes. See [The constraint](VISION.md#the-constraint-one-prompt).

### How do I answer several agents at once?

Questions wait until you're ready; the prompt never switches on its own. Bring up the next one, or mention the agent or workspace whose question you want to answer. Simple yes-or-no questions take a single key. If a reply could belong to more than one conversation, the prompt asks first.

### What about REPLs, debuggers, and editors?

You never type into a program. Ask what you want to know, and an agent drives the REPL, debugger, or database shell in a pane while you watch. Editors stay in their own app. See [Interactive programs](DESIGN.md#interactive-programs).

### Is there a mobile app?

Yes, for iOS and Android, with the same prompt. Because you only ever type intent, nothing is lost on a phone: give requests, answer agents' questions, approve permissions, review diffs, and watch any agent's session. See [On your phone](VISION.md#on-your-phone).

## Agents

### Which agent handles my requests?

The one you choose as your default: Claude Code, Codex, or any other. A built-in agent ships with the app, so the prompt works the moment you open it. Mention a different agent to hand it a single request. See [The agent behind the prompt](VISION.md#the-agent-behind-the-prompt).

### Why do agents need a terminal?

Because that's where they do all of the work. Most commands work fine through a plain subprocess, but a real terminal matters in three cases:

1. **Interactive programs.** REPLs, debuggers, `ssh`, prompts, and TUIs need a TTY.
2. **Long-running processes.** An agent should be able to watch a dev server that's already running, not launch its own.
3. **The environment.** A shell carries auth, environment variables, and tool versions.

The bigger reason is for you. When an agent works in a pane you can see, you can supervise it live and stop it with one key.

### Which agents does it support?

Any agent that runs in a terminal, including Claude Code, Codex, Cursor, and OpenCode. Hosted agents from providers' clouds connect through adapters, and the adapter spec is open. They run as ordinary processes in ordinary panes. Agents that use the [agent interface](DESIGN.md#the-agent-interface) get extras: status in the status bar, permission requests in the attention queue, and direct control of panes.

### Do I have to use the built-in agent?

No. It's there so everything works out of the box, it's free, and it supports bring-your-own-key for major model providers and local models. Choose any other agent as your default. Nothing in the app requires its own model, cloud, or agent.

### What stops an agent from doing something destructive?

By default an agent can only touch the panes it created and its own workspace. Anything more, such as reading other panes or starting more agents, needs your approval. Agent control is always visible, your requests always take priority, one key stops every agent, and every action is logged. See [Safety](DESIGN.md#safety).

### What's the difference between a project and a workspace?

A project is something you work on, such as a codebase or a server, and it lasts. A workspace is a task in progress: it brings in the projects it needs and holds the task's agents and history. You can mention either by name. See [Projects and workspaces](VISION.md#projects-and-workspaces).

### Why does each task get its own workspace?

So parallel agents never edit the same files, or run the app on the same port, as each other. A workspace holds a task's agents, their processes, and the diff. It isn't tied to a folder: you bring in local repositories, remote machines, or cloud sandboxes. When it includes a repository, it gets its own branch and isolated copy. Project scripts set it up and tear it down, and you review and merge the results. See [Projects and workspaces](VISION.md#projects-and-workspaces).

A workspace isolates code, not permissions. For a real security boundary, run the agent in a sandbox.

### Can agents run in the cloud?

Yes. Agents can run on your machine, in a local sandbox, on your own remote machine over SSH, in an agent provider's cloud, or in New Terminal's hosted cloud. They all look the same: a pane, a status, a workspace, and a diff. Only the hosted cloud is paid. See [Local and cloud agents](VISION.md#local-and-cloud-agents).

### Which model providers does it support?

Any. Agents that let you pick a model can use any provider's API key or a local model. Your keys go straight to the provider, never through the app.

## Comparisons

### Why not just use my IDE?

IDEs are organized around a folder: one window per project, with files at the center. Agent work spans many projects at once and is organized around intent. You won't find an IDE here: no file tree, no editor, no debugger UI. When you want to edit by hand, your editor is right there in its own app.

### How is this different from Ghostty?

Ghostty is a fast, correct terminal for running your shell, and a big influence on this one. New Terminal works one layer up: you don't type commands, agents do. You give intent at one prompt across every project, and it shows what agents are doing as blocks and panes.

### How is this different from tmux?

tmux multiplies places to type: sessions, windows, and panes, each with its own shell. Here you have one prompt. Panes show what agents are doing, and you never switch between them to find your work; you name the project or workspace and the request goes there.

### How is this different from cmux?

cmux showed that agents deserve their own panes, status, and notifications. New Terminal builds on that idea with one prompt for intent, workspaces, an attention queue, diff review in the terminal, and an interface that lets agents drive the terminal directly.

### How is this different from Conductor?

Conductor showed that the workspace, not the agent, is the right unit of work, and I borrow its model: setup, run, and archive scripts, per-workspace ports, and a merge-readiness view. New Terminal differs in four ways: you direct all workspaces from one prompt instead of switching between them, it's a terminal first, it runs on macOS, Linux, and Windows, and agents can drive the terminal directly through an open interface.

### How is this different from Warp?

Warp is the closest thing to New Terminal and a big inspiration.

**What's the same:** output grouped into blocks, a real text editor for the input line, agents built into the terminal, support for third-party agents like Claude Code and Codex and for any model, open source, and macOS, Linux, and Windows.

**What's different:**

- **Intent only.** In Warp you type shell commands and can also ask agents. Here you only give intent, and agents do all of the work.
- **One prompt.** Warp gives each tab and pane its own input. Here there's exactly one prompt for everything.
- **Mention instead of navigate.** In Warp you go to the tab or pane where a project lives. Here you mention the project and the request goes there.
- **The keyboard never leaves.** Warp hands the keyboard to full-screen programs like vim. Here it never leaves: agents drive programs while you watch, and editors stay in their own app.
- **A terminal, not a platform.** Warp spans a terminal, an IDE, the web, and automation across the development lifecycle. This is one thing: the place where you direct agents across every project.

### How is this different from other AI terminals?

Mostly the same as for Warp: intent only, one prompt, mentions instead of navigation, and a keyboard that never leaves. It also works with any agent instead of favoring its own, and no account is required to use it.

## Speed

### How fast is it?

Speed is a core promise: everything responds instantly and works from the keyboard, and a slowdown is a bug. See [Fast and interactive](VISION.md#2-fast-and-interactive).

## Configuration

### Can I customize it?

Some. One plain text file covers what people legitimately need to change: font, size, theme, keybindings, and your default agent. Beyond that I choose for you, the way an omakase chef does. If you want to tune every pixel, other terminals do that well.

### Does it support plugins?

Yes. Extensions run on their own and use the same interface agents use. A crashing extension can't take down the app.

## Open source and paid features

### Is it open source?

Yes, under the [Apache 2.0 license](LICENSE). That includes the whole local experience: the app itself, agent hosting, the attention queue, workspaces, diff review, and the built-in agent.

### What costs money?

Features that cost me money to run or that only make sense for teams: settings sync, cloud agents, remote access to your sessions, team workspaces, session sharing, and admin controls. The test: if a solo developer on a plane would miss it, it's free.

### Do I need an account?

No. An account is only needed for paid features.

### Can someone fork it?

Yes, under the terms of Apache 2.0. The name and logo are covered by a trademark policy, so forks need their own branding.
