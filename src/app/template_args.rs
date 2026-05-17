use std::collections::HashMap;

pub(super) fn parse_template_body(args: &str) -> Option<&str> {
    let body = args.trim();
    (!body.is_empty()).then_some(body)
}

pub(super) fn parse_template_find_query(args: &str) -> Option<&str> {
    let query = args.trim_start().strip_prefix("find")?.trim_start();
    (!query.is_empty()).then_some(query)
}

fn parse_template_id_and_body(args: &str) -> Option<(&str, &str)> {
    let args = args.trim();
    let split_at = args.find(char::is_whitespace)?;
    let (id, body) = args.split_at(split_at);
    let body = body.trim_start();
    (!id.is_empty() && !body.is_empty()).then_some((id, body))
}

pub(super) fn parse_template_subcommand_args(args: &str) -> Option<(&str, &str)> {
    let rest = args.trim_start().strip_prefix("replace")?.trim_start();
    parse_template_id_and_body(rest)
}

pub(super) fn parse_template_values(args: &str) -> HashMap<String, String> {
    let tokens = split_template_tokens(args);
    let mut parts = tokens.iter().map(String::as_str);
    let _subcommand = parts.next();
    let _name = parts.next();

    parts
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            (!key.is_empty()).then_some((key.to_string(), trim_matching_quotes(value).to_string()))
        })
        .collect()
}

fn split_template_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for ch in input.chars() {
        match quote {
            Some(active) if ch == active => {
                quote = None;
                current.push(ch);
            }
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                current.push(ch);
            }
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn trim_matching_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let first = bytes[0];
        let last = bytes[value.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return &value[1..value.len() - 1];
        }
    }
    value
}

pub(super) fn template_usage() -> &'static str {
    "usage: #template find <query> | #template show <id> | #template use <id> [key=value...] | #template rm <id> | #template replace <id> <body>"
}
