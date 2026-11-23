//! Conversation domain types, independent of persistence and rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConversationEntry {
    pub team_id: String,
    pub thread_id: String,
    /// Stable local ID or a namespaced event, task, request or message ID.
    pub entry_id: String,
    /// Stable ID of the request, task or direct message this entry follows.
    pub related_id: Option<String>,
    pub kind: ConversationKind,
    pub speaker_id: Option<String>,
    pub recipient_id: Option<String>,
    pub text: String,
    pub status: String,
    pub created_at: u64,
    pub updated_at: u64,
}

impl ConversationEntry {
    /// Answers and approval entries share a request ID even when other questions arrive.
    pub(crate) fn related_request_id(&self) -> Option<&str> {
        self.entry_id
            .strip_prefix("question:")
            .or_else(|| self.entry_id.strip_prefix("answer:"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConversationKind {
    Command,
    Answer,
    Message,
    Broadcast,
    Task,
    Approval,
    Progress,
    Failure,
}

impl ConversationKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Answer => "answer",
            Self::Message => "message",
            Self::Broadcast => "broadcast",
            Self::Task => "task",
            Self::Approval => "approval",
            Self::Progress => "progress",
            Self::Failure => "failure",
        }
    }
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "command" => Self::Command,
            "answer" => Self::Answer,
            "message" => Self::Message,
            "broadcast" => Self::Broadcast,
            "task" => Self::Task,
            "approval" => Self::Approval,
            "progress" => Self::Progress,
            "failure" => Self::Failure,
            _ => return None,
        })
    }
}

/// Reconstruct team-visible history from verified projections and signed events.
/// Project questions and direct messages are shown only to their participants.
pub(crate) fn reconstruct(
    team: &str,
    local_member: &str,
    events: &[crate::team::Event],
    questions: &[crate::cli::questions::Question],
    messages: &[(crate::crewai_message::DirectMessage, String)],
) -> Vec<ConversationEntry> {
    use crate::team::EventType;
    let mut entries = Vec::new();
    let mut request_times = std::collections::HashMap::new();
    let tasks = events
        .iter()
        .filter(|e| e.team_id == team && e.event_type == EventType::TaskCreated)
        .filter_map(|e| {
            Some((
                e.payload.get("task_id")?.as_str()?.to_owned(),
                (
                    e.payload.get("description")?.as_str()?.to_owned(),
                    e.payload.get("assignee_member_id")?.as_str()?.to_owned(),
                ),
            ))
        })
        .collect::<std::collections::HashMap<_, _>>();
    for event in events.iter().filter(|e| e.team_id == team) {
        let value = |key: &str| event.payload.get(key).and_then(|v| v.as_str());
        if event.event_type == EventType::RequestCreated {
            if let Some(id) = value("request_id") {
                request_times.insert(id.to_owned(), event.logical_time);
            }
            continue;
        }
        if event.event_type == EventType::TaskStatusChanged {
            if let (Some((description, recipient)), Some(status)) = (
                value("task_id").and_then(|id| tasks.get(id)),
                value("status"),
            ) {
                entries.push(ConversationEntry {
                    team_id: team.into(),
                    thread_id: "main".into(),
                    entry_id: format!("event:{}", event.event_id),
                    related_id: value("task_id").map(|id| format!("task:{id}")),
                    kind: ConversationKind::Task,
                    speaker_id: Some(event.actor_member_id.clone()),
                    recipient_id: Some(recipient.clone()),
                    text: description.clone(),
                    status: status.into(),
                    created_at: event.logical_time,
                    updated_at: event.logical_time,
                });
            }
            continue;
        }
        let (kind, text, recipient) = match event.event_type {
            EventType::BroadcastCreated if value("scope") == Some("team") => {
                (ConversationKind::Broadcast, value("message"), None)
            }
            EventType::TaskCreated => (
                ConversationKind::Task,
                value("description"),
                value("assignee_member_id"),
            ),
            EventType::TaskFollowUpAdded => (ConversationKind::Task, value("message"), None),
            _ => continue,
        };
        if let Some(text) = text {
            entries.push(ConversationEntry {
                team_id: team.into(),
                thread_id: "main".into(),
                entry_id: format!("event:{}", event.event_id),
                related_id: value("task_id").map(|id| format!("task:{id}")),
                kind,
                speaker_id: Some(event.actor_member_id.clone()),
                recipient_id: recipient.map(str::to_owned),
                text: text.into(),
                status: "recorded".into(),
                created_at: event.logical_time,
                updated_at: event.logical_time,
            });
        }
    }
    for question in questions {
        let Some(&at) = request_times.get(&question.request_id) else {
            continue;
        };
        if question.requester_member_id != local_member && question.target_member_id != local_member
        {
            continue;
        }
        entries.push(ConversationEntry {
            team_id: team.into(),
            thread_id: "main".into(),
            entry_id: format!("question:{}", question.request_id),
            related_id: Some(format!("question:{}", question.request_id)),
            kind: ConversationKind::Approval,
            speaker_id: Some(question.requester_member_id.clone()),
            recipient_id: Some(question.target_member_id.clone()),
            text: question.question.clone(),
            status: question.delivery.clone(),
            created_at: at,
            updated_at: at,
        });
        if let Some(answer) = &question.result {
            entries.push(ConversationEntry {
                team_id: team.into(),
                thread_id: "main".into(),
                entry_id: format!("answer:{}", question.request_id),
                related_id: Some(format!("question:{}", question.request_id)),
                kind: if question.delivery == "Failed" {
                    ConversationKind::Failure
                } else {
                    ConversationKind::Answer
                },
                speaker_id: Some(question.target_member_id.clone()),
                recipient_id: Some(question.requester_member_id.clone()),
                text: answer.clone(),
                status: question.delivery.clone(),
                created_at: at,
                updated_at: at,
            });
        }
    }
    for (message, state) in messages {
        if message.team_id != team
            || (message.sender_member_id != local_member
                && message.recipient_member_id != local_member)
        {
            continue;
        }
        entries.push(ConversationEntry {
            team_id: team.into(),
            thread_id: "main".into(),
            entry_id: format!("message:{}", message.message_id),
            related_id: message.reply_to.as_ref().map(|id| format!("message:{id}")),
            kind: ConversationKind::Message,
            speaker_id: Some(message.sender_member_id.clone()),
            recipient_id: Some(message.recipient_member_id.clone()),
            text: message.body.clone(),
            status: state.clone(),
            created_at: message.created_at,
            updated_at: message.created_at,
        });
    }
    sort_entries(&mut entries);
    let mut seen = std::collections::HashSet::new();
    entries.retain(|entry| seen.insert(entry.entry_id.clone()));
    if entries.len() > 500 {
        entries.drain(..entries.len() - 500);
    }
    entries
}

pub(crate) fn sort_entries(entries: &mut [ConversationEntry]) {
    let rank = |entry: &ConversationEntry| {
        if entry.kind == ConversationKind::Answer || entry.kind == ConversationKind::Failure {
            1
        } else {
            0
        }
    };
    entries.sort_by(|a, b| {
        (a.created_at, rank(a), &a.entry_id).cmp(&(b.created_at, rank(b), &b.entry_id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{Event, EventType};
    use serde_json::json;

    #[test]
    fn reconstruction_keeps_questions_bound_to_team_and_participants() {
        let event = |team: &str, id: &str, request: &str, at| {
            Event::create(
                team,
                "alice",
                "device",
                EventType::RequestCreated,
                json!({"request_id":request}),
                at,
                id,
            )
            .unwrap()
        };
        let events = vec![
            event("a", "event-a", "q1", 1),
            event("a", "event-b", "q2", 2),
            event("b", "event-c", "q3", 3),
        ];
        let question =
            |id: &str, status: &str, result: Option<&str>| crate::cli::questions::Question {
                request_id: id.into(),
                requester_member_id: "alice".into(),
                target_member_id: "bob".into(),
                repository_id: "repo".into(),
                question: format!("Question {id}"),
                delivery: status.into(),
                result: result.map(str::to_owned),
                consent: String::new(),
            };
        let questions = vec![
            question("q1", "Completed", Some("Answer one")),
            question("q2", "Queued", None),
            question("q3", "Completed", Some("Other team")),
        ];
        let entries = reconstruct("a", "bob", &events, &questions, &[]);
        assert_eq!(
            entries
                .iter()
                .map(|e| e.entry_id.as_str())
                .collect::<Vec<_>>(),
            vec!["question:q1", "answer:q1", "question:q2"]
        );
        assert_eq!(entries[1].text, "Answer one");
        assert_eq!(entries[1].related_id.as_deref(), Some("question:q1"));
        assert_eq!(entries[2].status, "Queued");
        assert!(reconstruct("a", "charlie", &events, &questions, &[]).is_empty());
        assert!(!entries.iter().any(|e| e.text == "Other team"));
        let broadcast = Event::create(
            "a",
            "alice",
            "device",
            EventType::BroadcastCreated,
            json!({"scope":"team","message":"Hello"}),
            4,
            "signature",
        )
        .unwrap();
        let repeated = reconstruct("a", "bob", &[broadcast.clone(), broadcast], &[], &[]);
        assert_eq!(repeated.len(), 1);
    }
}
