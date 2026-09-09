use super::context::BreakpointAction;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BreakpointOptions {
    pub condition: Option<String>,
    pub hit_condition: Option<String>,
    pub log_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakpointRequest {
    pub path: String,
    pub line: i64,
    pub action: BreakpointAction,
    pub options: BreakpointOptions,
}

pub fn parse_break_location(rest: &str) -> Result<(String, i64, BreakpointOptions), String> {
    if rest.is_empty() {
        return Err(
            "usage: break <file>:<line> [if EXPR] [hit N] [log MSG]".into(),
        );
    }

    if let Some((path, after_colon)) = rest.split_once(':') {
        let (line, options) = parse_line_and_options(after_colon)?;
        return Ok((path.to_string(), line, options));
    }

    let parts = rest.split_whitespace().collect::<Vec<_>>();
    match parts.len() {
        0 => Err("line number required".into()),
        1 => Err("line number required".into()),
        2 => {
            let line = parts[1]
                .parse::<i64>()
                .map_err(|_| "invalid line number".to_string())?;
            Ok((parts[0].to_string(), line, BreakpointOptions::default()))
        }
        _ => {
            let line = parts[1]
                .parse::<i64>()
                .map_err(|_| "invalid line number".to_string())?;
            let options = parse_options_text(&parts[2..].join(" "))?;
            Ok((parts[0].to_string(), line, options))
        }
    }
}

fn parse_line_and_options(rest: &str) -> Result<(i64, BreakpointOptions), String> {
    let rest = rest.trim();
    if rest.is_empty() {
        return Err("line number required".into());
    }

    if let Some((line_str, options_text)) = rest.split_once(char::is_whitespace) {
        let line = line_str
            .parse::<i64>()
            .map_err(|_| "invalid line number in break location".to_string())?;
        let options = parse_options_text(options_text)?;
        Ok((line, options))
    } else {
        rest.parse::<i64>()
            .map(|line| (line, BreakpointOptions::default()))
            .map_err(|_| "invalid line number in break location".to_string())
    }
}

pub fn parse_options_text(text: &str) -> Result<BreakpointOptions, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(BreakpointOptions::default());
    }

    let mut options = BreakpointOptions::default();
    let mut remaining = text.to_string();

    while !remaining.is_empty() {
        let rest = remaining.trim_start();
        if let Some(tail) = rest.strip_prefix("if ") {
            let (value, tail) = take_until_keyword(tail, &[" hit ", " log "]);
            options.condition = Some(value.trim().to_string());
            remaining = tail;
            continue;
        }
        if let Some(tail) = rest.strip_prefix("hit ") {
            let (value, tail) = take_until_keyword(tail, &[" if ", " log "]);
            options.hit_condition = Some(value.trim().to_string());
            remaining = tail;
            continue;
        }
        if let Some(tail) = rest.strip_prefix("log ") {
            options.log_message = Some(tail.trim().to_string());
            remaining = String::new();
            continue;
        }

        if options.condition.is_none()
            && options.hit_condition.is_none()
            && options.log_message.is_none()
        {
            options.log_message = Some(rest.to_string());
        }
        break;
    }

    Ok(options)
}

fn take_until_keyword(text: &str, keywords: &[&str]) -> (String, String) {
    let mut earliest: Option<usize> = None;
    for keyword in keywords {
        if let Some(index) = text.find(keyword) {
            if earliest.map(|pos| index < pos).unwrap_or(true) {
                earliest = Some(index);
            }
        }
    }

    match earliest {
        Some(index) => (
            text[..index].to_string(),
            text[index..].trim_start().to_string(),
        ),
        None => (text.to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_condition_hit_and_log() {
        let (_, line, options) =
            parse_break_location("main.rs:10 if x > 5 hit 3 log value={x}").unwrap();
        assert_eq!(line, 10);
        assert_eq!(options.condition.as_deref(), Some("x > 5"));
        assert_eq!(options.hit_condition.as_deref(), Some("3"));
        assert_eq!(options.log_message.as_deref(), Some("value={x}"));
    }

    #[test]
    fn parses_log_only_option() {
        let options = parse_options_text("log entered function").unwrap();
        assert_eq!(options.log_message.as_deref(), Some("entered function"));
    }

    #[test]
    fn rejects_invalid_break_location() {
        assert!(parse_break_location("main.rs").is_err());
        assert!(parse_break_location("main.rs:not-a-line").is_err());
    }

    #[test]
    fn take_until_keyword_splits_on_first_match() {
        let (left, right) = take_until_keyword("x > 0 hit 3", &["hit", "log"]);
        assert_eq!(left, "x > 0 ");
        assert_eq!(right, "hit 3");
    }
}
