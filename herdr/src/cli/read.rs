use std::path::PathBuf;

use crate::team_storage::TeamStore;

const READ_USAGE: &str = "usage: herdr read --db PATH --repository-id ID --question TEXT";

pub(crate) fn run_read_command(args: &[String]) -> std::io::Result<i32> {
    let mut database = None;
    let mut repository_id = None;
    let mut question = None;
    let mut index = 0;
    while index < args.len() {
        let Some(value) = args.get(index + 1) else {
            if args[index] == "--help" || args[index] == "-h" {
                println!("{READ_USAGE}\n\nStarts a separate Codex CLI session in --sandbox read-only against a locally approved repository. The query and needed repository context go to Codex for its answer; no repository data goes through the team relay.");
                return Ok(0);
            }
            eprintln!("read: missing value for {}", args[index]);
            eprintln!("{READ_USAGE}");
            return Ok(2);
        };
        match args[index].as_str() {
            "--db" => database = Some(PathBuf::from(value)),
            "--repository-id" => repository_id = Some(value.clone()),
            "--question" | "-q" => question = Some(value.clone()),
            flag => {
                eprintln!("read: unknown option: {flag}");
                eprintln!("{READ_USAGE}");
                return Ok(2);
            }
        }
        index += 2;
    }
    let (Some(database), Some(repository_id), Some(question)) = (database, repository_id, question)
    else {
        eprintln!("read: --db, --repository-id, and --question are required");
        eprintln!("{READ_USAGE}");
        return Ok(2);
    };
    if question.trim().is_empty() || question.len() > 16 * 1024 {
        eprintln!("read: question must be between 1 and 16384 bytes");
        return Ok(2);
    }

    let store = TeamStore::open_read_only(&database).map_err(|error| {
        std::io::Error::other(format!(
            "could not open approved-repository database: {error}"
        ))
    })?;
    let approval = store.repository_approval(&repository_id).map_err(|error| {
        std::io::Error::other(format!("could not check repository approval: {error}"))
    })?;
    let Some(approval) = approval else {
        eprintln!("read: repository is not approved in this local profile: {repository_id}");
        return Ok(1);
    };
    let repository = std::fs::canonicalize(&approval.local_path).map_err(|_| {
        std::io::Error::other("approved repository path is unavailable on this device")
    })?;
    if !repository.is_dir() {
        eprintln!("read: approved repository path is not a directory");
        return Ok(1);
    }

    match answer_local(&database, &repository_id, &question) {
        Ok(answer) => {
            print!("{answer}");
            Ok(0)
        }
        Err(error) => {
            eprintln!("read: {error}");
            Ok(1)
        }
    }
}

pub(crate) fn answer_local(
    database: &std::path::Path,
    repository_id: &str,
    question: &str,
) -> Result<String, String> {
    let store = TeamStore::open_read_only(database)
        .map_err(|_| "Set up your local profile first".to_string())?;
    let approval = store
        .repository_approval(repository_id)
        .map_err(|e| e.to_string())?
        .ok_or("Connect and approve a project folder first")?;
    let repository = std::fs::canonicalize(&approval.local_path)
        .map_err(|_| "This project folder is unavailable on this computer".to_string())?;
    if !repository.is_dir() {
        return Err("The approved project is not a directory".into());
    }
    super::assistant::answer(&repository, repository_id, question)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn read_refuses_unapproved_repository_without_starting_codex() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let database = std::env::temp_dir().join(format!("herdr-read-{stamp}.sqlite"));
        let _store = TeamStore::open(&database).unwrap();
        let args = vec![
            "--db".into(),
            database.to_string_lossy().into_owned(),
            "--repository-id".into(),
            "repo-not-approved".into(),
            "--question".into(),
            "What does this project do?".into(),
        ];
        assert_eq!(run_read_command(&args).unwrap(), 1);
        let _ = std::fs::remove_file(database);
    }
}
