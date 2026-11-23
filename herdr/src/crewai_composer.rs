//! Editing and completion state for the conversation composer.
use crate::team::Member;

const MAX_INPUT: usize = 16 * 1024;
const COMMANDS: [(&str, &str); 5] = [
    ("/read", "Ask about a project"),
    ("/team-mates", "Manage your team"),
    ("/task", "Assign work"),
    ("/broadcast", "Message the team"),
    ("/message", "Message one teammate"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Suggestion {
    pub label: String,
    pub description: String,
    pub replacement: String,
    pub member_id: Option<String>,
    pub member_name: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Picker {
    pub start: usize,
    pub end: usize,
    pub options: Vec<Suggestion>,
    pub selected: usize,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Composer {
    pub text: String,
    pub cursor: usize,
    pub picker_index: usize,
    pub selected_member: Option<(String, String, String)>, // id, label, exact mention
    pub picker_suppressed: bool,
}

impl Composer {
    pub(crate) fn from_text(text: String) -> Self {
        let cursor = text.len();
        Self {
            text,
            cursor,
            ..Self::default()
        }
    }
    fn after_edit(&mut self) {
        self.picker_index = 0;
        self.picker_suppressed = false;
        if self
            .selected_member
            .as_ref()
            .is_some_and(|(_, _, token)| !contains_mention(&self.text, token))
        {
            self.selected_member = None;
        }
    }
    pub(crate) fn insert(&mut self, value: &str) -> bool {
        if self.text.len() + value.len() > MAX_INPUT
            || value
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return false;
        }
        self.text.insert_str(self.cursor, value);
        self.cursor += value.len();
        self.after_edit();
        true
    }
    pub(crate) fn backspace(&mut self) {
        if let Some((index, _)) = self.text[..self.cursor].char_indices().next_back() {
            self.text.drain(index..self.cursor);
            self.cursor = index;
            self.after_edit();
        }
    }
    pub(crate) fn delete(&mut self) {
        if let Some(ch) = self.text[self.cursor..].chars().next() {
            self.text.drain(self.cursor..self.cursor + ch.len_utf8());
            self.after_edit();
        }
    }
    pub(crate) fn left(&mut self) {
        if let Some((index, _)) = self.text[..self.cursor].char_indices().next_back() {
            self.cursor = index;
            self.picker_index = 0;
        }
    }
    pub(crate) fn right(&mut self) {
        if let Some(ch) = self.text[self.cursor..].chars().next() {
            self.cursor += ch.len_utf8();
            self.picker_index = 0;
        }
    }
    pub(crate) fn home(&mut self) {
        self.cursor = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
        self.picker_index = 0;
    }
    pub(crate) fn end(&mut self) {
        self.cursor = self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |i| self.cursor + i);
        self.picker_index = 0;
    }
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn picker(&self, members: &[Member]) -> Option<Picker> {
        if self.picker_suppressed {
            return None;
        }
        let before = &self.text[..self.cursor];
        if before.starts_with('/') && !before.chars().any(char::is_whitespace) {
            let token_end = self
                .text
                .find(char::is_whitespace)
                .unwrap_or(self.text.len());
            let options = COMMANDS
                .iter()
                .filter(|(name, _)| name.starts_with(before))
                .map(|(name, description)| Suggestion {
                    label: (*name).into(),
                    description: (*description).into(),
                    replacement: (*name).into(),
                    member_id: None,
                    member_name: None,
                })
                .collect::<Vec<_>>();
            if !options.is_empty() {
                return Some(Picker {
                    start: 0,
                    end: token_end,
                    selected: self.picker_index.min(options.len() - 1),
                    options,
                });
            }
        }
        let start = before.rfind('@')?;
        if start > 0 && !before[..start].chars().next_back()?.is_whitespace() {
            return None;
        }
        let query = &before[start + 1..];
        if query.contains(char::is_whitespace) || query.contains('"') {
            return None;
        }
        let end = self.text[start + 1..]
            .find(char::is_whitespace)
            .map_or(self.text.len(), |i| start + 1 + i);
        let options = members
            .iter()
            .filter(|member| {
                member.active
                    && member
                        .display_name
                        .to_lowercase()
                        .contains(&query.to_lowercase())
            })
            .map(|member| member_suggestion(member, members))
            .collect::<Vec<_>>();
        if options.is_empty() {
            return None;
        }
        Some(Picker {
            start,
            end,
            selected: self.picker_index.min(options.len() - 1),
            options,
        })
    }

    /// Open an explicit choice when a typed name is missing or ambiguous.
    pub(crate) fn target_picker(&self, name: &str, members: &[Member]) -> Option<Picker> {
        let quoted = format!("@{}", serde_json::to_string(name).ok()?);
        let plain = format!("@{name}");
        let (start, end) = [quoted.as_str(), plain.as_str()].iter().find_map(|token| {
            self.text.match_indices(token).find_map(|(at, _)| {
                let end = at + token.len();
                let left = at == 0
                    || self.text[..at]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_whitespace);
                let right = end == self.text.len()
                    || self.text[end..]
                        .chars()
                        .next()
                        .is_some_and(char::is_whitespace);
                (left && right).then_some((at, end))
            })
        })?;
        let matching = members
            .iter()
            .filter(|member| member.active && member.display_name.eq_ignore_ascii_case(name))
            .collect::<Vec<_>>();
        let choices = if matching.is_empty() {
            members
                .iter()
                .filter(|member| member.active)
                .collect::<Vec<_>>()
        } else {
            matching
        };
        let options = choices
            .into_iter()
            .map(|member| member_suggestion(member, members))
            .collect::<Vec<_>>();
        if options.is_empty() {
            return None;
        }
        Some(Picker {
            start,
            end,
            selected: 0,
            options,
        })
    }

    pub(crate) fn choose(&mut self, picker: &Picker, index: usize) -> bool {
        let Some(option) = picker.options.get(index) else {
            return false;
        };
        if picker.start > picker.end
            || picker.end > self.text.len()
            || !self.text.is_char_boundary(picker.start)
            || !self.text.is_char_boundary(picker.end)
        {
            return false;
        }
        let mut replacement = option.replacement.clone();
        if picker.end == self.text.len() {
            replacement.push(' ');
        }
        self.text
            .replace_range(picker.start..picker.end, &replacement);
        self.cursor = picker.start + replacement.len();
        self.selected_member = option
            .member_id
            .as_ref()
            .zip(option.member_name.as_ref())
            .map(|(id, name)| (id.clone(), name.clone(), option.replacement.clone()));
        self.picker_index = 0;
        self.picker_suppressed = true;
        true
    }
}

fn member_suggestion(member: &Member, all: &[Member]) -> Suggestion {
    let replacement = if member.display_name.chars().any(char::is_whitespace)
        || member.display_name.contains(['"', '\\'])
    {
        format!(
            "@{}",
            serde_json::to_string(&member.display_name).expect("name serializes")
        )
    } else {
        format!("@{}", member.display_name)
    };
    let duplicate = all
        .iter()
        .filter(|candidate| {
            candidate.active
                && candidate
                    .display_name
                    .eq_ignore_ascii_case(&member.display_name)
        })
        .count()
        > 1;
    let description = if duplicate {
        format!(
            "{:?} · {}",
            member.role,
            member
                .member_id
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        )
    } else {
        format!("{:?}", member.role)
    };
    Suggestion {
        label: format!("@{}", member.display_name),
        description,
        replacement,
        member_id: Some(member.member_id.clone()),
        member_name: Some(member.display_name.clone()),
    }
}

fn contains_mention(text: &str, token: &str) -> bool {
    text.match_indices(token).any(|(at, _)| {
        (at == 0
            || text[..at]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace))
            && (at + token.len() == text.len()
                || text[at + token.len()..]
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::Role;
    #[test]
    fn edits_unicode_and_preserves_suffix_when_selecting_teammates() {
        let members = vec![Member {
            member_id: "one".into(),
            display_name: "Hemant Joshi".into(),
            role: Role::Implementer,
            active: true,
        }];
        let mut input = Composer::from_text("/message @Hem status".into());
        input.cursor = "/message @Hem".len();
        let picker = input.picker(&members).unwrap();
        assert!(input.choose(&picker, 0));
        assert_eq!(input.text, "/message @\"Hemant Joshi\" status");
        assert_eq!(input.selected_member.as_ref().unwrap().0, "one");
        input.end();
        assert!(input.insert(" ✓"));
        input.backspace();
        assert!(input.text.ends_with("status "));
        assert_eq!(input.selected_member.as_ref().unwrap().0, "one");
        input.home();
        input.delete();
        assert!(!input.text.starts_with('/'));
        input.cursor = input.text.find("Hemant").unwrap() + "Hemant".len();
        input.backspace();
        assert!(
            input.selected_member.is_none(),
            "editing the mention discards the old identity"
        );
    }
    #[test]
    fn command_picker_and_input_limits() {
        let mut input = Composer::from_text("/bro".into());
        let picker = input.picker(&[]).unwrap();
        assert_eq!(picker.options[0].label, "/broadcast");
        assert!(input.choose(&picker, 0));
        assert_eq!(input.text, "/broadcast ");
        assert!(!input.insert(&"x".repeat(MAX_INPUT)));
        assert!(!input.insert("\0"));
    }

    #[test]
    fn ambiguous_names_offer_a_stable_member_choice_without_losing_text() {
        let member = |id: &str| Member {
            member_id: id.into(),
            display_name: "Hemant".into(),
            role: Role::Implementer,
            active: true,
        };
        let members = vec![member("one"), member("two")];
        let mut composer = Composer::from_text("/task @Hemant finish this".into());
        let picker = composer.target_picker("Hemant", &members).unwrap();
        assert_eq!(picker.options.len(), 2);
        assert!(composer.choose(&picker, 1));
        assert_eq!(composer.text, "/task @Hemant finish this");
        assert_eq!(composer.selected_member.as_ref().unwrap().0, "two");
    }
}
