pub(crate) fn is_shell_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_variable_names_match_shell_assignment_shape() {
        assert!(is_shell_variable_name("OPENAI_API_KEY"));
        assert!(is_shell_variable_name("_AISH_SECRET"));
        assert!(!is_shell_variable_name(""));
        assert!(!is_shell_variable_name("1BAD"));
        assert!(!is_shell_variable_name("BAD-NAME"));
        assert!(!is_shell_variable_name("BAD NAME"));
    }
}
