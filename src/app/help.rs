use std::io::Write;

use anyhow::Result;

use crate::commands::HELP_TOPICS;
use crate::keybindings::default_keybindings;

#[derive(Debug, Clone, Copy)]
struct HelpEntry {
    usage: &'static str,
    description: &'static str,
}

const HELP_USAGE: &str =
    "usage: #help [commands|keys|ai|completion|templates|sync|encryption|config]";

const COMMAND_HELP: &[HelpEntry] = &[
    HelpEntry {
        usage: "#help [topic]",
        description: "show grouped help or one help topic",
    },
    HelpEntry {
        usage: "#status",
        description: "show current mode, shell, AI, completion, encryption, and sync status",
    },
    HelpEntry {
        usage: "#config",
        description: "show effective runtime configuration",
    },
    HelpEntry {
        usage: "#doctor",
        description: "show diagnostics for shell, storage, editor, GPG, and sync",
    },
    HelpEntry {
        usage: "#prompt [draft|history|ai <template>|reset]",
        description: "show or update prompt templates",
    },
    HelpEntry {
        usage: "#model [name]",
        description: "show or update the AI model",
    },
    HelpEntry {
        usage: "#base-url [url]",
        description: "show or update the chat-completions-compatible endpoint",
    },
    HelpEntry {
        usage: "#env-key [ENV_NAME]",
        description: "show or update the API key environment variable name",
    },
    HelpEntry {
        usage: "#key set | #key clear",
        description: "store or clear the encrypted API key override",
    },
    HelpEntry {
        usage: "#context [on|off|confirm on|confirm off|<bytes>]",
        description: "show or update context capture settings",
    },
    HelpEntry {
        usage: "#completion [subcommand]",
        description: "show or update completion behavior",
    },
    HelpEntry {
        usage: "#log <count>",
        description: "show recent Aish event log entries",
    },
    HelpEntry {
        usage: "#editor",
        description: "show editor configuration and resolution",
    },
    HelpEntry {
        usage: "#history <count>",
        description: "trim stored shell and AI history",
    },
    HelpEntry {
        usage: "#mt <template-body>",
        description: "store a reusable command template",
    },
    HelpEntry {
        usage: "#template <subcommand>",
        description: "find, show, use, remove, or replace templates",
    },
    HelpEntry {
        usage: "#encrypt <subcommand>",
        description: "enable, rotate, disable, or rewrite encrypted storage",
    },
    HelpEntry {
        usage: "#set-remote <git-url>",
        description: "save the sync remote without running git",
    },
    HelpEntry {
        usage: "#push",
        description: "run the conservative manual sync push flow",
    },
    HelpEntry {
        usage: "#sync [off|<cron>|ai|history|templates|drafts on|off]",
        description: "show or update sync schedule and category settings",
    },
    HelpEntry {
        usage: "#exit",
        description: "exit Aish",
    },
    HelpEntry {
        usage: "#quit",
        description: "exit Aish",
    },
];

const AI_HELP: &[HelpEntry] = &[
    HelpEntry {
        usage: "# <prompt>",
        description: "send an AI prompt",
    },
    HelpEntry {
        usage: "# <prompt> < <command>",
        description: "ask with captured command output as context",
    },
    HelpEntry {
        usage: "#model [name]",
        description: "show or update the model",
    },
    HelpEntry {
        usage: "#base-url [url]",
        description: "show or update the chat-completions endpoint",
    },
    HelpEntry {
        usage: "#env-key [ENV_NAME]",
        description: "show or update the API key environment variable name",
    },
    HelpEntry {
        usage: "#key set | #key clear",
        description: "store or clear the encrypted API key override",
    },
    HelpEntry {
        usage: "#context [on|off|confirm on|confirm off|<bytes>]",
        description: "show or update context capture settings",
    },
    HelpEntry {
        usage: "# TODO: <text>",
        description: "store a note",
    },
    HelpEntry {
        usage: "# NOTE: <text>",
        description: "store a note",
    },
    HelpEntry {
        usage: "# FIXME: <text>",
        description: "store a note",
    },
    HelpEntry {
        usage: "# HACK: <text>",
        description: "store a note",
    },
    HelpEntry {
        usage: "# XXX: <text>",
        description: "store a note",
    },
];

const COMPLETION_HELP: &[HelpEntry] = &[
    HelpEntry {
        usage: "#completion",
        description: "show completion settings",
    },
    HelpEntry {
        usage: "#completion on|off",
        description: "enable auto completion or disable all Aish completion",
    },
    HelpEntry {
        usage: "#completion mode auto|tab|off",
        description: "auto shows live hints while typing; tab waits for Tab; off disables completion",
    },
    HelpEntry {
        usage: "#completion max <count>",
        description: "set below-prompt candidate row count",
    },
    HelpEntry {
        usage: "#completion coalesce-ms <0-1000>",
        description: "coalesce fast completion tiers before refreshing the UI",
    },
    HelpEntry {
        usage: "#completion display-delay-ms <0-1000>",
        description: "delay auto-mode drawing while matching continues in the background",
    },
    HelpEntry {
        usage: "#completion inline on|off",
        description: "legacy switch; off maps to tab-triggered completion",
    },
    HelpEntry {
        usage: "#completion fuzzy on|off",
        description: "enable or disable typo-correction work",
    },
    HelpEntry {
        usage: "#completion tab-accept full|word",
        description: "choose whether Tab accepts the whole suggestion or one word",
    },
    HelpEntry {
        usage: "#completion match-threshold <0-100>",
        description: "set structural word-position match threshold",
    },
    HelpEntry {
        usage: "#completion typo-threshold <0-100>",
        description: "set typo-correction similarity threshold",
    },
];

const TEMPLATE_HELP: &[HelpEntry] = &[
    HelpEntry {
        usage: "#mt <template-body>",
        description: "store a reusable template",
    },
    HelpEntry {
        usage: "#template find <query>",
        description: "find templates by id or body text",
    },
    HelpEntry {
        usage: "#template show <id>",
        description: "show the newest matching template",
    },
    HelpEntry {
        usage: "#template use <id> [key=value ...]",
        description: "copy a template into the draft with optional placeholder values",
    },
    HelpEntry {
        usage: "#template rm <id>",
        description: "remove matching templates",
    },
    HelpEntry {
        usage: "#template replace <id> <template-body>",
        description: "replace matching templates with a new body",
    },
];

const SYNC_HELP: &[HelpEntry] = &[
    HelpEntry {
        usage: "#set-remote <git-url>",
        description: "save the sync remote without running git",
    },
    HelpEntry {
        usage: "#push",
        description: "run conservative git init/add/commit/pull/push steps",
    },
    HelpEntry {
        usage: "#sync",
        description: "show sync and encryption status without running git",
    },
    HelpEntry {
        usage: "#sync <cron-expression>",
        description: "save the startup sync schedule",
    },
    HelpEntry {
        usage: "#sync off",
        description: "disable startup sync",
    },
    HelpEntry {
        usage: "#sync ai|history|templates|drafts on|off",
        description: "enable or disable managed sync categories",
    },
];

const ENCRYPTION_HELP: &[HelpEntry] = &[
    HelpEntry {
        usage: "#encrypt on [key-fingerprint|unique-email]",
        description: "enable encrypted storage and migrate current storage",
    },
    HelpEntry {
        usage: "#encrypt rotate <key-fingerprint|unique-email>",
        description: "decrypt with the current key and re-encrypt with the new key",
    },
    HelpEntry {
        usage: "#encrypt rewrite-history plan",
        description: "print the explicit destructive history rewrite plan",
    },
    HelpEntry {
        usage: "#encrypt rewrite-history run <key-fingerprint|unique-email> --confirm-rewrite-history",
        description: "rewrite managed storage in git history after explicit confirmation",
    },
    HelpEntry {
        usage: "#encrypt off",
        description: "decrypt current storage and write plaintext from now on",
    },
    HelpEntry {
        usage: "#key set | #key clear",
        description: "store or clear the encrypted AI API key override",
    },
];

const CONFIG_HELP: &[HelpEntry] = &[
    HelpEntry {
        usage: "#status",
        description: "show current runtime state",
    },
    HelpEntry {
        usage: "#config",
        description: "show effective runtime configuration",
    },
    HelpEntry {
        usage: "#doctor",
        description: "show diagnostics for shell, storage, editor, GPG, and sync",
    },
    HelpEntry {
        usage: "#prompt [draft|history|ai <template>|reset]",
        description: "show or update prompt templates",
    },
    HelpEntry {
        usage: "#editor",
        description: "show editor configuration and resolution",
    },
    HelpEntry {
        usage: "#log <count>",
        description: "show recent Aish event log entries",
    },
    HelpEntry {
        usage: "#history <count>",
        description: "trim stored shell and AI history",
    },
];

pub(super) fn write_help(out: &mut impl Write, args: &str) -> Result<()> {
    let mut parts = args.split_whitespace();
    match (parts.next(), parts.next()) {
        (None, None) => write_full_help(out),
        (Some("commands"), None) => write_commands_help(out),
        (Some("keys"), None) => write_keys_help(out),
        (Some("ai"), None) => write_ai_help(out),
        (Some("completion"), None) => write_completion_help(out),
        (Some("templates"), None) => write_templates_help(out),
        (Some("sync"), None) => write_sync_help(out),
        (Some("encryption"), None) => write_encryption_help(out),
        (Some("config"), None) => write_config_help(out),
        (Some(topic), None) => {
            writeln!(out, "unknown help topic: {topic}")?;
            writeln!(out, "{HELP_USAGE}")?;
            Ok(())
        }
        _ => {
            writeln!(out, "{HELP_USAGE}")?;
            Ok(())
        }
    }
}

fn write_full_help(out: &mut impl Write) -> Result<()> {
    writeln!(out, "Aish help")?;
    writeln!(out, "Usage:")?;
    writeln!(out, "  #help [topic]")?;
    writeln!(out, "Topics:")?;
    writeln!(out, "  {}", HELP_TOPICS.join(", "))?;
    writeln!(out)?;
    write_commands_help(out)?;
    writeln!(out)?;
    write_keys_help(out)?;
    writeln!(out)?;
    write_ai_help(out)?;
    writeln!(out)?;
    write_completion_help(out)?;
    writeln!(out)?;
    write_templates_help(out)?;
    writeln!(out)?;
    write_sync_help(out)?;
    writeln!(out)?;
    write_encryption_help(out)?;
    writeln!(out)?;
    write_config_help(out)?;
    Ok(())
}

fn write_commands_help(out: &mut impl Write) -> Result<()> {
    writeln!(out, "Private commands:")?;
    write_entries(out, COMMAND_HELP)
}

fn write_keys_help(out: &mut impl Write) -> Result<()> {
    writeln!(out, "Keybindings:")?;
    for binding in default_keybindings() {
        writeln!(out, "  {} - {}", binding.key, binding.action)?;
    }
    Ok(())
}

fn write_ai_help(out: &mut impl Write) -> Result<()> {
    writeln!(out, "AI and notes:")?;
    write_entries(out, AI_HELP)
}

fn write_completion_help(out: &mut impl Write) -> Result<()> {
    writeln!(out, "Completion help")?;
    write_entries(out, COMPLETION_HELP)
}

fn write_templates_help(out: &mut impl Write) -> Result<()> {
    writeln!(out, "Template help")?;
    write_entries(out, TEMPLATE_HELP)
}

fn write_sync_help(out: &mut impl Write) -> Result<()> {
    writeln!(out, "Sync help")?;
    write_entries(out, SYNC_HELP)
}

fn write_encryption_help(out: &mut impl Write) -> Result<()> {
    writeln!(out, "Encryption help")?;
    write_entries(out, ENCRYPTION_HELP)
}

fn write_config_help(out: &mut impl Write) -> Result<()> {
    writeln!(out, "Config and diagnostics help")?;
    write_entries(out, CONFIG_HELP)
}

fn write_entries(out: &mut impl Write, entries: &[HelpEntry]) -> Result<()> {
    for entry in entries {
        writeln!(out, "  {} - {}", entry.usage, entry.description)?;
    }
    Ok(())
}
