use std::path::Path;

#[path = "support/shell.rs"]
mod shell_support;
#[path = "support/tmux.rs"]
mod tmux_support;

use tmux_support::{
    assert_adjacent_output, assert_at_least_n_lines, assert_common_shell_workflow_output,
    assert_first_non_empty_line, assert_line_absent, assert_line_prefix, assert_line_present,
    command_available, find_shell, fish_backend_tests_enabled, run_tmux_script,
    run_tmux_script_with_env,
};

#[path = "tmux_capture/manual.rs"]
mod manual;
#[path = "tmux_capture/modes_lifecycle.rs"]
mod modes_lifecycle;
#[path = "tmux_capture/passthrough_prompts.rs"]
mod passthrough_prompts;
#[path = "tmux_capture/render_completion.rs"]
mod render_completion;
#[path = "tmux_capture/shell_backends.rs"]
mod shell_backends;
