pub fn plaintext_git_history_warning() -> &'static str {
    "warning: existing plaintext data may remain in git history; Aish will not rewrite history automatically"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaintext_git_history_warning_is_conservative() {
        let warning = plaintext_git_history_warning();

        assert!(warning.contains("plaintext data may remain in git history"));
        assert!(warning.contains("will not rewrite history automatically"));
    }
}
