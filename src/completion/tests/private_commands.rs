use super::*;

#[test]
fn private_command_completion_uses_aish_commands_only() {
    let candidates = complete_private_commands("#sta", usize::MAX);

    assert_eq!(
        candidates,
        [CompletionCandidate {
            display: "#status".to_string(),
            replacement: "#status".to_string(),
            is_dir: false,
            source: CompletionSource::PrivateCommand,
        }]
    );
    assert!(complete_private_commands("#", usize::MAX).is_empty());
    assert!(complete_private_commands("# ", usize::MAX).is_empty());
}

#[test]
fn private_command_completion_includes_nested_arguments() {
    let candidates =
        complete_private_command_line("#completion ", "#completion ".len(), usize::MAX);

    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.replacement == "mode")
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.replacement == "tab-accept")
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.replacement == "display-delay-ms")
    );

    let paste_candidates = complete_private_command_line("#paste ", "#paste ".len(), usize::MAX);
    assert_eq!(
        paste_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        [
            "multiline",
            "confirm",
            "confirm-execute",
            "preview",
            "preview-lines",
            "preview-bytes"
        ]
    );
    let paste_preview_candidates =
        complete_private_command_line("#paste preview ", "#paste preview ".len(), usize::MAX);
    assert_eq!(
        paste_preview_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["on", "off"]
    );

    let prompt_candidates = complete_private_command_line("#prompt ", "#prompt ".len(), usize::MAX);
    assert_eq!(
        prompt_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["draft", "history", "ai", "reset"]
    );

    let sync_candidates = complete_private_command_line("#sync ", "#sync ".len(), usize::MAX);
    assert_eq!(
        sync_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        [
            "now",
            "abort",
            "continue",
            "resolve-union",
            "off",
            "startup",
            "exit",
            "ai",
            "history",
            "templates",
            "drafts"
        ]
    );
    let sync_trigger_candidates =
        complete_private_command_line("#sync startup ", "#sync startup ".len(), usize::MAX);
    assert_eq!(
        sync_trigger_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["on", "off"]
    );

    let template_candidates =
        complete_private_command_line("#template ", "#template ".len(), usize::MAX);
    assert_eq!(
        template_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        [
            "analyze", "fetch", "find", "import", "list", "publish", "remote", "rm", "replace",
            "search", "show", "use"
        ]
    );
    let template_remote_candidates =
        complete_private_command_line("#template remote ", "#template remote ".len(), usize::MAX);
    assert_eq!(
        template_remote_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["add", "list", "rm"]
    );
    let template_publish_candidates =
        complete_private_command_line("#template publish ", "#template publish ".len(), usize::MAX);
    assert_eq!(
        template_publish_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["--encrypt"]
    );

    let history_candidates =
        complete_private_command_line("#history ", "#history ".len(), usize::MAX);
    assert_eq!(
        history_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["list", "search"]
    );

    let encrypt_candidates =
        complete_private_command_line("#encrypt ", "#encrypt ".len(), usize::MAX);
    assert!(
        encrypt_candidates
            .iter()
            .any(|candidate| candidate.replacement == "unlock-mode")
    );
    let unlock_mode_candidates = complete_private_command_line(
        "#encrypt unlock-mode ",
        "#encrypt unlock-mode ".len(),
        usize::MAX,
    );
    assert_eq!(
        unlock_mode_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["lazy", "prompt"]
    );

    let mode_candidates =
        complete_private_command_line("#completion mode ", "#completion mode ".len(), usize::MAX);

    assert_eq!(
        mode_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["auto", "tab", "off"]
    );

    let partial_arg_candidates =
        complete_private_command_line("#completion m", "#completion m".len(), usize::MAX);

    assert_eq!(
        partial_arg_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["mode", "max", "match-threshold"]
    );

    let partial_nested_candidates =
        complete_private_command_line("#completion mode t", "#completion mode t".len(), usize::MAX);

    assert_eq!(
        partial_nested_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["tab"]
    );

    let help_candidates = complete_private_command_line("#help ", "#help ".len(), usize::MAX);
    assert_eq!(
        help_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        [
            "commands",
            "keys",
            "ai",
            "paste",
            "completion",
            "templates",
            "sync",
            "encryption",
            "config"
        ]
    );

    let partial_help_candidates = complete_private_command_line("#help c", "#help c".len(), 10);
    assert_eq!(
        partial_help_candidates
            .iter()
            .map(|candidate| candidate.replacement.as_str())
            .collect::<Vec<_>>(),
        ["commands", "completion", "config"]
    );

    assert!(complete_private_command_line("# ", "# ".len(), usize::MAX).is_empty());
}
