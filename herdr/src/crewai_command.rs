//! Parsing for the conversational composer; does not perform operations.
use crate::team::Member;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Read {
        question: String,
        target: Option<String>,
    },
    TeamMates,
    Task {
        target: String,
        description: String,
    },
    Broadcast {
        message: String,
    },
    Message {
        target: String,
        message: String,
    },
}

pub(crate) fn parse(input: &str) -> Result<Command, &'static str> {
    if input.len() > 16 * 1024
        || input
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r'))
    {
        return Err("Input is invalid or too long");
    }
    let trimmed = input.trim();
    let split = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let (name, rest) = trimmed.split_at(split);
    let rest = rest.trim();
    if name == "/team-mates" {
        return if rest.is_empty() {
            Ok(Command::TeamMates)
        } else {
            Err("/team-mates takes no text")
        };
    }
    if !matches!(name, "/read" | "/task" | "/broadcast" | "/message") {
        return Err("Choose a supported slash command");
    }
    if rest.is_empty() {
        return Err("Enter a question or message");
    }
    if name == "/broadcast" {
        return Ok(Command::Broadcast {
            message: rest.into(),
        });
    }
    let mentions: Vec<_> = rest
        .match_indices('@')
        .filter(|(i, _)| {
            *i == 0
                || rest[..*i]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace)
        })
        .collect();
    if mentions.len() > 1 {
        return Err("Choose one teammate");
    }
    let (target, body) = if let Some((start, _)) = mentions.first() {
        let start = *start;
        let (target, end) = if rest[start + 1..].starts_with('"') {
            let mut decoded =
                serde_json::Deserializer::from_str(&rest[start + 1..]).into_iter::<String>();
            let name = decoded
                .next()
                .ok_or("Choose a teammate")?
                .map_err(|_| "Finish the teammate name")?;
            (name, start + 1 + decoded.byte_offset())
        } else {
            let end = rest[start + 1..]
                .find(char::is_whitespace)
                .map(|n| start + 1 + n)
                .unwrap_or(rest.len());
            (rest[start + 1..end].to_owned(), end)
        };
        if target.is_empty() {
            return Err("Choose a teammate");
        }
        let body = if start == 0 {
            rest[end..].trim_start().to_owned()
        } else if end == rest.len() {
            rest[..start].trim_end().to_owned()
        } else {
            format!("{}{}", &rest[..start], rest[end..].trim_start())
        };
        (Some(target), body)
    } else {
        (None, rest.to_owned())
    };
    if body.is_empty() {
        return Err("Enter a question or message");
    }
    match name {
        "/read" => Ok(Command::Read {
            question: body,
            target,
        }),
        "/task" => Ok(Command::Task {
            target: target.ok_or("Choose a teammate")?,
            description: body,
        }),
        "/message" => Ok(Command::Message {
            target: target.ok_or("Choose a teammate")?,
            message: body,
        }),
        _ => unreachable!(),
    }
}

pub(crate) fn resolve_member<'a>(
    name: &str,
    members: &'a [Member],
) -> Result<&'a Member, &'static str> {
    let mut matches = members
        .iter()
        .filter(|m| m.active && m.display_name.eq_ignore_ascii_case(name));
    let member = matches.next().ok_or("Choose an approved teammate")?;
    if matches.next().is_some() {
        return Err("Choose a teammate from the list");
    }
    Ok(member)
}

/// Resolve the picker's stored selection by identity, never by its display label.
pub(crate) fn resolve_selected_member<'a>(
    selected_id: &str,
    members: &'a [Member],
) -> Result<&'a Member, &'static str> {
    members
        .iter()
        .find(|m| m.active && m.member_id == selected_id)
        .ok_or("Choose an approved teammate")
}

/// Describe what the composer must do before dispatching a recipient action.
/// The UI owns the picker; the parser never guesses between duplicate labels.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RecipientChoice {
    Resolved(String),
    Pick(Vec<(String, String)>),
}

pub(crate) fn choose_recipient(name: Option<&str>, members: &[Member]) -> RecipientChoice {
    let eligible: Vec<_> = members.iter().filter(|m| m.active).collect();
    let matching: Vec<_> = eligible
        .iter()
        .copied()
        .filter(|m| name.is_some_and(|n| m.display_name.eq_ignore_ascii_case(n)))
        .collect();
    if matching.len() == 1 {
        return RecipientChoice::Resolved(matching[0].member_id.clone());
    }
    let options = if matching.is_empty() {
        eligible
    } else {
        matching
    };
    RecipientChoice::Pick(
        options
            .into_iter()
            .map(|m| (m.member_id.clone(), m.display_name.clone()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_real_commands_and_preserves_body() {
        assert_eq!(
            parse("/read what is this project about @hemant"),
            Ok(Command::Read {
                question: "what is this project about".into(),
                target: Some("hemant".into())
            })
        );
        assert_eq!(
            parse("/read @hemant explain\nthis"),
            Ok(Command::Read {
                question: "explain\nthis".into(),
                target: Some("hemant".into())
            })
        );
        assert_eq!(
            parse("/message @hemant check https://example.org/a"),
            Ok(Command::Message {
                target: "hemant".into(),
                message: "check https://example.org/a".into()
            })
        );
        assert!(matches!(
            parse("/task @hemant fix it"),
            Ok(Command::Task { .. })
        ));
        assert_eq!(
            parse("/read explain this @\"Hemant Joshi\""),
            Ok(Command::Read {
                question: "explain this".into(),
                target: Some("Hemant Joshi".into())
            })
        );
        assert_eq!(
            parse("/message @\"Hemant Joshi\" hello there"),
            Ok(Command::Message {
                target: "Hemant Joshi".into(),
                message: "hello there".into()
            })
        );
        assert!(matches!(
            parse("/broadcast hello team"),
            Ok(Command::Broadcast { .. })
        ));
        assert_eq!(parse("/team-mates"), Ok(Command::TeamMates));
    }
    #[test]
    fn invalid_input_never_dispatches() {
        for v in [
            "",
            "/unknown a",
            "/message hello",
            "/task @x",
            "/read @",
            "/read @x @y hello",
            "/broadcast \0",
        ] {
            assert!(parse(v).is_err(), "{v:?}");
        }
        assert!(parse(&format!("/read {}", "x".repeat(16384))).is_err());
    }
    #[test]
    fn names_are_not_authentication_and_duplicates_are_ambiguous() {
        let m = Member {
            member_id: "a".into(),
            display_name: "Hemant".into(),
            role: crate::team::Role::Implementer,
            active: true,
        };
        assert_eq!(
            resolve_member("hemant", &[m.clone()]).unwrap().member_id,
            "a"
        );
        let mut other = m.clone();
        other.member_id = "b".into();
        assert!(
            matches!(choose_recipient(Some("Hemant"), &[m.clone(), other.clone()]), RecipientChoice::Pick(options) if options.len()==2)
        );
        assert_eq!(
            choose_recipient(Some("missing"), &[m.clone()]),
            RecipientChoice::Pick(vec![("a".into(), "Hemant".into())])
        );
        assert_eq!(
            choose_recipient(Some("hemant"), &[m.clone()]),
            RecipientChoice::Resolved("a".into())
        );
        assert!(resolve_member("Hemant", &[m.clone(), other.clone()]).is_err());
        assert_eq!(
            resolve_selected_member("b", &[m, other]).unwrap().member_id,
            "b"
        );
    }
}
