use std::io::Write;

use anyhow::Result;

use crate::ai::normalize_chat_completions_url;
use crate::config::{self, CompletionConfig, CompletionMode, CompletionTabAccept, ContextConfig};
use crate::log::EventLevel;

use super::AppState;

pub(super) fn update_ai_config_field(
    state: &mut AppState,
    out: &mut impl Write,
    name: &str,
    args: &str,
) -> Result<()> {
    let value = args.trim();
    if value.is_empty() {
        write_ai_config_value(out, name, state)?;
        return Ok(());
    }
    let Some(path) = &state.config_path else {
        writeln!(out, "config path is not configured; #{name} not saved")?;
        return Ok(());
    };

    let mut config = match config::load_config(path) {
        Ok(config) => config,
        Err(err) => {
            state.append_event(EventLevel::Error, "config error")?;
            return Err(err);
        }
    };
    match name {
        "model" => config.ai.model = value.to_string(),
        "base-url" => config.ai.base_url = normalize_chat_completions_url(value)?,
        "env-key" => config.ai.env_key = value.to_string(),
        _ => unreachable!("unknown AI config field"),
    }
    config::normalize_config(&mut config);
    if let Err(err) = config::save_config(path, &config) {
        state.append_event(EventLevel::Error, "config error")?;
        return Err(err);
    }
    state.ai_config = config.ai;
    write_ai_config_value(out, name, state)
}

fn write_ai_config_value(out: &mut impl Write, name: &str, state: &AppState) -> Result<()> {
    let value = match name {
        "model" => &state.ai_config.model,
        "base-url" => &state.ai_config.base_url,
        "env-key" => &state.ai_config.env_key,
        _ => unreachable!("unknown AI config field"),
    };
    if value.is_empty() {
        writeln!(out, "#{name}=unconfigured")?;
    } else {
        writeln!(out, "#{name}={value}")?;
    }
    Ok(())
}

pub(super) fn update_context_config(
    state: &mut AppState,
    out: &mut impl Write,
    args: &str,
) -> Result<()> {
    let mut parts = args.split_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (None, None, None) => write_context_config(out, &state.context_config),
        (Some("on"), None, None) => set_context_config(state, out, |config| {
            config.context.enabled = true;
            Ok(())
        }),
        (Some("off"), None, None) => set_context_config(state, out, |config| {
            config.context.enabled = false;
            Ok(())
        }),
        (Some("confirm"), Some("on"), None) => set_context_config(state, out, |config| {
            config.context.confirm = true;
            Ok(())
        }),
        (Some("confirm"), Some("off"), None) => set_context_config(state, out, |config| {
            config.context.confirm = false;
            Ok(())
        }),
        (Some(bytes), None, None) => {
            let max_bytes = bytes.parse::<usize>()?;
            if max_bytes == 0 {
                writeln!(out, "context max bytes must be greater than 0")?;
                return Ok(());
            }
            set_context_config(state, out, |config| {
                config.context.max_bytes = max_bytes;
                Ok(())
            })
        }
        _ => writeln!(
            out,
            "usage: #context [on|off|confirm on|confirm off|<bytes>]"
        )
        .map_err(Into::into),
    }
}

fn set_context_config(
    state: &mut AppState,
    out: &mut impl Write,
    update: impl FnOnce(&mut config::Config) -> Result<()>,
) -> Result<()> {
    let Some(path) = &state.config_path else {
        writeln!(out, "config path is not configured; #context not saved")?;
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
    state.context_config = config.context;
    write_context_config(out, &state.context_config)
}

fn write_context_config(out: &mut impl Write, config: &ContextConfig) -> Result<()> {
    writeln!(
        out,
        "context.enabled={} context.confirm={} context.max_bytes={}",
        config.enabled, config.confirm, config.max_bytes
    )?;
    Ok(())
}

pub(super) fn update_completion_config(
    state: &mut AppState,
    out: &mut impl Write,
    args: &str,
) -> Result<()> {
    let mut parts = args.split_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (None, None, None) => write_completion_config(out, &state.completion_config),
        (Some(value @ ("on" | "off")), None, None) => {
            let mode = if value == "on" {
                CompletionMode::Auto
            } else {
                CompletionMode::Off
            };
            set_completion_config(state, out, |config| {
                config.completion.set_mode(mode);
                Ok(())
            })
        }
        (Some("mode"), Some(value), None) => {
            let Some(mode) = parse_completion_mode(value) else {
                writeln!(out, "usage: #completion mode auto|tab|off")?;
                return Ok(());
            };
            set_completion_config(state, out, |config| {
                config.completion.set_mode(mode);
                Ok(())
            })
        }
        (Some("max"), Some(count), None) => {
            let max_results = count.parse::<usize>();
            let Ok(max_results) = max_results else {
                writeln!(out, "usage: #completion max <count>")?;
                return Ok(());
            };
            if max_results == 0 {
                writeln!(out, "completion max results must be greater than 0")?;
                return Ok(());
            }
            set_completion_config(state, out, |config| {
                config.completion.max_results = max_results;
                Ok(())
            })
        }
        (Some("coalesce" | "coalesce-ms"), Some(value), None) => {
            let Ok(coalesce_ms) = value.parse::<u64>() else {
                writeln!(out, "usage: #completion coalesce-ms <0-1000>")?;
                return Ok(());
            };
            if coalesce_ms > 1_000 {
                writeln!(out, "completion coalesce ms must be between 0 and 1000")?;
                return Ok(());
            }
            set_completion_config(state, out, |config| {
                config.completion.coalesce_ms = coalesce_ms;
                Ok(())
            })
        }
        (Some("display-delay" | "display-delay-ms"), Some(value), None) => {
            let Ok(display_delay_ms) = value.parse::<u64>() else {
                writeln!(out, "usage: #completion display-delay-ms <0-1000>")?;
                return Ok(());
            };
            if display_delay_ms > 1_000 {
                writeln!(out, "completion display delay ms must be between 0 and 1000")?;
                return Ok(());
            }
            set_completion_config(state, out, |config| {
                config.completion.display_delay_ms = display_delay_ms;
                Ok(())
            })
        }
        (Some("inline"), Some(value), None) => {
            let Some(inline) = parse_on_off(value) else {
                writeln!(out, "usage: #completion inline on|off")?;
                return Ok(());
            };
            set_completion_config(state, out, |config| {
                config.completion.set_mode(if inline {
                    CompletionMode::Auto
                } else {
                    CompletionMode::Tab
                });
                Ok(())
            })
        }
        (Some("fuzzy"), Some(value), None) => {
            let Some(fuzzy) = parse_on_off(value) else {
                writeln!(out, "usage: #completion fuzzy on|off")?;
                return Ok(());
            };
            set_completion_config(state, out, |config| {
                config.completion.fuzzy = fuzzy;
                Ok(())
            })
        }
        (Some("tab-accept"), Some(value), None) => {
            let Some(tab_accept) = parse_completion_tab_accept(value) else {
                writeln!(out, "usage: #completion tab-accept full|word")?;
                return Ok(());
            };
            set_completion_config(state, out, |config| {
                config.completion.tab_accept = tab_accept;
                Ok(())
            })
        }
        (Some("match-threshold"), Some(value), None) => {
            let Ok(percent) = value.parse::<usize>() else {
                writeln!(out, "usage: #completion match-threshold <0-100>")?;
                return Ok(());
            };
            if percent > 100 {
                writeln!(out, "completion match threshold must be between 0 and 100")?;
                return Ok(());
            }
            set_completion_config(state, out, |config| {
                config.completion.match_threshold_percent = percent;
                Ok(())
            })
        }
        (Some("typo-threshold"), Some(value), None) => {
            let Ok(percent) = value.parse::<usize>() else {
                writeln!(out, "usage: #completion typo-threshold <0-100>")?;
                return Ok(());
            };
            if percent > 100 {
                writeln!(out, "completion typo threshold must be between 0 and 100")?;
                return Ok(());
            }
            set_completion_config(state, out, |config| {
                config.completion.typo_threshold_percent = percent;
                Ok(())
            })
        }
        _ => writeln!(
            out,
            "usage: #completion on|off|mode auto|tab|off|max <count>|coalesce-ms <0-1000>|display-delay-ms <0-1000>|inline on|off|fuzzy on|off|tab-accept full|word|match-threshold <0-100>|typo-threshold <0-100>"
        )
        .map_err(Into::into),
    }
}

fn set_completion_config(
    state: &mut AppState,
    out: &mut impl Write,
    update: impl FnOnce(&mut config::Config) -> Result<()>,
) -> Result<()> {
    let Some(path) = &state.config_path else {
        writeln!(out, "config path is not configured; #completion not saved")?;
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
    state.completion_config = config.completion;
    write_completion_config(out, &state.completion_config)
}

fn write_completion_config(out: &mut impl Write, config: &CompletionConfig) -> Result<()> {
    writeln!(out, "completion.mode={}", config.mode().as_str())?;
    writeln!(out, "completion.enabled={}", config.enabled)?;
    writeln!(out, "completion.max_results={}", config.max_results)?;
    writeln!(out, "completion.coalesce_ms={}", config.coalesce_ms)?;
    writeln!(
        out,
        "completion.display_delay_ms={}",
        config.display_delay_ms
    )?;
    writeln!(out, "completion.ignore_spaces={}", config.ignore_spaces)?;
    writeln!(out, "completion.template_first={}", config.template_first)?;
    writeln!(out, "completion.inline={}", config.inline)?;
    writeln!(out, "completion.fuzzy={}", config.fuzzy)?;
    writeln!(out, "completion.tab_accept={}", config.tab_accept.as_str())?;
    writeln!(
        out,
        "completion.match_threshold_percent={}",
        config.match_threshold_percent
    )?;
    writeln!(
        out,
        "completion.typo_threshold_percent={}",
        config.typo_threshold_percent
    )?;
    Ok(())
}

fn parse_completion_mode(value: &str) -> Option<CompletionMode> {
    match value {
        "auto" => Some(CompletionMode::Auto),
        "tab" => Some(CompletionMode::Tab),
        "off" => Some(CompletionMode::Off),
        _ => None,
    }
}

fn parse_completion_tab_accept(value: &str) -> Option<CompletionTabAccept> {
    match value {
        "full" => Some(CompletionTabAccept::Full),
        "word" => Some(CompletionTabAccept::Word),
        _ => None,
    }
}

fn parse_on_off(value: &str) -> Option<bool> {
    match value {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}
