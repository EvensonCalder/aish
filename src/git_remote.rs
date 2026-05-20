pub(crate) fn sanitize_git_remote(remote: &str) -> Option<String> {
    let remote = remote.trim();
    if remote.is_empty() || remote.starts_with('-') || remote.chars().any(char::is_control) {
        return None;
    }
    Some(remote.to_string())
}

pub(crate) fn valid_git_branch_name(branch: &str) -> bool {
    if branch.is_empty()
        || branch != branch.trim()
        || branch == "@"
        || branch.starts_with('-')
        || branch.starts_with('/')
        || branch.ends_with('/')
        || branch.ends_with('.')
        || branch.contains("..")
        || branch.contains("//")
        || branch.contains("@{")
    {
        return false;
    }
    if branch
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\'))
    {
        return false;
    }
    branch.split('/').all(|component| {
        !component.is_empty() && !component.starts_with('.') && !component.ends_with(".lock")
    })
}

pub(crate) fn valid_template_remote_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_remote_sanitizer_trims_and_rejects_control_characters() {
        assert_eq!(
            sanitize_git_remote("  git@example.invalid:aish.git  "),
            Some("git@example.invalid:aish.git".to_string())
        );
        assert_eq!(sanitize_git_remote(""), None);
        assert_eq!(sanitize_git_remote("--upload-pack=/tmp/hook"), None);
        assert_eq!(
            sanitize_git_remote("git@example.invalid:aish.git\n--upload-pack=x"),
            None
        );
    }

    #[test]
    fn git_branch_names_reject_invalid_or_option_like_refs() {
        assert!(valid_git_branch_name("main"));
        assert!(valid_git_branch_name("feature/aish-sync"));
        assert!(valid_git_branch_name("release_2026-05"));
        assert!(!valid_git_branch_name(""));
        assert!(!valid_git_branch_name("-main"));
        assert!(!valid_git_branch_name("main lock"));
        assert!(!valid_git_branch_name("main..other"));
        assert!(!valid_git_branch_name(".hidden"));
        assert!(!valid_git_branch_name("feature/.hidden"));
        assert!(!valid_git_branch_name("feature/main.lock"));
        assert!(!valid_git_branch_name("feature//main"));
        assert!(!valid_git_branch_name("main@{1}"));
        assert!(!valid_git_branch_name("main."));
        assert!(!valid_git_branch_name("main\nother"));
    }

    #[test]
    fn template_remote_names_allow_only_stable_path_components() {
        assert!(valid_template_remote_name("shared"));
        assert!(valid_template_remote_name("team-templates_1"));
        assert!(!valid_template_remote_name(""));
        assert!(!valid_template_remote_name("../shared"));
        assert!(!valid_template_remote_name("shared remote"));
    }
}
