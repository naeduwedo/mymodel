//! Chat state, kept free of GTK so it can be tested without a display.
//!
//! The same rules the Windows shell enforces, expressed once here:
//!
//! * an empty prompt is refused before it reaches the core, which would
//!   otherwise load a model to answer nothing;
//! * the pending prompt is added to the request but not to the transcript, so a
//!   failed call does not leave the history claiming a turn that never happened;
//! * an empty delta is not a token and must not be counted.

use gguf_engine_protocol::pb;

/// Why a chat request could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChatError {
    /// No model chosen.
    #[error("choose a model first")]
    NoModel,
    /// The prompt is blank.
    #[error("the prompt must not be empty")]
    EmptyPrompt,
}

/// The conversation, and the request built from it.
#[derive(Debug, Default)]
pub struct Transcript {
    messages: Vec<pb::ChatMessage>,
    system: Option<String>,
}

impl Transcript {
    /// An empty transcript.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the system prompt applied to every request.
    ///
    /// Not wired to a widget yet - the chat page has no system-prompt field -
    /// but the rule it enforces (a blank prompt is not a system prompt) is
    /// tested, and the shell needs it the moment that field exists.
    #[allow(dead_code)]
    pub fn set_system(&mut self, system: Option<String>) {
        self.system = system.filter(|text| !text.trim().is_empty());
    }

    /// Messages so far.
    pub fn messages(&self) -> &[pb::ChatMessage] {
        &self.messages
    }

    /// Append a user turn.
    pub fn add_user(&mut self, prompt: &str) -> Result<(), ChatError> {
        if prompt.trim().is_empty() {
            return Err(ChatError::EmptyPrompt);
        }
        self.messages.push(pb::ChatMessage {
            role: "user".to_owned(),
            content: prompt.to_owned(),
        });
        Ok(())
    }

    /// Append an assistant turn, including a partial one after a stop.
    pub fn add_assistant(&mut self, content: &str) {
        self.messages.push(pb::ChatMessage {
            role: "assistant".to_owned(),
            content: content.to_owned(),
        });
    }

    /// Forget the conversation.
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Build a request: system prompt, then history, then the pending prompt.
    pub fn build_request(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<pb::ChatRequest, ChatError> {
        if model.trim().is_empty() {
            return Err(ChatError::NoModel);
        }
        if prompt.trim().is_empty() {
            return Err(ChatError::EmptyPrompt);
        }

        let mut request = pb::ChatRequest {
            model_id: model.to_owned(),
            messages: Vec::with_capacity(self.messages.len() + 2),
            task_id: None,
            max_tokens: Some(max_tokens),
            temperature: Some(temperature),
            stream: Some(true),
        };

        if let Some(system) = &self.system {
            request.messages.push(pb::ChatMessage {
                role: "system".to_owned(),
                content: system.clone(),
            });
        }

        request.messages.extend(self.messages.iter().cloned());
        request.messages.push(pb::ChatMessage {
            role: "user".to_owned(),
            content: prompt.to_owned(),
        });

        Ok(request)
    }
}

/// Running totals for one streamed answer.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SessionStats {
    /// Non-empty deltas received.
    pub chunks: u32,
    /// Whether the stream reached its done chunk.
    pub completed: bool,
    /// Whether the user stopped it.
    pub cancelled: bool,
}

impl SessionStats {
    /// Record one delta, returning whether it counted.
    ///
    /// An empty delta is the runtime saying nothing, not a token; counting it
    /// would make the reported token count wrong.
    pub fn record_delta(&mut self, delta: &str) -> bool {
        if delta.is_empty() {
            return false;
        }
        self.chunks += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_prompt_is_refused_before_it_reaches_the_core() {
        let transcript = Transcript::new();
        assert_eq!(
            transcript.build_request("m", "  ", 16, 0.7),
            Err(ChatError::EmptyPrompt)
        );
    }

    #[test]
    fn a_missing_model_is_reported_as_choose_a_model() {
        let transcript = Transcript::new();
        assert_eq!(
            transcript.build_request("", "hi", 16, 0.7),
            Err(ChatError::NoModel)
        );
    }

    #[test]
    fn the_request_is_system_then_history_then_the_new_prompt() {
        let mut transcript = Transcript::new();
        transcript.set_system(Some("be brief".to_owned()));
        transcript.add_user("first").unwrap();
        transcript.add_assistant("first answer");

        let request = transcript.build_request("m", "second", 64, 0.2).unwrap();

        assert_eq!(request.model_id, "m");
        assert_eq!(request.max_tokens, Some(64));
        assert_eq!(request.stream, Some(true));
        assert_eq!(request.messages.len(), 4);
        assert_eq!(request.messages[0].role, "system");
        assert_eq!(request.messages[3].content, "second");
    }

    #[test]
    fn building_a_request_does_not_append_the_prompt() {
        let mut transcript = Transcript::new();
        transcript.add_user("stored").unwrap();
        let _ = transcript.build_request("m", "pending", 16, 0.7);
        assert_eq!(transcript.messages().len(), 1);
    }

    #[test]
    fn an_empty_system_prompt_is_ignored() {
        let mut transcript = Transcript::new();
        transcript.set_system(Some("   ".to_owned()));
        let request = transcript.build_request("m", "hi", 16, 0.7).unwrap();
        assert_eq!(request.messages.len(), 1, "only the user turn");
    }

    #[test]
    fn an_empty_delta_is_not_a_token() {
        let mut stats = SessionStats::default();
        assert!(!stats.record_delta(""));
        assert!(stats.record_delta("real"));
        assert_eq!(stats.chunks, 1);
    }
}