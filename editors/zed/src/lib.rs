use zed_extension_api::{self as zed, settings::LspSettings, Result};

struct GinExtension;

impl zed::Extension for GinExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree).ok();

        let binary = settings
            .as_ref()
            .and_then(|s| s.binary.as_ref())
            .and_then(|b| b.path.clone())
            .or_else(|| worktree.which("gin-language-server"))
            .ok_or_else(|| "gin-language-server not found in PATH".to_string())?;

        Ok(zed::Command {
            command: binary,
            args: vec![],
            env: Default::default(),
        })
    }
}

zed::register_extension!(GinExtension);
