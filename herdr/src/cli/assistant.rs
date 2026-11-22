//! Local native assistant discovery and bounded answer capture.
use crate::agent_runner::{self, AgentRequest, AgentRunState, CancellationToken};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) fn resolve_codex() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("HERDR_CODEX_PATH") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path).ok_or(
            "The configured coding assistant is missing. Update its location in setup.".into(),
        );
    }
    if let Some(path) = std::env::var_os("CODEX_CLI_PATH")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        return Ok(path);
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
    {
        dirs.extend([
            home.join(".local/bin"),
            home.join(".cargo/bin"),
            home.join(".npm-global/bin"),
        ]);
        for dir in &dirs {
            for name in crate::platform::codex_program_names() {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
        if let Ok(extensions) = std::fs::read_dir(home.join(".vscode/extensions")) {
            let mut extensions = extensions
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("openai.chatgpt-"))
                })
                .collect::<Vec<_>>();
            extensions.sort();
            extensions.reverse();
            for extension in extensions {
                for relative in crate::platform::codex_extension_locations() {
                    let candidate = extension.join(relative);
                    if candidate.is_file() {
                        return Ok(candidate);
                    }
                }
            }
        }
    }
    for dir in dirs {
        for name in crate::platform::codex_program_names() {
            let path = dir.join(name);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    Err("Code questions need the Codex CLI on this computer. Install and sign in to Codex, then retry setup. An open Codex conversation alone does not configure Herdr.".into())
}

pub(crate) struct AnswerOutput {
    directory: PathBuf,
    file: PathBuf,
}
impl AnswerOutput {
    pub(crate) fn new() -> Result<Self, String> {
        let directory =
            std::env::temp_dir().join(format!("herdr-answer-{}", super::team::generate_token()));
        crate::platform::create_remote_private_dir(&directory)
            .map_err(|_| "Could not create private answer storage".to_string())?;
        let file = directory.join("answer.txt");
        Ok(Self { directory, file })
    }
    pub(crate) fn path(&self) -> &Path {
        &self.file
    }
    pub(crate) fn read(&self, limit: usize) -> Result<String, String> {
        let file = std::fs::File::open(&self.file).map_err(|_| {
            "The assistant did not return a final answer. Check Codex sign-in and retry."
                .to_string()
        })?;
        let mut bytes = Vec::new();
        file.take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read the assistant answer".to_string())?;
        let truncated = bytes.len() > limit;
        bytes.truncate(limit);
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        if truncated {
            text.push_str("\n[Answer reached the output limit]");
        }
        if text.trim().is_empty() {
            return Err(
                "The assistant returned an empty answer. Check Codex sign-in and retry.".into(),
            );
        }
        Ok(text)
    }
}
impl Drop for AnswerOutput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

pub(crate) fn args(repository: &Path, question: &str, output: &Path) -> Vec<String> {
    vec!["exec".into(), "--sandbox".into(), "read-only".into(), "--cd".into(), repository.to_string_lossy().into_owned(),
        "--ephemeral".into(), "--skip-git-repo-check".into(), "--output-last-message".into(), output.to_string_lossy().into_owned(),
        format!("Answer the question using this approved local project. Do not modify files. Keep the answer concise and do not include secrets.\n\nQuestion: {question}")]
}

pub(crate) fn answer(
    repository: &Path,
    repository_id: &str,
    question: &str,
) -> Result<String, String> {
    if question.trim().is_empty() || question.len() > 16 * 1024 {
        return Err("Enter a question of at most 16 KiB".into());
    }
    let program = resolve_codex()?;
    let output = AnswerOutput::new()?;
    let result = agent_runner::run_agent(
        AgentRequest {
            request_id: format!("local-answer-{}", super::team::generate_token()),
            repository_id: repository_id.into(),
            program,
            args: args(repository, question, output.path()),
            working_directory: Some(repository.into()),
            timeout: Duration::from_secs(300),
            output_limit_bytes: 64 * 1024,
        },
        &CancellationToken::default(),
    )
    .map_err(|_| "Could not start Codex. Check the assistant location and sign-in.".to_string())?;
    if result.state != AgentRunState::Completed {
        return Err(format!(
            "The coding assistant {:?}. Check Codex sign-in or retry the question.",
            result.state
        ));
    }
    output.read(64 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_answer_capture_is_bounded_and_not_the_process_log() {
        let output = AnswerOutput::new().unwrap();
        std::fs::write(output.path(), "abcdefghijk").unwrap();
        let text = output.read(5).unwrap();
        assert!(text.starts_with("abcde\n"));
        assert!(!text.contains("fghijk"));
        assert!(text.contains("output limit"));
    }
    #[test]
    fn missing_final_answer_is_an_actionable_error() {
        let output = AnswerOutput::new().unwrap();
        assert!(output.read(64).unwrap_err().contains("sign-in"));
    }
    #[test]
    fn native_arguments_preserve_readonly_and_allow_approved_non_git_folders() {
        let a = args(
            Path::new("approved folder"),
            "what does this do?",
            Path::new("answer.txt"),
        );
        assert!(a.windows(2).any(|w| w == ["--sandbox", "read-only"]));
        assert!(a.iter().any(|s| s == "--skip-git-repo-check"));
        assert!(a.iter().any(|s| s == "--output-last-message"));
        assert!(!a.iter().any(|s| s.contains("bypass")));
    }
}
