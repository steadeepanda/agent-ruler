//! Runner command normalization and structured-output parsing.
//!
//! This module keeps runner-specific invocation details centralized so
//! `agent-ruler run -- ...` remains deterministic across runner adapters.

use std::path::Path;

use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredOutputKind {
    ClaudeJson,
    ClaudeStreamJson,
    OpenCodeJson,
}

impl StructuredOutputKind {
    pub fn runner_id(self) -> &'static str {
        match self {
            Self::ClaudeJson | Self::ClaudeStreamJson => "claudecode",
            Self::OpenCodeJson => "opencode",
        }
    }

    pub fn parser_label(self) -> &'static str {
        match self {
            Self::ClaudeJson => "claude-json",
            Self::ClaudeStreamJson => "claude-stream-json",
            Self::OpenCodeJson => "opencode-json",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredOutputSummary {
    pub runner_id: &'static str,
    pub parser: &'static str,
    pub payload_count: usize,
    pub tool_event_count: usize,
    pub approval_reference_count: usize,
    pub error_event_count: usize,
    pub parse_error: Option<String>,
}

pub fn normalize_runner_command(cmd: &[String]) -> Vec<String> {
    let Some(exec_index) = command_exec_index(cmd) else {
        return cmd.to_vec();
    };
    let executable = basename(&cmd[exec_index]);

    if executable.eq_ignore_ascii_case("claude") {
        if !claude_print_mode(&cmd[exec_index + 1..]) {
            return cmd.to_vec();
        }
        if claude_output_format(&cmd[exec_index + 1..]).is_some() {
            return cmd.to_vec();
        }
        let mut normalized = cmd.to_vec();
        normalized.push("--output-format".to_string());
        normalized.push("json".to_string());
        return normalized;
    }

    if executable.eq_ignore_ascii_case("opencode") {
        let after_exec = &cmd[exec_index + 1..];
        if !opencode_run_mode(after_exec) {
            return cmd.to_vec();
        }
        if opencode_output_format(after_exec).is_some() {
            return cmd.to_vec();
        }

        let mut normalized = Vec::with_capacity(cmd.len() + 2);
        normalized.extend_from_slice(&cmd[..exec_index + 1]);
        normalized.push("run".to_string());
        normalized.push("--format".to_string());
        normalized.push("json".to_string());
        normalized.extend_from_slice(&cmd[exec_index + 2..]);
        return normalized;
    }

    cmd.to_vec()
}

pub fn detect_structured_output_kind(cmd: &[String]) -> Option<StructuredOutputKind> {
    let exec_index = command_exec_index(cmd)?;
    let executable = basename(&cmd[exec_index]);
    if executable.eq_ignore_ascii_case("claude") {
        let format = claude_output_format(&cmd[exec_index + 1..])?;
        return match format.as_str() {
            "json" => Some(StructuredOutputKind::ClaudeJson),
            "stream-json" => Some(StructuredOutputKind::ClaudeStreamJson),
            _ => None,
        };
    }

    if executable.eq_ignore_ascii_case("opencode") {
        let after_exec = &cmd[exec_index + 1..];
        if !opencode_run_mode(after_exec) {
            return None;
        }
        let format = opencode_output_format(after_exec)?;
        if format == "json" {
            return Some(StructuredOutputKind::OpenCodeJson);
        }
    }

    None
}

pub fn summarize_structured_output(
    kind: StructuredOutputKind,
    stdout: &str,
    stderr: &str,
) -> StructuredOutputSummary {
    let parse_attempt = if !stdout.trim().is_empty() {
        stdout
    } else {
        stderr
    };

    let parse_result = parse_json_values(parse_attempt);
    match parse_result {
        Ok(values) => {
            let mut counters = StructuredOutputCounters::default();
            for value in &values {
                scan_json_value(value, &mut counters);
            }
            StructuredOutputSummary {
                runner_id: kind.runner_id(),
                parser: kind.parser_label(),
                payload_count: values.len(),
                tool_event_count: counters.tool_events,
                approval_reference_count: counters.approval_refs,
                error_event_count: counters.error_events,
                parse_error: None,
            }
        }
        Err(parse_error) => StructuredOutputSummary {
            runner_id: kind.runner_id(),
            parser: kind.parser_label(),
            payload_count: 0,
            tool_event_count: 0,
            approval_reference_count: 0,
            error_event_count: 0,
            parse_error: Some(parse_error),
        },
    }
}

#[derive(Default)]
struct StructuredOutputCounters {
    tool_events: usize,
    approval_refs: usize,
    error_events: usize,
}

fn parse_json_values(raw: &str) -> Result<Vec<JsonValue>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("structured output stream was empty".to_string());
    }

    if let Ok(value) = serde_json::from_str::<JsonValue>(trimmed) {
        return Ok(vec![value]);
    }

    let mut values = Vec::new();
    for (index, line) in trimmed.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<JsonValue>(line) {
            Ok(value) => values.push(value),
            Err(err) => {
                return Err(format!(
                    "failed to parse structured output line {}: {err}",
                    index + 1
                ));
            }
        }
    }

    if values.is_empty() {
        return Err("structured output did not contain JSON objects".to_string());
    }
    Ok(values)
}

fn scan_json_value(value: &JsonValue, counters: &mut StructuredOutputCounters) {
    match value {
        JsonValue::Object(map) => {
            let mut has_tool_event = false;
            if map.contains_key("tool")
                || map.contains_key("tool_name")
                || map.contains_key("tool_use")
                || map.contains_key("tool_call")
                || map.contains_key("tool_calls")
            {
                has_tool_event = true;
            }
            if map
                .get("type")
                .and_then(JsonValue::as_str)
                .map(|kind| kind.to_ascii_lowercase().contains("tool"))
                .unwrap_or(false)
            {
                has_tool_event = true;
            }
            if has_tool_event {
                counters.tool_events += 1;
            }

            if map.contains_key("approval_id") {
                counters.approval_refs += 1;
            }
            if map
                .get("is_error")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
            {
                counters.error_events += 1;
            }
            if map
                .get("error")
                .map(|error| !error.is_null() && error != &JsonValue::Bool(false))
                .unwrap_or(false)
            {
                counters.error_events += 1;
            }

            for child in map.values() {
                scan_json_value(child, counters);
            }
        }
        JsonValue::Array(items) => {
            for child in items {
                scan_json_value(child, counters);
            }
        }
        _ => {}
    }
}

fn command_exec_index(cmd: &[String]) -> Option<usize> {
    let first = cmd.first()?;
    if !basename(first).eq_ignore_ascii_case("env") {
        return Some(0);
    }

    let mut index = 1usize;
    while index < cmd.len() {
        let token = cmd[index].as_str();
        if token.contains('=') {
            index += 1;
            continue;
        }
        return Some(index);
    }
    None
}

fn basename(token: &str) -> String {
    Path::new(token)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(token)
        .to_string()
}

fn claude_print_mode(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "-p" || arg == "--print")
}

fn claude_output_format(args: &[String]) -> Option<String> {
    value_for_flag(args, "--output-format")
}

fn opencode_run_mode(args: &[String]) -> bool {
    args.first().map(|arg| arg == "run").unwrap_or(false)
}

fn opencode_output_format(args: &[String]) -> Option<String> {
    value_for_flag(args, "--format")
}

fn value_for_flag(args: &[String], flag: &str) -> Option<String> {
    for (index, token) in args.iter().enumerate() {
        if token == flag {
            if let Some(value) = args.get(index + 1) {
                return Some(value.trim().to_ascii_lowercase());
            }
            return None;
        }
        let prefix = format!("{flag}=");
        if let Some(value) = token.strip_prefix(&prefix) {
            return Some(value.trim().to_ascii_lowercase());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        detect_structured_output_kind, normalize_runner_command, summarize_structured_output,
        StructuredOutputKind,
    };

    #[test]
    fn normalize_claude_print_adds_json_output_when_missing() {
        let cmd = vec![
            "claude".to_string(),
            "-p".to_string(),
            "reply exactly ok".to_string(),
        ];
        let normalized = normalize_runner_command(&cmd);
        assert_eq!(
            normalized,
            vec![
                "claude".to_string(),
                "-p".to_string(),
                "reply exactly ok".to_string(),
                "--output-format".to_string(),
                "json".to_string(),
            ]
        );
    }

    #[test]
    fn normalize_claude_keeps_existing_output_format() {
        let cmd = vec![
            "claude".to_string(),
            "-p".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "reply".to_string(),
        ];
        let normalized = normalize_runner_command(&cmd);
        assert_eq!(normalized, cmd);
    }

    #[test]
    fn normalize_opencode_run_adds_json_format_when_missing() {
        let cmd = vec![
            "opencode".to_string(),
            "run".to_string(),
            "Reply with exactly: OK".to_string(),
        ];
        let normalized = normalize_runner_command(&cmd);
        assert_eq!(
            normalized,
            vec![
                "opencode".to_string(),
                "run".to_string(),
                "--format".to_string(),
                "json".to_string(),
                "Reply with exactly: OK".to_string(),
            ]
        );
    }

    #[test]
    fn normalize_openclaw_passthrough_keeps_full_tail() {
        let cmd = vec![
            "openclaw".to_string(),
            "sessions".to_string(),
            "cleanup".to_string(),
            "alpha".to_string(),
            "--dry-run".to_string(),
            "--".to_string(),
            "--all".to_string(),
        ];
        let normalized = normalize_runner_command(&cmd);
        assert_eq!(normalized, cmd);
    }

    #[test]
    fn detect_structured_output_kind_matches_runner_flags() {
        let claude = vec![
            "claude".to_string(),
            "-p".to_string(),
            "--output-format".to_string(),
            "json".to_string(),
            "hello".to_string(),
        ];
        assert_eq!(
            detect_structured_output_kind(&claude),
            Some(StructuredOutputKind::ClaudeJson)
        );

        let opencode = vec![
            "opencode".to_string(),
            "run".to_string(),
            "--format".to_string(),
            "json".to_string(),
            "hello".to_string(),
        ];
        assert_eq!(
            detect_structured_output_kind(&opencode),
            Some(StructuredOutputKind::OpenCodeJson)
        );
    }

    #[test]
    fn summarize_structured_output_counts_tool_and_approval_events() {
        let stdout = r#"{"type":"tool_use","tool_name":"write","approval_id":"abc"}
{"type":"message","error":{"message":"x"}}"#;
        let summary =
            summarize_structured_output(StructuredOutputKind::ClaudeStreamJson, stdout, "");
        assert_eq!(summary.runner_id, "claudecode");
        assert_eq!(summary.payload_count, 2);
        assert_eq!(summary.tool_event_count, 1);
        assert_eq!(summary.approval_reference_count, 1);
        assert_eq!(summary.error_event_count, 1);
        assert!(summary.parse_error.is_none());
    }

    #[test]
    fn summarize_structured_output_reports_parse_error() {
        let summary =
            summarize_structured_output(StructuredOutputKind::OpenCodeJson, "not-json", "");
        assert_eq!(summary.payload_count, 0);
        assert!(summary.parse_error.is_some());
    }
}
