mod file;
mod layout;
mod model;
mod normalize;
mod paths;
mod private;

pub use file::{init_default_layout, load_config, load_or_create_config, save_config};
pub use layout::DirectoryLayout;
pub use model::{
    AiConfig, CompletionConfig, CompletionMode, CompletionTabAccept, Config, ContextConfig,
    DraftConfig, EditorConfig, EncryptionConfig, EncryptionStartupUnlockMode, PasteConfig,
    PromptConfig, ShellConfig, StorageConfig, SyncConfig, TemplateRemoteConfig,
    TemplateSharingConfig,
};
pub use normalize::normalize_config;
pub use paths::{default_aish_dir, runtime_aish_dir};
pub use private::{
    create_private_dir_all, set_private_dir_permissions, set_private_file_handle_permissions,
    set_private_file_permissions, write_private_file,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_config_matches_spec_basics() {
        let config = Config::default();
        assert_eq!(config.shell.backend, "auto");
        assert_eq!(config.prompt.draft, "{user}@{host} {cwd} > ");
        assert_eq!(config.prompt.history, "{user}@{host} {cwd} $ ");
        assert_eq!(config.prompt.ai, "{user}@{host} {cwd} % ");
        assert!(config.draft.persist);
        assert!(!config.draft.sync);
        assert!(config.editor.command.is_empty());
        assert!(!config.editor.execute_after_save);
        assert_eq!(config.paste.multiline, "editor");
        assert!(config.paste.confirm_execute);
        assert!(config.paste.preview);
        assert_eq!(config.paste.preview_lines, 3);
        assert_eq!(config.paste.preview_bytes, 240);
        assert_eq!(config.completion.max_results, 5);
        assert!(config.completion.enabled);
        assert_eq!(config.completion.coalesce_ms, 50);
        assert_eq!(config.completion.display_delay_ms, 120);
        assert!(config.completion.ignore_spaces);
        assert!(config.completion.template_first);
        assert!(config.completion.inline);
        assert!(config.completion.fuzzy);
        assert_eq!(config.completion.mode(), CompletionMode::Auto);
        assert_eq!(config.completion.tab_accept, CompletionTabAccept::Word);
        assert_eq!(config.completion.match_threshold_percent, 50);
        assert_eq!(config.completion.typo_threshold_percent, 80);
        assert_eq!(config.keybindings.history_search[0].as_str(), "Ctrl-R");
        assert_eq!(
            config.keybindings.external_editor[0].as_str(),
            "Ctrl-X Ctrl-E"
        );
        assert_eq!(config.ai, AiConfig::default());
        assert_eq!(config.context, ContextConfig::default());
        assert_eq!(config.encryption, EncryptionConfig::default());
        assert_eq!(config.sync, SyncConfig::default());
        assert!(config.storage.home.ends_with(".aish"));
    }

    #[test]
    fn normalize_replaces_empty_values() {
        let mut config = Config {
            shell: ShellConfig {
                backend: "   ".to_string(),
            },
            prompt: PromptConfig {
                draft: String::new(),
                history: String::new(),
                ai: String::new(),
            },
            storage: StorageConfig {
                home: PathBuf::new(),
            },
            draft: DraftConfig::default(),
            editor: EditorConfig {
                command: vec![String::new(), "vim".to_string()],
                execute_after_save: false,
            },
            paste: PasteConfig {
                multiline: "unknown".to_string(),
                confirm_execute: true,
                preview: true,
                preview_lines: 0,
                preview_bytes: 4_097,
            },
            completion: CompletionConfig {
                mode: None,
                enabled: true,
                max_results: 0,
                coalesce_ms: 1_001,
                display_delay_ms: 1_001,
                ignore_spaces: true,
                template_first: true,
                inline: true,
                fuzzy: true,
                tab_accept: CompletionTabAccept::Full,
                match_threshold_percent: 101,
                typo_threshold_percent: 101,
            },
            keybindings: crate::keybindings::KeybindingConfig::default(),
            ai: AiConfig {
                model: "  gpt-test  ".to_string(),
                base_url: "  https://example.invalid/v1  ".to_string(),
                env_key: "  OPENAI_API_KEY  ".to_string(),
                api_key_override: Some("must-not-persist".to_string()),
            },
            context: ContextConfig {
                enabled: false,
                confirm: false,
                max_bytes: 0,
            },
            encryption: EncryptionConfig {
                enabled: true,
                key_fingerprint: "  ABCDEF0123456789ABCDEF0123456789ABCDEF01  ".to_string(),
                startup_unlock: EncryptionStartupUnlockMode::Prompt,
                recipient: "  test@example.invalid  ".to_string(),
            },
            sync: SyncConfig {
                remote: "  git@example.invalid:aish.git  ".to_string(),
                enabled: true,
                schedule: "  0 * * * *  ".to_string(),
                startup: true,
                exit: false,
                ai: true,
                history: false,
                templates: true,
                drafts: false,
            },
            template_sharing: TemplateSharingConfig {
                remotes: vec![
                    TemplateRemoteConfig {
                        name: "  shared  ".to_string(),
                        remote: "  git@example.invalid:templates.git  ".to_string(),
                    },
                    TemplateRemoteConfig {
                        name: "   ".to_string(),
                        remote: "git@example.invalid:empty-name.git".to_string(),
                    },
                    TemplateRemoteConfig {
                        name: "../bad".to_string(),
                        remote: "git@example.invalid:bad.git".to_string(),
                    },
                    TemplateRemoteConfig {
                        name: "shared".to_string(),
                        remote: "git@example.invalid:duplicate.git".to_string(),
                    },
                    TemplateRemoteConfig {
                        name: "badremote".to_string(),
                        remote: "git@example.invalid:bad.git\n--upload-pack=x".to_string(),
                    },
                ],
            },
        };

        normalize_config(&mut config);

        let mut expected = Config::default();
        expected.editor.command = vec!["vim".to_string()];
        expected.ai = AiConfig {
            model: "gpt-test".to_string(),
            base_url: "https://example.invalid/v1".to_string(),
            env_key: "OPENAI_API_KEY".to_string(),
            api_key_override: None,
        };
        expected.context = ContextConfig {
            enabled: false,
            confirm: false,
            max_bytes: 65_536,
        };
        expected.encryption = EncryptionConfig {
            enabled: true,
            key_fingerprint: "ABCDEF0123456789ABCDEF0123456789ABCDEF01".to_string(),
            startup_unlock: EncryptionStartupUnlockMode::Prompt,
            recipient: "test@example.invalid".to_string(),
        };
        expected.completion.match_threshold_percent = 50;
        expected.completion.typo_threshold_percent = 80;
        expected.completion.coalesce_ms = 50;
        expected.completion.display_delay_ms = 120;
        expected.completion.tab_accept = CompletionTabAccept::Full;
        expected.sync = SyncConfig {
            remote: "git@example.invalid:aish.git".to_string(),
            enabled: true,
            schedule: "0 * * * *".to_string(),
            startup: true,
            exit: false,
            ai: true,
            history: false,
            templates: true,
            drafts: false,
        };
        expected.template_sharing = TemplateSharingConfig {
            remotes: vec![TemplateRemoteConfig {
                name: "shared".to_string(),
                remote: "git@example.invalid:templates.git".to_string(),
            }],
        };
        assert_eq!(config, expected);
    }

    #[test]
    fn normalize_trims_configured_shell_backend() {
        let mut config = Config::default();
        config.shell.backend = "  /usr/local/bin/fish  ".to_string();

        normalize_config(&mut config);

        assert_eq!(config.shell.backend, "/usr/local/bin/fish");
    }

    #[test]
    fn first_run_creates_layout_and_default_config() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("aish-home");

        let (layout, config) = init_default_layout(&root).unwrap();

        assert_eq!(layout.root, root);
        assert!(layout.config.exists());
        assert!(layout.history.is_dir());
        assert!(layout.templates.is_dir());
        assert!(layout.secrets.is_dir());
        assert!(layout.logs.is_dir());
        assert_eq!(layout.events, root.join("logs/events.jsonl"));
        assert!(layout.runtime_cache.is_dir());
        assert_eq!(config.storage.home, layout.root);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            for dir in [
                &layout.root,
                &layout.history,
                &layout.templates,
                &layout.secrets,
                &layout.logs,
                &layout.cache,
                &layout.runtime_cache,
            ] {
                let mode = fs::metadata(dir).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o700, "directory is not private: {}", dir.display());
            }
            let mode = fs::metadata(&layout.config).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn invalid_config_has_readable_error() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        fs::write(&path, "[shell\nbackend = true").unwrap();

        let err = load_config(&path).unwrap_err().to_string();

        assert!(err.contains("invalid config"));
        assert!(err.contains("config.toml"));
    }

    #[test]
    fn completion_tab_accept_empty_normalizes_to_default() {
        let raw = r#"
            [completion]
            tab_accept = ""
        "#;

        let config: Config = toml::from_str(raw).unwrap();

        assert_eq!(config.completion.tab_accept, CompletionTabAccept::Word);
    }

    #[test]
    fn completion_tab_accept_rejects_unsupported_modes() {
        let raw = r#"
            [completion]
            tab_accept = "line"
        "#;

        let err = toml::from_str::<Config>(raw).unwrap_err().to_string();

        assert!(err.contains("invalid value"));
        assert!(err.contains("full"));
        assert!(err.contains("word"));
    }

    #[test]
    fn completion_mode_overrides_legacy_enabled_inline_fields() {
        let raw = r#"
            [completion]
            mode = "tab"
            enabled = false
            inline = true
        "#;

        let mut config: Config = toml::from_str(raw).unwrap();
        normalize_config(&mut config);

        assert_eq!(config.completion.mode(), CompletionMode::Tab);
        assert!(config.completion.enabled);
        assert!(!config.completion.inline);
    }

    #[test]
    fn completion_mode_rejects_unknown_values() {
        let raw = r#"
            [completion]
            mode = "manual"
        "#;

        let err = toml::from_str::<Config>(raw).unwrap_err().to_string();

        assert!(err.contains("unknown variant"));
        assert!(err.contains("auto"));
        assert!(err.contains("tab"));
        assert!(err.contains("off"));
    }

    #[test]
    fn keybinding_config_rejects_invalid_key_sequences() {
        let raw = r#"
            [keybindings]
            history_search = ["Ctrl-"]
        "#;

        let err = toml::from_str::<Config>(raw).unwrap_err().to_string();

        assert!(err.contains("invalid key binding"));
    }

    #[test]
    fn partial_keybinding_config_preserves_unspecified_defaults() {
        let raw = r#"
            [keybindings]
            history_search = ["Ctrl-P"]
            file_picker = []
        "#;

        let config: Config = toml::from_str(raw).unwrap();

        assert_eq!(config.keybindings.history_search[0].as_str(), "Ctrl-P");
        assert!(config.keybindings.file_picker.is_empty());
        assert_eq!(config.keybindings.clear_screen[0].as_str(), "Ctrl-L");
        assert_eq!(
            config.keybindings.external_editor[0].as_str(),
            "Ctrl-X Ctrl-E"
        );
    }

    #[test]
    fn config_roundtrips_through_json_for_future_jsonl_storage() {
        let config = Config::default();

        let raw = serde_json::to_string(&config).unwrap();
        let parsed: Config = serde_json::from_str(&raw).unwrap();

        assert_eq!(parsed, config);
    }

    #[test]
    fn aish_home_environment_overrides_default_root() {
        let _guard = ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("AISH_HOME", temp.path());
        }

        let root = default_aish_dir();

        unsafe {
            std::env::remove_var("AISH_HOME");
        }
        assert_eq!(root, temp.path());
    }

    #[test]
    fn runtime_aish_dir_rejects_missing_home() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("AISH_HOME");
            std::env::set_var("HOME", "");
        }

        let err = runtime_aish_dir().unwrap_err().to_string();

        unsafe {
            std::env::remove_var("HOME");
        }
        assert!(err.contains("AISH_HOME or HOME must be set to an absolute path"));
    }

    #[test]
    fn runtime_aish_dir_empty_aish_home_falls_back_to_home() {
        let _guard = ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("AISH_HOME", "");
            std::env::set_var("HOME", temp.path());
        }

        let root = runtime_aish_dir().unwrap();

        unsafe {
            std::env::remove_var("AISH_HOME");
            std::env::remove_var("HOME");
        }
        assert_eq!(root, temp.path().join(".aish"));
    }

    #[test]
    fn runtime_aish_dir_rejects_relative_aish_home() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("AISH_HOME", "relative-aish");
        }

        let err = runtime_aish_dir().unwrap_err().to_string();

        unsafe {
            std::env::remove_var("AISH_HOME");
        }
        assert!(err.contains("AISH_HOME must be set to an absolute path"));
    }
}
