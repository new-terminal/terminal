//! Maps one stdout line from `claude` in stream-json mode to the lines the
//! author sees and the effect on the agent's state. A pure function: the
//! actor applies the effect.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use serde_json::Value;

use crate::LineKind;
use crate::block;
use crate::log::rfc3339_millis;

const TOOL_LINE_CHARS: usize = 200;
const SUMMARY_CHARS: usize = 80;
const PREVIEW_CHARS: usize = 200;
const MS_PER_SECOND: u64 = 1000;

/// What the mapping needs to know about the agent.
#[derive(Debug)]
pub struct Context<'a> {
    pub target: &'a str,
    /// Paths inside it show relative to it.
    pub path: &'a Path,
    /// The app sent an interrupt, so the turn's `result` is expected and
    /// hidden.
    pub interrupted: bool,
    /// The process already sent an `init`. The CLI sends one per turn, and
    /// only the first one shows.
    pub init_seen: bool,
    /// `tool_use_id`s whose error result stays in the agent log only.
    pub hidden_results: &'a BTreeSet<String>,
}

/// One line to show, labeled with the agent's target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub kind: LineKind,
    pub text: String,
}

/// A `can_use_tool` request. The agent waits until stdin answers it.
#[derive(Clone, Debug)]
pub struct PermissionRequest {
    pub request_id: String,
    pub tool: String,
    /// Ties the request to the `tool_result` the agent gets for it.
    pub tool_use_id: Option<String>,
    /// As received. An allow must send it back unchanged.
    pub input: Value,
    /// `Allow <tool>: <detail> (<n> lines)`, for the attention line.
    pub summary: String,
}

#[derive(Clone, Debug)]
pub enum Effect {
    Init {
        cwd: String,
        permission_mode: String,
        session_id: String,
    },
    /// The agent wrote text. `last_text` is its last text block.
    Working {
        last_text: String,
    },
    Permission(PermissionRequest),
    TurnDone,
    TurnFailed,
    /// The `result` that ends a turn the app interrupted.
    InterruptedResult,
}

#[derive(Clone, Debug, Default)]
pub struct Mapped {
    pub shown: Vec<Shown>,
    pub effect: Option<Effect>,
}

impl Mapped {
    fn line(kind: LineKind, text: String) -> Self {
        Self {
            shown: vec![Shown { kind, text }],
            effect: None,
        }
    }

    fn with(mut self, effect: Effect) -> Self {
        self.effect = Some(effect);
        self
    }
}

pub fn map(line: &str, cx: &Context<'_>) -> Mapped {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return Mapped::line(
            LineKind::Warning,
            format!("Unreadable output: {}", cut(line, PREVIEW_CHARS)),
        );
    };
    match message["type"].as_str() {
        Some("system") => system(&message, cx),
        Some("assistant") => assistant(&message, cx),
        Some("user") => user(&message, cx),
        Some("control_request") => control_request(&message, cx),
        Some("rate_limit_event") => rate_limit(&message),
        Some("result") => result(&message, cx),
        _ => Mapped::default(),
    }
}

fn system(message: &Value, cx: &Context<'_>) -> Mapped {
    match message["subtype"].as_str() {
        Some("init") => {
            let cwd = text_field(&message["cwd"]);
            let shown = if cx.init_seen {
                Mapped::default()
            } else {
                Mapped::line(LineKind::App, format!("agent started in {cwd}"))
            };
            shown.with(Effect::Init {
                cwd,
                permission_mode: text_field(&message["permissionMode"]),
                session_id: text_field(&message["session_id"]),
            })
        }
        Some("permission_denied") => Mapped::line(LineKind::Error, text_field(&message["message"])),
        _ => Mapped::default(),
    }
}

fn assistant(message: &Value, cx: &Context<'_>) -> Mapped {
    let mut mapped = Mapped::default();
    for block in blocks(&message["message"]["content"]) {
        match block["type"].as_str() {
            Some("text") => {
                let text = block["text"].as_str().unwrap_or_default();
                mapped.shown.extend(text.lines().map(|line| Shown {
                    kind: LineKind::AgentText,
                    text: line.to_owned(),
                }));
                mapped.effect = Some(Effect::Working {
                    last_text: text.to_owned(),
                });
            }
            Some("tool_use") => mapped.shown.push(Shown {
                kind: LineKind::Tool,
                text: tool_line(block, cx.path),
            }),
            _ => {}
        }
    }
    mapped
}

fn user(message: &Value, cx: &Context<'_>) -> Mapped {
    let mut mapped = Mapped::default();
    for block in blocks(&message["message"]["content"]) {
        let hidden = block["tool_use_id"]
            .as_str()
            .is_some_and(|id| cx.hidden_results.contains(id));
        if block["type"] == "tool_result" && block["is_error"] == true && !hidden {
            let content = match &block["content"] {
                Value::String(text) => text.clone(),
                content => blocks(content)
                    .find_map(|part| part["text"].as_str())
                    .unwrap_or_default()
                    .to_owned(),
            };
            mapped.shown.push(Shown {
                kind: LineKind::Error,
                text: first_line(&content).to_owned(),
            });
        }
    }
    mapped
}

fn control_request(message: &Value, cx: &Context<'_>) -> Mapped {
    let request = &message["request"];
    let subtype = request["subtype"].as_str().unwrap_or("unnamed");
    if subtype != "can_use_tool" {
        return Mapped::line(
            LineKind::Warning,
            format!(
                "{} sent a {subtype} request New Terminal does not answer. Press ⌘. to stop it.",
                cx.target
            ),
        );
    }
    let (Some(request_id), Some(tool)) = (
        message["request_id"].as_str(),
        request["tool_name"].as_str(),
    ) else {
        return Mapped::line(
            LineKind::Warning,
            format!(
                "{} sent a request New Terminal cannot read. Press ⌘. to stop it.",
                cx.target
            ),
        );
    };
    let input = request["input"].clone();
    Mapped::default().with(Effect::Permission(PermissionRequest {
        request_id: request_id.to_owned(),
        tool: tool.to_owned(),
        tool_use_id: request["tool_use_id"].as_str().map(str::to_owned),
        summary: summary(tool, &input, cx.path),
        input,
    }))
}

fn rate_limit(message: &Value) -> Mapped {
    let info = &message["rate_limit_info"];
    let status = info["status"].as_str().unwrap_or("unknown");
    if status == "allowed" {
        return Mapped::default();
    }
    let text = info["resetsAt"].as_u64().map_or_else(
        || format!("Rate limit: {status}."),
        |seconds| {
            let resets = rfc3339_millis(UNIX_EPOCH + Duration::from_secs(seconds));
            format!("Rate limit: {status}. It resets at {resets}.")
        },
    );
    Mapped::line(LineKind::App, text)
}

fn result(message: &Value, cx: &Context<'_>) -> Mapped {
    if cx.interrupted {
        return Mapped::default().with(Effect::InterruptedResult);
    }
    let succeeded = message["subtype"] == "success" && message["is_error"] != true;
    if succeeded {
        let text = message["duration_ms"].as_u64().map_or_else(
            || "done".to_owned(),
            |ms| format!("done ({} s)", (ms + MS_PER_SECOND / 2) / MS_PER_SECOND),
        );
        return Mapped::line(LineKind::Done, text).with(Effect::TurnDone);
    }
    let text = message["result"]
        .as_str()
        .or_else(|| message["subtype"].as_str())
        .unwrap_or("the turn ended with an error")
        .to_owned();
    Mapped::line(LineKind::Failed, text).with(Effect::TurnFailed)
}

/// `Allow <tool>: <detail>`, cut to 80 characters, then the line count of
/// what the request would run or write. The detail escapes characters that
/// could hide or reorder the answer keys shown after it.
pub fn summary(tool: &str, input: &Value, target: &Path) -> String {
    let text = |field: &str| input[field].as_str().unwrap_or_default();
    let (detail, lines) = match tool {
        "Bash" => (
            first_line(text("command")).to_owned(),
            line_count(text("command")),
        ),
        "Write" => (
            relative(text("file_path"), target),
            line_count(text("content")),
        ),
        "Edit" => (
            relative(text("file_path"), target),
            line_count(text("old_string")) + line_count(text("new_string")),
        ),
        _ => (
            input.to_string(),
            serde_json::to_string_pretty(input).map_or(1, |pretty| line_count(&pretty)),
        ),
    };
    let unit = if lines == 1 { "line" } else { "lines" };
    format!(
        "{} ({lines} {unit})",
        cut(
            &format!("Allow {tool}: {}", block::escape(&detail)),
            SUMMARY_CHARS
        )
    )
}

/// The tool name and its main argument, cut to 200 characters.
fn tool_line(block: &Value, target: &Path) -> String {
    let input = &block["input"];
    let name = block["name"].as_str().unwrap_or("tool");
    let detail = if let Some(command) = input["command"].as_str() {
        let more = if command.lines().nth(1).is_some() {
            " …"
        } else {
            ""
        };
        format!("{}{more}", first_line(command))
    } else if let Some(path) = input["file_path"].as_str() {
        relative(path, target)
    } else {
        ["pattern", "url"]
            .iter()
            .find_map(|field| input[*field].as_str())
            .unwrap_or_default()
            .to_owned()
    };
    cut(format!("{name} {detail}").trim_end(), TOOL_LINE_CHARS)
}

fn blocks(content: &Value) -> impl Iterator<Item = &Value> {
    content.as_array().into_iter().flatten()
}

fn text_field(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

fn line_count(text: &str) -> usize {
    text.lines().count()
}

/// `path` relative to `target` when it lies inside it.
pub fn relative(path: &str, target: &Path) -> String {
    Path::new(path)
        .strip_prefix(target)
        .map_or_else(|_| path.to_owned(), |inside| inside.display().to_string())
}

/// At most `max` characters, ending in `…` when cut.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut kept: String = text.chars().take(max - 1).collect();
    kept.push('…');
    kept
}
