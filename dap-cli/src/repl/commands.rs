use super::breakpoint::{parse_break_location, BreakpointOptions};
use super::context::FunctionBreakpointAction;

/// Parsed interactive debugger command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplCommand {
    Help { topic: Option<String> },
    Version,
    Quit,
    Continue,
    StepOver,
    StepIn,
    StepOut,
    Threads,
    Stack { full: bool },
    Status,
    Sync,
    Capabilities,
    Show { context: Option<u32> },
    Break {
        path: String,
        line: i64,
        options: BreakpointOptions,
    },
    Trace {
        path: String,
        line: i64,
        message: String,
    },
    Clear {
        path: String,
        line: Option<i64>,
    },
    SkipList,
    SkipAdd { pattern: String },
    SkipClear { pattern: Option<String> },
    Eval { expression: String, pretty: bool },
    Scopes,
    Locals,
    Frame(i64),
    Thread(i64),
    InfoBreak,
    InfoCatch,
    SetVariable { name: String, value: String },
    Catch {
        filter: Option<String>,
        condition: Option<String>,
        clear: bool,
    },
    WatchList,
    WatchAdd { expression: String },
    WatchRemove { expression: String },
    SmartStep { enable: Option<bool> },
    Goto { path: String, line: i64 },
    BreakFunction {
        name: String,
        action: FunctionBreakpointAction,
    },
    DataWatchList,
    DataWatchAdd { expression: String },
    DataWatchRemove { expression: String },
    RestartFrame { frame_id: Option<i64> },
    Disassemble {
        memory_reference: String,
        offset: i64,
        count: i64,
    },
    Colors { enable: Option<bool> },
    Empty,
    Unknown(String),
}

pub fn parse_line(line: &str) -> ReplCommand {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return ReplCommand::Empty;
    }

    let (command, rest) = split_command(trimmed);
    let cmd = command.to_ascii_lowercase();

    match cmd.as_str() {
        "help" | "h" | "?" => ReplCommand::Help {
            topic: parse_help_topic(rest),
        },
        "version" | "ver" => ReplCommand::Version,
        "quit" | "q" | "exit" => ReplCommand::Quit,
        "continue" | "cont" | "c" | "run" => ReplCommand::Continue,
        "next" | "n" | "step-over" | "stepover" => ReplCommand::StepOver,
        "step" | "s" | "step-in" | "stepin" => ReplCommand::StepIn,
        "finish" | "step-out" | "stepout" => ReplCommand::StepOut,
        "threads" => ReplCommand::Threads,
        "info" if rest == "threads" => ReplCommand::Threads,
        "info" if rest == "scopes" => ReplCommand::Scopes,
        "info" if rest == "locals" => ReplCommand::Locals,
        "info" if rest == "break" || rest == "breakpoints" => ReplCommand::InfoBreak,
        "info" if rest == "catch" => ReplCommand::InfoCatch,
        "info" => ReplCommand::Unknown(format!("unknown info subcommand: {rest}")),
        "bt" | "backtrace" | "stack" | "where" => ReplCommand::Stack {
            full: rest.trim().eq_ignore_ascii_case("full"),
        },
        "status" => ReplCommand::Status,
        "sync" => ReplCommand::Sync,
        "capabilities" | "caps" => ReplCommand::Capabilities,
        "show" | "src" | "code" => parse_show(rest),
        "break" | "b" => {
            if let Some(name) = rest.strip_prefix("function ") {
                return parse_break_function(name);
            }
            match parse_break_location(rest) {
                Ok((path, line, options)) => ReplCommand::Break { path, line, options },
                Err(message) => ReplCommand::Unknown(message),
            }
        }
        "trace" | "tbreak" => parse_trace(rest),
        "clear" => parse_clear(rest),
        "skip" => parse_skip(rest),
        "print" | "p" | "eval" | "expr" => parse_eval(rest, false),
        "pp" => parse_eval(rest, true),
        "scopes" => ReplCommand::Scopes,
        "locals" => ReplCommand::Locals,
        "watch" if rest.is_empty() => ReplCommand::WatchList,
        "watch" => parse_watch_command(rest),
        "unwatch" => parse_unwatch_command(rest),
        "smart-step" | "smartstep" => ReplCommand::SmartStep {
            enable: parse_optional_bool(rest),
        },
        "goto" | "jump" => parse_goto(rest),
        "data-watch" | "dwatch" => parse_data_watch(rest),
        "restart" => parse_restart(rest),
        "disasm" | "disassemble" => parse_disassemble(rest),
        "frame" | "f" => parse_frame(rest),
        "thread" | "t" => parse_thread(rest),
        "set" => parse_set(rest),
        "catch" => parse_catch(rest),
        "colors" | "color" | "syntax-color" | "syntax-colors" => parse_colors(rest),
        other => ReplCommand::Unknown(format!("unknown command: {other}")),
    }
}

fn parse_help_topic(rest: &str) -> Option<String> {
    let rest = rest.trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

fn parse_eval(rest: &str, pretty: bool) -> ReplCommand {
    if rest.is_empty() {
        ReplCommand::Unknown("expression required".into())
    } else {
        ReplCommand::Eval {
            expression: rest.to_string(),
            pretty,
        }
    }
}

fn parse_show(rest: &str) -> ReplCommand {
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        return ReplCommand::Show { context: None };
    }
    match trimmed.parse::<u32>() {
        Ok(lines) => ReplCommand::Show {
            context: Some(lines),
        },
        Err(_) => ReplCommand::Unknown("usage: show [context-lines]".into()),
    }
}

fn parse_trace(rest: &str) -> ReplCommand {
    match parse_break_location(rest) {
        Ok((path, line, options)) if options.log_message.is_some() => ReplCommand::Trace {
            path,
            line,
            message: options.log_message.unwrap(),
        },
        Ok((path, line, _)) => ReplCommand::Trace {
            path,
            line,
            message: "tracepoint".into(),
        },
        Err(_) => {
            if let Some((path, after_colon)) = rest.split_once(':') {
                if let Ok(line) = after_colon.trim().parse::<i64>() {
                    return ReplCommand::Trace {
                        path: path.to_string(),
                        line,
                        message: "tracepoint".into(),
                    };
                }
            }
            ReplCommand::Unknown("usage: trace <file>:<line> [log MESSAGE]".into())
        }
    }
}

fn parse_skip(rest: &str) -> ReplCommand {
    let rest = rest.trim();
    if rest.is_empty() || rest == "list" {
        return ReplCommand::SkipList;
    }
    if rest == "clear" {
        return ReplCommand::SkipClear { pattern: None };
    }
    if let Some(pattern) = rest.strip_prefix("add ") {
        return ReplCommand::SkipAdd {
            pattern: pattern.trim().to_string(),
        };
    }
    if let Some(pattern) = rest.strip_prefix("clear ") {
        return ReplCommand::SkipClear {
            pattern: Some(pattern.trim().to_string()),
        };
    }
    ReplCommand::SkipAdd {
        pattern: rest.to_string(),
    }
}

fn parse_clear(rest: &str) -> ReplCommand {
    let rest = rest.trim();
    if rest.is_empty() {
        return ReplCommand::Unknown(
            "usage: clear <file> or clear <file>:<line>".into(),
        );
    }

    if let Some((path, after_colon)) = rest.split_once(':') {
        match after_colon.trim().parse::<i64>() {
            Ok(line) => ReplCommand::Clear {
                path: path.to_string(),
                line: Some(line),
            },
            Err(_) => ReplCommand::Unknown("invalid line number in clear location".into()),
        }
    } else {
        ReplCommand::Clear {
            path: rest.to_string(),
            line: None,
        }
    }
}

fn parse_frame(rest: &str) -> ReplCommand {
    let rest = rest.trim();
    if rest.is_empty() {
        return ReplCommand::Unknown("frame requires a number".into());
    }
    match rest.parse::<i64>() {
        Ok(value) => ReplCommand::Frame(value),
        Err(_) => ReplCommand::Unknown(format!("invalid frame number: {rest}")),
    }
}

fn parse_thread(rest: &str) -> ReplCommand {
    let rest = rest.trim();
    if rest.is_empty() {
        return ReplCommand::Unknown("thread requires a number".into());
    }
    match rest.parse::<i64>() {
        Ok(value) => ReplCommand::Thread(value),
        Err(_) => ReplCommand::Unknown(format!("invalid thread id: {rest}")),
    }
}

fn parse_set(rest: &str) -> ReplCommand {
    let rest = rest.trim();
    let rest = rest.strip_prefix("variable ").unwrap_or(rest);
    if let Some((name, value)) = rest.split_once('=') {
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() {
            return ReplCommand::Unknown("set requires a variable name".into());
        }
        if value.is_empty() {
            return ReplCommand::Unknown("set requires a value".into());
        }
        return ReplCommand::SetVariable {
            name: name.to_string(),
            value: value.to_string(),
        };
    }
    ReplCommand::Unknown("usage: set <name> = <value>".into())
}

fn parse_goto(rest: &str) -> ReplCommand {
    match parse_break_location(rest) {
        Ok((path, line, _)) => ReplCommand::Goto { path, line },
        Err(message) => ReplCommand::Unknown(message),
    }
}

fn parse_break_function(rest: &str) -> ReplCommand {
    let rest = rest.trim();
    if rest.is_empty() {
        return ReplCommand::Unknown("usage: break function <name>".into());
    }
    if rest == "clear" {
        return ReplCommand::BreakFunction {
            name: String::new(),
            action: FunctionBreakpointAction::Clear,
        };
    }
    if let Some(name) = rest.strip_prefix("clear ") {
        return ReplCommand::BreakFunction {
            name: name.trim().to_string(),
            action: FunctionBreakpointAction::Remove,
        };
    }
    ReplCommand::BreakFunction {
        name: rest.to_string(),
        action: FunctionBreakpointAction::Add,
    }
}

fn parse_data_watch(rest: &str) -> ReplCommand {
    let rest = rest.trim();
    if rest.is_empty() || rest == "list" {
        return ReplCommand::DataWatchList;
    }
    if let Some(expression) = rest.strip_prefix("remove ") {
        return ReplCommand::DataWatchRemove {
            expression: expression.trim().to_string(),
        };
    }
    if let Some(expression) = rest.strip_prefix("add ") {
        return ReplCommand::DataWatchAdd {
            expression: expression.trim().to_string(),
        };
    }
    ReplCommand::DataWatchAdd {
        expression: rest.to_string(),
    }
}

fn parse_restart(rest: &str) -> ReplCommand {
    let rest = rest.trim();
    if rest.is_empty() {
        return ReplCommand::RestartFrame { frame_id: None };
    }
    let rest = rest.strip_prefix("frame ").unwrap_or(rest);
    match rest.parse::<i64>() {
        Ok(frame_id) => ReplCommand::RestartFrame {
            frame_id: Some(frame_id),
        },
        Err(_) => ReplCommand::Unknown("usage: restart [frame <id>]".into()),
    }
}

fn parse_disassemble(rest: &str) -> ReplCommand {
    let parts = rest.split_whitespace().collect::<Vec<_>>();
    if parts.is_empty() {
        return ReplCommand::Unknown("usage: disasm <memory-reference> [offset] [count]".into());
    }
    let memory_reference = parts[0].to_string();
    let offset = parts
        .get(1)
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let count = parts
        .get(2)
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(1);
    ReplCommand::Disassemble {
        memory_reference,
        offset,
        count,
    }
}

fn parse_colors(rest: &str) -> ReplCommand {
    match rest.trim().to_ascii_lowercase().as_str() {
        "" => ReplCommand::Colors { enable: None },
        "on" | "1" | "true" | "yes" => ReplCommand::Colors { enable: Some(true) },
        "off" | "0" | "false" | "no" => ReplCommand::Colors { enable: Some(false) },
        other => ReplCommand::Unknown(format!("usage: colors [on|off] (got '{other}')")),
    }
}

fn parse_catch(rest: &str) -> ReplCommand {
    let rest = rest.trim();
    if rest.is_empty() {
        return ReplCommand::Unknown("usage: catch <filter> [if EXPR] or catch clear [filter]".into());
    }
    if rest == "clear" {
        return ReplCommand::Catch {
            filter: None,
            condition: None,
            clear: true,
        };
    }
    if let Some(filter) = rest.strip_prefix("clear ") {
        let filter = filter.trim();
        if filter.is_empty() {
            return ReplCommand::Catch {
                filter: None,
                condition: None,
                clear: true,
            };
        }
        return ReplCommand::Catch {
            filter: Some(filter.to_string()),
            condition: None,
            clear: true,
        };
    }
    if let Some((filter, condition)) = rest.split_once(" if ") {
        let filter = filter.trim();
        let condition = condition.trim();
        if filter.is_empty() || condition.is_empty() {
            return ReplCommand::Unknown("usage: catch <filter> if EXPR".into());
        }
        return ReplCommand::Catch {
            filter: Some(filter.to_string()),
            condition: Some(condition.to_string()),
            clear: false,
        };
    }
    ReplCommand::Catch {
        filter: Some(rest.to_string()),
        condition: None,
        clear: false,
    }
}

fn parse_watch_command(rest: &str) -> ReplCommand {
    let expression = rest.trim();
    if expression.is_empty() {
        return ReplCommand::Unknown("usage: watch <expression>".into());
    }
    ReplCommand::WatchAdd {
        expression: expression.to_string(),
    }
}

fn parse_unwatch_command(rest: &str) -> ReplCommand {
    let expression = rest.trim();
    if expression.is_empty() {
        return ReplCommand::Unknown("usage: unwatch <expression>".into());
    }
    ReplCommand::WatchRemove {
        expression: expression.to_string(),
    }
}

fn parse_optional_bool(rest: &str) -> Option<bool> {
    match rest.trim().to_ascii_lowercase().as_str() {
        "" => None,
        "on" | "1" | "true" | "yes" => Some(true),
        "off" | "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

fn split_command(line: &str) -> (&str, &str) {
    let mut parts = line.splitn(2, char::is_whitespace);
    let command = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();
    (command, rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_break_with_modifiers() {
        let cmd = parse_line("break main.rs:10 if x > 5 hit 3 log entered");
        assert_eq!(
            cmd,
            ReplCommand::Break {
                path: "main.rs".into(),
                line: 10,
                options: BreakpointOptions {
                    condition: Some("x > 5".into()),
                    hit_condition: Some("3".into()),
                    log_message: Some("entered".into()),
                },
            }
        );
    }

    #[test]
    fn parses_trace_command() {
        let cmd = parse_line("trace main.rs:4 log loop={i}");
        assert!(matches!(cmd, ReplCommand::Trace { .. }));
    }

    #[test]
    fn parses_skip_commands() {
        assert_eq!(parse_line("skip"), ReplCommand::SkipList);
        assert_eq!(
            parse_line("skip add /vendor/"),
            ReplCommand::SkipAdd {
                pattern: "/vendor/".into()
            }
        );
    }

    #[test]
    fn parses_info_break_and_set_variable() {
        assert_eq!(parse_line("info break"), ReplCommand::InfoBreak);
        assert_eq!(parse_line("info breakpoints"), ReplCommand::InfoBreak);
        assert_eq!(
            parse_line("bt full"),
            ReplCommand::Stack { full: true }
        );
        assert_eq!(
            parse_line("pp x"),
            ReplCommand::Eval {
                expression: "x".into(),
                pretty: true,
            }
        );
        assert_eq!(
            parse_line("colors on"),
            ReplCommand::Colors { enable: Some(true) }
        );
        assert_eq!(parse_line("color"), ReplCommand::Colors { enable: None });
        assert_eq!(
            parse_line("set x = 42"),
            ReplCommand::SetVariable {
                name: "x".into(),
                value: "42".into(),
            }
        );
    }

    #[test]
    fn parses_catch_commands() {
        assert_eq!(
            parse_line("catch uncaught"),
            ReplCommand::Catch {
                filter: Some("uncaught".into()),
                condition: None,
                clear: false,
            }
        );
        assert_eq!(
            parse_line("catch raised if err != nil"),
            ReplCommand::Catch {
                filter: Some("raised".into()),
                condition: Some("err != nil".into()),
                clear: false,
            }
        );
        assert_eq!(
            parse_line("catch clear"),
            ReplCommand::Catch {
                filter: None,
                condition: None,
                clear: true,
            }
        );
        assert_eq!(parse_line("info catch"), ReplCommand::InfoCatch);
        assert_eq!(parse_line("show"), ReplCommand::Show { context: None });
        assert_eq!(
            parse_line("show 3"),
            ReplCommand::Show {
                context: Some(3),
            }
        );
    }

    #[test]
    fn parses_clear_and_unknown_commands() {
        assert_eq!(
            parse_line("clear main.rs:10"),
            ReplCommand::Clear {
                path: "main.rs".into(),
                line: Some(10),
            }
        );
        assert!(matches!(parse_line("info foo"), ReplCommand::Unknown(_)));
        assert!(matches!(parse_line("set bad"), ReplCommand::Unknown(_)));
    }
}
