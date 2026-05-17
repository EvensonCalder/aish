use std::io::Write;

use anyhow::Result;

use crate::config::{self, PromptConfig};
use crate::log::EventLevel;

use super::{AppState, PromptTemplates};

pub(super) fn update_prompt_config(
    state: &mut AppState,
    out: &mut impl Write,
    args: &str,
) -> Result<()> {
    let args = args.trim_start();
    if args.is_empty() {
        return write_prompt_config(out, &state.prompt_templates);
    }
    let (field, rest) = split_first_word(args);
    match field {
        "draft" | "history" | "ai" => {
            let raw_value = rest.trim_start();
            let Some(value) = parse_prompt_template_value(raw_value) else {
                writeln!(out, "usage: #prompt [draft|history|ai <template>|reset]")?;
                return Ok(());
            };
            if value.is_empty() {
                writeln!(out, "prompt template must not be empty")?;
                return Ok(());
            }
            set_prompt_config(state, out, |config| {
                match field {
                    "draft" => config.prompt.draft = value,
                    "history" => config.prompt.history = value,
                    "ai" => config.prompt.ai = value,
                    _ => unreachable!("validated prompt field"),
                }
                Ok(())
            })
        }
        "reset" if rest.trim().is_empty() => set_prompt_config(state, out, |config| {
            config.prompt = PromptConfig::default();
            Ok(())
        }),
        _ => {
            writeln!(out, "usage: #prompt [draft|history|ai <template>|reset]").map_err(Into::into)
        }
    }
}

fn set_prompt_config(
    state: &mut AppState,
    out: &mut impl Write,
    update: impl FnOnce(&mut config::Config) -> Result<()>,
) -> Result<()> {
    let Some(path) = &state.config_path else {
        writeln!(out, "config path is not configured; #prompt not saved")?;
        return Ok(());
    };
    let mut config = match config::load_config(path) {
        Ok(config) => config,
        Err(err) => {
            state.append_event(EventLevel::Error, "config error")?;
            return Err(err);
        }
    };
    update(&mut config)?;
    config::normalize_config(&mut config);
    if let Err(err) = config::save_config(path, &config) {
        state.append_event(EventLevel::Error, "config error")?;
        return Err(err);
    }
    state.prompt_templates = config.prompt.into();
    write_prompt_config(out, &state.prompt_templates)
}

pub(super) fn write_prompt_config(out: &mut impl Write, prompt: &PromptTemplates) -> Result<()> {
    writeln!(
        out,
        "prompt.draft={}",
        format_prompt_template_value(&prompt.draft)
    )?;
    writeln!(
        out,
        "prompt.history={}",
        format_prompt_template_value(&prompt.history)
    )?;
    writeln!(
        out,
        "prompt.ai={}",
        format_prompt_template_value(&prompt.ai)
    )?;
    Ok(())
}

fn parse_prompt_template_value(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    match raw.chars().next()? {
        '"' => serde_json::from_str::<String>(raw).ok(),
        '\'' => raw
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
            .map(str::to_string),
        _ => Some(raw.to_string()),
    }
}

fn format_prompt_template_value(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("{value:?}"))
}

fn split_first_word(value: &str) -> (&str, &str) {
    let split_at = value.find(char::is_whitespace).unwrap_or(value.len());
    let (word, rest) = value.split_at(split_at);
    (word, rest)
}
