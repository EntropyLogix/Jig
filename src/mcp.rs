use crate::commands::{self, ArgSpec, ArgType, CommandOutput, McpToolSpec};
use crate::instructions;
use serde_json::{Map, Value, json};
use std::io::{self, BufRead, Write};
use std::path::Path;

const PROTOCOL_VERSION: &str = "2025-06-18";

pub fn serve(cwd: &Path) -> Result<(), String> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        let count = input
            .read_until(b'\n', &mut bytes)
            .map_err(|error| format!("read MCP stdin: {error}"))?;
        if count == 0 {
            break;
        }
        while bytes
            .last()
            .is_some_and(|byte| *byte == b'\n' || *byte == b'\r')
        {
            bytes.pop();
        }
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let response = match std::str::from_utf8(&bytes) {
            Ok(line) => match serde_json::from_str::<Value>(line) {
                Ok(message) => handle_message(&message, cwd),
                Err(_) => Some(error_response(Value::Null, -32700, "Parse error")),
            },
            Err(_) => Some(error_response(Value::Null, -32700, "Parse error")),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut stdout, &response)
                .map_err(|error| format!("write MCP stdout: {error}"))?;
            stdout
                .write_all(b"\n")
                .and_then(|()| stdout.flush())
                .map_err(|error| format!("write MCP stdout: {error}"))?;
        }
    }
    Ok(())
}

fn handle_message(message: &Value, cwd: &Path) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(error_response(Value::Null, -32600, "Invalid Request"));
    };
    let id = object.get("id").cloned();
    if id
        .as_ref()
        .is_some_and(|id| !id.is_string() && !id.is_number())
    {
        return Some(error_response(Value::Null, -32600, "Invalid Request"));
    }
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(error_response(
            id.unwrap_or(Value::Null),
            -32600,
            "Invalid Request",
        ));
    }
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return id.map(|id| error_response(id, -32600, "Invalid Request"));
    };
    let id = id?;
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => Some(initialize_response(id)),
        "ping" => Some(success_response(id, json!({}))),
        "tools/list" => Some(success_response(id, json!({"tools": tool_list()}))),
        "tools/call" => Some(tool_call_response(id, &params, cwd)),
        _ => Some(error_response(id, -32601, "Method not found")),
    }
}

fn initialize_response(id: Value) -> Value {
    let instructions = format!(
        "{}\n\n{}",
        instructions::WORKFLOW.trim_end_matches('\n'),
        instructions::MCP_INSTRUCTIONS
    );
    success_response(
        id,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "serverInfo": {"name":"jig","version":env!("CARGO_PKG_VERSION")},
            "capabilities": {"tools": {}},
            "instructions": instructions
        }),
    )
}

fn tool_list() -> Vec<Value> {
    commands::mcp_tools()
        .iter()
        .map(|tool| {
            let mut result = Map::new();
            result.insert("name".into(), Value::String(tool.name.to_string()));
            let description = if tool.command.decision {
                format!("A person's decision: {}", tool.description)
            } else {
                tool.description.to_string()
            };
            result.insert("description".into(), Value::String(description));
            result.insert("inputSchema".into(), input_schema(tool));
            if commands::JSON_CMDS.contains(&tool.command.name) {
                result.insert("annotations".into(), json!({"readOnlyHint":true}));
            }
            Value::Object(result)
        })
        .collect()
}

fn input_schema(tool: &McpToolSpec) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for arg in tool.command.args {
        if !arg.mcp || (tool.action.is_some() && arg.name == "action") {
            continue;
        }
        properties.insert(arg.name.to_string(), schema_for(arg.kind));
        if arg.required {
            required.push(Value::String(arg.name.to_string()));
        }
    }
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

fn schema_for(kind: ArgType) -> Value {
    match kind {
        ArgType::Text(_) => json!({"type":"string"}),
        ArgType::Boolean => json!({"type":"boolean"}),
        ArgType::Integer => json!({"type":"integer","minimum":0,"maximum":4}),
        ArgType::TextList(_) => json!({"type":"array","items":{"type":"string"}}),
        ArgType::Enum(values) => json!({"type":"string","enum":values}),
    }
}

fn tool_call_response(id: Value, params: &Value, cwd: &Path) -> Value {
    let object = match params {
        Value::Object(object) => object,
        _ => return error_response(id, -32602, "Invalid params: expected an object"),
    };
    let Some(name) = object.get("name").and_then(Value::as_str) else {
        return error_response(
            id,
            -32602,
            "Invalid params: missing required argument \"name\"",
        );
    };
    let tools = commands::mcp_tools();
    let Some(tool) = tools.iter().find(|tool| tool.name == name) else {
        return error_response(id, -32602, &format!("Unknown tool \"{name}\""));
    };
    let arguments = match object.get("arguments") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(arguments)) => arguments.clone(),
        Some(_) => {
            return error_response(
                id,
                -32602,
                "Invalid params: \"arguments\" must be an object",
            );
        }
    };
    if let Err(message) = validate_arguments(tool, &arguments) {
        return error_response(id, -32602, &message);
    }
    let call = match build_args(tool, &arguments) {
        Ok(call) => call,
        Err(message) => return error_response(id, -32602, &message),
    };
    match commands::run_mcp(&call, cwd) {
        Ok(output) => success_response(id, tool_result(output)),
        Err(error) => success_response(
            id,
            json!({"content":[{"type":"text","text":error.msg}],"isError":true}),
        ),
    }
}

fn validate_arguments(tool: &McpToolSpec, arguments: &Map<String, Value>) -> Result<(), String> {
    let specs: Vec<&ArgSpec> = tool
        .command
        .args
        .iter()
        .filter(|arg| arg.mcp && !(tool.action.is_some() && arg.name == "action"))
        .collect();
    for key in arguments.keys() {
        if !specs.iter().any(|arg| arg.name == key) {
            return Err(format!("unknown argument \"{key}\""));
        }
    }
    for arg in specs {
        let Some(value) = arguments.get(arg.name) else {
            if arg.required {
                return Err(format!("missing required argument \"{}\"", arg.name));
            }
            continue;
        };
        validate_value(arg, value)?;
    }
    Ok(())
}

fn validate_value(arg: &ArgSpec, value: &Value) -> Result<(), String> {
    let valid = match arg.kind {
        ArgType::Text(_) => value.is_string(),
        ArgType::Boolean => value.is_boolean(),
        ArgType::Integer => value
            .as_i64()
            .is_some_and(|number| (0..=4).contains(&number)),
        ArgType::TextList(_) => value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string)),
        ArgType::Enum(values) => value.as_str().is_some_and(|text| values.contains(&text)),
    };
    if valid {
        return Ok(());
    }
    let reason = match arg.kind {
        ArgType::Text(_) => "must be a string",
        ArgType::Boolean => "must be a boolean",
        ArgType::Integer => "must be an integer 0–4",
        ArgType::TextList(_) => "must be an array of strings",
        ArgType::Enum(_) => "has an invalid value",
    };
    Err(format!("\"{}\" {reason}", arg.name))
}

fn build_args(tool: &McpToolSpec, arguments: &Map<String, Value>) -> Result<Vec<String>, String> {
    let mut options = Vec::new();
    let mut positional = Vec::new();
    let mut out = Vec::new();
    if commands::JSON_CMDS.contains(&tool.command.name) {
        out.push("--json".to_string());
    }
    out.push(tool.command.name.to_string());
    if let Some(action) = tool.action {
        out.push(action.to_string());
    }
    for arg in tool.command.args {
        if !arg.mcp || (tool.action.is_some() && arg.name == "action") {
            continue;
        }
        let Some(value) = arguments.get(arg.name) else {
            continue;
        };
        if arg.positional {
            positional.extend(value_strings(arg, value)?);
        } else if value_is_true(arg, value)? {
            options.push(
                arg.cli
                    .ok_or_else(|| format!("argument \"{}\" has no CLI option", arg.name))?
                    .to_string(),
            );
        } else if let Some(cli) = arg.cli {
            for value in value_strings(arg, value)? {
                options.push(cli.to_string());
                options.push(value);
            }
        }
    }
    out.extend(options);
    if !positional.is_empty() {
        out.push("--".to_string());
        out.extend(positional);
    }
    Ok(out)
}

fn value_is_true(arg: &ArgSpec, value: &Value) -> Result<bool, String> {
    if matches!(arg.kind, ArgType::Boolean) {
        return value
            .as_bool()
            .ok_or_else(|| format!("\"{}\" must be a boolean", arg.name));
    }
    Ok(false)
}

fn value_strings(arg: &ArgSpec, value: &Value) -> Result<Vec<String>, String> {
    match arg.kind {
        ArgType::Text(_) | ArgType::Enum(_) => Ok(vec![
            value
                .as_str()
                .ok_or_else(|| format!("\"{}\" must be a string", arg.name))?
                .to_string(),
        ]),
        ArgType::Integer => Ok(vec![
            value
                .as_i64()
                .ok_or_else(|| format!("\"{}\" must be an integer", arg.name))?
                .to_string(),
        ]),
        ArgType::TextList(_) => value
            .as_array()
            .ok_or_else(|| format!("\"{}\" must be an array", arg.name))?
            .iter()
            .map(|item| {
                item.as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| format!("\"{}\" must contain strings", arg.name))
            })
            .collect(),
        ArgType::Boolean => Ok(Vec::new()),
    }
}

fn tool_result(output: CommandOutput) -> Value {
    let mut text = output.stdout;
    if !output.warnings.is_empty() {
        if !text.is_empty() {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            text.push('\n');
        }
        for warning in output.warnings {
            text.push_str("jig: warning: ");
            text.push_str(&warning);
            text.push('\n');
        }
    }
    json!({"content":[{"type":"text","text":text}],"isError":false})
}

fn success_response(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
