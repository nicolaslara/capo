//! Memory packet scaffolding.
//!
//! P9 adds source-linked memory packets with inclusion/exclusion reasons. The
//! packet is replayable prompt-input evidence; its sources remain authoritative.

use capo_core::{BoundaryBinding, BoundaryKind, MemoryPacketId, SessionId};

/// The first memory backend proves packet provenance without semantic search.
pub const PROTOTYPE_MEMORY_BACKEND: &str = "fake-packet-builder";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryBackend {
    Fake(FakeMemoryBackend),
}

impl MemoryBackend {
    pub fn fake() -> Self {
        Self::Fake(FakeMemoryBackend)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(backend) => backend.binding(),
        }
    }

    pub fn build_packet(&self, request: FakeMemoryPacketRequest) -> FakeMemoryPacket {
        match self {
            Self::Fake(backend) => backend.build_packet(request),
        }
    }

    pub fn build_source_linked_packet(
        &self,
        request: SourceLinkedMemoryPacketRequest,
    ) -> MemoryPacketBuild {
        match self {
            Self::Fake(backend) => backend.build_source_linked_packet(request),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeMemoryBackend;

impl FakeMemoryBackend {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::MemoryBackend, "fake-memory")
    }

    pub fn build_packet(&self, request: FakeMemoryPacketRequest) -> FakeMemoryPacket {
        FakeMemoryPacket {
            memory_packet_id: request.memory_packet_id,
            session_id: request.session_id,
            artifact_id: format!("artifact-memory-{}", request.goal_slug),
            purpose: "turn_context".to_string(),
            source_summary: request.summary,
        }
    }

    pub fn build_source_linked_packet(
        &self,
        request: SourceLinkedMemoryPacketRequest,
    ) -> MemoryPacketBuild {
        let mut included = Vec::new();
        let mut excluded = Vec::new();
        let mut used_budget = 0usize;

        for candidate in request.candidates.clone() {
            if candidate.sensitivity == MemorySensitivity::Secret {
                excluded.push(MemoryPacketDecision {
                    source: candidate.source,
                    title: candidate.title,
                    reason: "excluded: secret or credential material is never packet memory"
                        .to_string(),
                    estimated_tokens: candidate.estimated_tokens,
                });
                continue;
            }

            if candidate.review_state != MemoryReviewState::Reviewed {
                excluded.push(MemoryPacketDecision {
                    source: candidate.source,
                    title: candidate.title,
                    reason: format!("excluded: review_state={}", candidate.review_state.as_str()),
                    estimated_tokens: candidate.estimated_tokens,
                });
                continue;
            }

            if used_budget + candidate.estimated_tokens > request.budget_tokens {
                excluded.push(MemoryPacketDecision {
                    source: candidate.source,
                    title: candidate.title,
                    reason: "excluded: packet budget exhausted".to_string(),
                    estimated_tokens: candidate.estimated_tokens,
                });
                continue;
            }

            used_budget += candidate.estimated_tokens;
            included.push(MemoryPacketIncludedItem {
                source: candidate.source,
                title: candidate.title,
                body: candidate.body,
                inclusion_reason: candidate.inclusion_reason,
                estimated_tokens: candidate.estimated_tokens,
            });
        }

        let packet_markdown = render_packet_markdown(&request, &included, &excluded);
        let explanation_markdown = render_explanation_markdown(&request, &included, &excluded);
        let packet_artifact_id = format!("artifact-memory-packet-{}", request.memory_packet_id);
        let explanation_artifact_id =
            format!("artifact-memory-explanation-{}", request.memory_packet_id);

        MemoryPacketBuild {
            memory_packet_id: request.memory_packet_id,
            session_id: request.session_id,
            turn_id: request.turn_id,
            run_id: request.run_id,
            purpose: request.purpose,
            budget_tokens: request.budget_tokens,
            packet_artifact_id,
            explanation_artifact_id,
            packet_markdown,
            explanation_markdown,
            included,
            excluded,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeMemoryPacketRequest {
    pub memory_packet_id: MemoryPacketId,
    pub session_id: SessionId,
    pub goal_slug: String,
    pub summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeMemoryPacket {
    pub memory_packet_id: MemoryPacketId,
    pub session_id: SessionId,
    pub artifact_id: String,
    pub purpose: String,
    pub source_summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceLinkedMemoryPacketRequest {
    pub memory_packet_id: MemoryPacketId,
    pub session_id: SessionId,
    pub run_id: String,
    pub turn_id: String,
    pub purpose: String,
    pub budget_tokens: usize,
    pub candidates: Vec<MemoryCandidate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryCandidate {
    pub title: String,
    pub body: String,
    pub source: MemorySourceRef,
    pub review_state: MemoryReviewState,
    pub sensitivity: MemorySensitivity,
    pub estimated_tokens: usize,
    pub inclusion_reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemorySourceRef {
    pub source_kind: MemorySourceKind,
    pub source_ref: String,
    pub anchor: Option<String>,
    pub content_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemorySourceKind {
    Event,
    Artifact,
    Markdown,
    ExternalImport,
}

impl MemorySourceKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Event => "event",
            Self::Artifact => "artifact",
            Self::Markdown => "markdown",
            Self::ExternalImport => "external_import",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryReviewState {
    Generated,
    Reviewed,
    Rejected,
    Superseded,
    Invalidated,
}

impl MemoryReviewState {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Generated => "generated",
            Self::Reviewed => "reviewed",
            Self::Rejected => "rejected",
            Self::Superseded => "superseded",
            Self::Invalidated => "invalidated",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemorySensitivity {
    Public,
    Internal,
    Sensitive,
    Secret,
}

impl MemorySensitivity {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Sensitive => "sensitive",
            Self::Secret => "secret",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryPacketIncludedItem {
    pub source: MemorySourceRef,
    pub title: String,
    pub body: String,
    pub inclusion_reason: String,
    pub estimated_tokens: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryPacketDecision {
    pub source: MemorySourceRef,
    pub title: String,
    pub reason: String,
    pub estimated_tokens: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryPacketBuild {
    pub memory_packet_id: MemoryPacketId,
    pub session_id: SessionId,
    pub run_id: String,
    pub turn_id: String,
    pub purpose: String,
    pub budget_tokens: usize,
    pub packet_artifact_id: String,
    pub explanation_artifact_id: String,
    pub packet_markdown: String,
    pub explanation_markdown: String,
    pub included: Vec<MemoryPacketIncludedItem>,
    pub excluded: Vec<MemoryPacketDecision>,
}

fn render_packet_markdown(
    request: &SourceLinkedMemoryPacketRequest,
    included: &[MemoryPacketIncludedItem],
    excluded: &[MemoryPacketDecision],
) -> String {
    let mut markdown = format!(
        "# Memory Packet {}\n\nPurpose: {}\nBudget tokens: {}\nIncluded: {}\nExcluded: {}\n\n",
        request.memory_packet_id,
        request.purpose,
        request.budget_tokens,
        included.len(),
        excluded.len()
    );

    for item in included {
        markdown.push_str(&format!(
            "## {}\n\n{}\n\nSource: {}:{}{}\nReason: {}\n\n",
            item.title,
            item.body,
            item.source.source_kind.as_str(),
            item.source.source_ref,
            item.source
                .anchor
                .as_ref()
                .map(|anchor| format!("#{anchor}"))
                .unwrap_or_default(),
            item.inclusion_reason
        ));
    }

    markdown
}

fn render_explanation_markdown(
    request: &SourceLinkedMemoryPacketRequest,
    included: &[MemoryPacketIncludedItem],
    excluded: &[MemoryPacketDecision],
) -> String {
    let mut markdown = format!(
        "# Memory Packet Explanation {}\n\nPurpose: {}\n\n",
        request.memory_packet_id, request.purpose
    );
    markdown.push_str("## Included\n\n");
    for item in included {
        markdown.push_str(&format!(
            "- {} from {}:{} because {}\n",
            item.title,
            item.source.source_kind.as_str(),
            item.source.source_ref,
            item.inclusion_reason
        ));
    }
    markdown.push_str("\n## Excluded\n\n");
    for item in excluded {
        markdown.push_str(&format!(
            "- {} from {}:{} because {}\n",
            item.title,
            item.source.source_kind.as_str(),
            item.source.source_ref,
            item.reason
        ));
    }
    markdown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prototype_memory_backend_is_fake_packet_builder() {
        assert_eq!(PROTOTYPE_MEMORY_BACKEND, "fake-packet-builder");
    }

    #[test]
    fn fake_memory_reports_memory_boundary() {
        assert_eq!(
            MemoryBackend::fake().binding().kind,
            BoundaryKind::MemoryBackend
        );
    }

    #[test]
    fn source_linked_packet_includes_reviewed_sources_and_reasons() {
        let packet = MemoryBackend::fake().build_source_linked_packet(packet_request(vec![
            reviewed_candidate(
                "Architecture Boundary",
                MemorySourceKind::Markdown,
                "workpads/architecture/boundaries.md",
                "Use static dispatch for known prototype boundaries.",
                24,
            ),
            reviewed_candidate(
                "Tool Event",
                MemorySourceKind::Event,
                "event-tool-result-delivered-session-fake-codex",
                "Tool result delivery has durable audit evidence.",
                18,
            ),
        ]));

        assert_eq!(packet.included.len(), 2);
        assert_eq!(packet.excluded.len(), 0);
        assert!(
            packet
                .packet_markdown
                .contains("Source: markdown:workpads/architecture/boundaries.md")
        );
        assert!(
            packet
                .packet_markdown
                .contains("Reason: relevant reviewed project context")
        );
        assert!(packet.explanation_markdown.contains("Tool Event"));
        assert_eq!(
            packet.packet_artifact_id,
            "artifact-memory-packet-packet-source-linked"
        );
    }

    #[test]
    fn source_linked_packet_excludes_unreviewed_secret_and_over_budget_items() {
        let packet = MemoryBackend::fake().build_source_linked_packet(packet_request(vec![
            reviewed_candidate(
                "Included",
                MemorySourceKind::Markdown,
                "workpads/prototype/knowledge.md",
                "Prototype evidence is tracked in workpads.",
                20,
            ),
            MemoryCandidate {
                title: "Generated Draft".to_string(),
                body: "Generated memory should wait for review.".to_string(),
                source: source(MemorySourceKind::Artifact, "artifact-generated"),
                review_state: MemoryReviewState::Generated,
                sensitivity: MemorySensitivity::Internal,
                estimated_tokens: 10,
                inclusion_reason: "unreviewed generated note".to_string(),
            },
            MemoryCandidate {
                title: "Credential".to_string(),
                body: "session_token=secret".to_string(),
                source: source(MemorySourceKind::Artifact, "artifact-secret"),
                review_state: MemoryReviewState::Reviewed,
                sensitivity: MemorySensitivity::Secret,
                estimated_tokens: 10,
                inclusion_reason: "should never be included".to_string(),
            },
            reviewed_candidate(
                "Too Large",
                MemorySourceKind::ExternalImport,
                "external-record",
                "This otherwise valid memory exceeds the remaining budget.",
                90,
            ),
        ]));

        assert_eq!(packet.included.len(), 1);
        assert_eq!(packet.excluded.len(), 3);
        assert!(
            packet
                .excluded
                .iter()
                .any(|item| item.reason.contains("review_state=generated"))
        );
        assert!(
            packet
                .excluded
                .iter()
                .any(|item| item.reason.contains("credential material"))
        );
        assert!(
            packet
                .excluded
                .iter()
                .any(|item| item.reason.contains("budget exhausted"))
        );
        assert!(!packet.packet_markdown.contains("session_token"));
    }

    fn packet_request(candidates: Vec<MemoryCandidate>) -> SourceLinkedMemoryPacketRequest {
        SourceLinkedMemoryPacketRequest {
            memory_packet_id: MemoryPacketId::new("packet-source-linked"),
            session_id: SessionId::new("session-fake-codex"),
            run_id: "run-fake-codex".to_string(),
            turn_id: "turn-fake-codex".to_string(),
            purpose: "turn_context".to_string(),
            budget_tokens: 64,
            candidates,
        }
    }

    fn reviewed_candidate(
        title: &str,
        source_kind: MemorySourceKind,
        source_ref: &str,
        body: &str,
        estimated_tokens: usize,
    ) -> MemoryCandidate {
        MemoryCandidate {
            title: title.to_string(),
            body: body.to_string(),
            source: source(source_kind, source_ref),
            review_state: MemoryReviewState::Reviewed,
            sensitivity: MemorySensitivity::Internal,
            estimated_tokens,
            inclusion_reason: "relevant reviewed project context".to_string(),
        }
    }

    fn source(source_kind: MemorySourceKind, source_ref: &str) -> MemorySourceRef {
        MemorySourceRef {
            source_kind,
            source_ref: source_ref.to_string(),
            anchor: Some("P9".to_string()),
            content_hash: "fnv1a64:test".to_string(),
        }
    }
}
