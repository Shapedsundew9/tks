//! Model Context Protocol (MCP) prompt templates and capability handlers (TB-14, D-124).
//!
//! Provides canonical, versioned pass instructions for the Intent Elicitation pipeline:
//! 1. `elicitation/episode-segmentation/v1`: Pass 1 Episode Segmentation & Coreference
//! 2. `elicitation/goal-extraction/v1`: Pass 2 Schema-Constrained Technology-Agnostic Extraction
//! 3. `elicitation/entailment-judge/v1`: Pass 4 Entailment Verification Judge
//! 4. `elicitation/gap-questions/v1`: Pass 5 Gap Analysis & Grounded Question Generation

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Definition of an argument accepted by an MCP prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPromptArgument {
    pub name: String,
    pub description: String,
    pub required: bool,
}

/// Definition of an MCP prompt for `prompts/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPromptDefinition {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<McpPromptArgument>,
}

/// Text content block in an MCP prompt message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPromptMessageContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

/// A single message in an MCP prompt sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPromptMessage {
    pub role: String,
    pub content: McpPromptMessageContent,
}

/// Result returned by `prompts/get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPromptGetResult {
    pub description: String,
    pub messages: Vec<McpPromptMessage>,
}

/// Returns definitions for all MCP prompts supported by TKS (TB-14).
#[must_use]
pub fn list_prompts() -> Vec<McpPromptDefinition> {
    vec![
        McpPromptDefinition {
            name: "elicitation/episode-segmentation/v1".to_string(),
            description: "Segments a conversational elicitation transcript into contiguous, coherent topic episodes and resolves coreferences to explicit verbatim referent quotes.".to_string(),
            arguments: vec![McpPromptArgument {
                name: "transcript_text".to_string(),
                description: "Full text of the conversational transcript to segment into topic episodes".to_string(),
                required: true,
            }],
        },
        McpPromptDefinition {
            name: "elicitation/goal-extraction/v1".to_string(),
            description: "Extracts technology-agnostic functional goals (ACHIEVE), operational constraints (MAINTAIN), anti-goals (AVOID), and qualitative qualities (SOFTGOAL) with verbatim quotes from conversation turns.".to_string(),
            arguments: vec![
                McpPromptArgument {
                    name: "episode_topic".to_string(),
                    description: "Topic of the current conversation episode".to_string(),
                    required: true,
                },
                McpPromptArgument {
                    name: "episode_text".to_string(),
                    description: "Utterance text and speaker turns for this episode".to_string(),
                    required: true,
                },
                McpPromptArgument {
                    name: "participants".to_string(),
                    description: "List of participants and roles (e.g. 'Priya (stakeholder), Interviewer (facilitator)')".to_string(),
                    required: true,
                },
            ],
        },
        McpPromptDefinition {
            name: "elicitation/entailment-judge/v1".to_string(),
            description: "Evaluates whether a candidate goal is forward entailed by the stakeholder's statements (ENTAILED), represents scope creep (NEUTRAL), or contradicts them (CONTRADICTED), and checks for reverse over-constraint.".to_string(),
            arguments: vec![
                McpPromptArgument {
                    name: "premise_turns".to_string(),
                    description: "Verbatim utterance text of the cited stakeholder turns".to_string(),
                    required: true,
                },
                McpPromptArgument {
                    name: "goal_statement".to_string(),
                    description: "Extracted candidate goal statement to verify".to_string(),
                    required: true,
                },
            ],
        },
        McpPromptDefinition {
            name: "elicitation/gap-questions/v1".to_string(),
            description: "Formulates a neutral, grounded, open-ended question for the stakeholder to address a detected requirement or vision gap in the next elicitation cycle.".to_string(),
            arguments: vec![
                McpPromptArgument {
                    name: "gap_id".to_string(),
                    description: "Identifier of the detected gap (e.g. 'GP-1')".to_string(),
                    required: true,
                },
                McpPromptArgument {
                    name: "gap_detector".to_string(),
                    description: "Type of gap detector that fired (e.g. 'SOFTGOAL_WITHOUT_CONTRIBUTOR')".to_string(),
                    required: true,
                },
                McpPromptArgument {
                    name: "gap_description".to_string(),
                    description: "Detailed description of the observed gap".to_string(),
                    required: true,
                },
                McpPromptArgument {
                    name: "context_turns".to_string(),
                    description: "Relevant transcript excerpts or context turns".to_string(),
                    required: false,
                },
            ],
        },
    ]
}

/// Retrieves and renders an MCP prompt by name and parameter map (TB-14).
///
/// # Errors
///
/// Returns an error string if prompt name is unknown or required arguments are missing.
pub fn get_prompt(name: &str, params: &Value) -> Result<McpPromptGetResult, String> {
    let mut args_map = HashMap::new();
    if let Some(obj) = params.as_object() {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                args_map.insert(k.as_str(), s);
            } else if !v.is_null() {
                args_map.insert(k.as_str(), v.to_string().leak());
            }
        }
    }

    match name {
        "elicitation/episode-segmentation/v1" => {
            let transcript_text = args_map
                .get("transcript_text")
                .ok_or_else(|| "Missing required argument 'transcript_text'".to_string())?;

            let text = format!(
                "You are an expert requirements engineer executing Pass 1 (Episode Segmentation & Coreference Resolution) of the TKS Intent Elicitation Pipeline.\n\n\
                ### Objective\n\
                Divide the provided transcript into contiguous, coherent topic episodes. Every turn from the first to the last must be covered without gaps, overlaps, or omissions.\n\n\
                ### Rules\n\
                1. Contiguity: Episodes must be strictly contiguous. If Episode 1 ends at turn t5, Episode 2 must begin at turn t6.\n\
                2. Complete Coverage: The first episode must begin at turn t1. The final episode must end at the last turn in the transcript.\n\
                3. Coherence: Each episode focuses on a coherent discussion topic (e.g., status visibility pain, data entry friction, boundary discussion).\n\
                4. Coreference Resolution: For any conversational references (\"that idea\", \"what she said earlier\", \"it\"), identify the explicit verbatim referent quote from the speaker.\n\n\
                ### Output Format\n\
                Submit the episodes via the `submit_episodes` MCP tool formatted as:\n\
                {{\n  \"episodes\": [\n    {{\n      \"id\": \"E1\",\n      \"turns\": [\"t1\", \"t5\"],\n      \"topic\": \"Concise topic summary\"\n    }}\n  ]\n}}\n\n\
                ### Transcript Text:\n\
                {transcript_text}"
            );

            Ok(McpPromptGetResult {
                description: "Segments a conversational elicitation transcript into contiguous, coherent topic episodes.".to_string(),
                messages: vec![McpPromptMessage {
                    role: "user".to_string(),
                    content: McpPromptMessageContent {
                        content_type: "text".to_string(),
                        text,
                    },
                }],
            })
        }

        "elicitation/goal-extraction/v1" => {
            let episode_topic = args_map
                .get("episode_topic")
                .ok_or_else(|| "Missing required argument 'episode_topic'".to_string())?;
            let episode_text = args_map
                .get("episode_text")
                .ok_or_else(|| "Missing required argument 'episode_text'".to_string())?;
            let participants = args_map
                .get("participants")
                .ok_or_else(|| "Missing required argument 'participants'".to_string())?;

            let text = format!(
                "You are an expert requirements engineer executing Pass 2 (Schema-Constrained Technology-Agnostic Extraction) of the TKS Intent Elicitation Pipeline.\n\n\
                ### Objective\n\
                Extract all stated goals, constraints, anti-goals, and soft goals from the provided episode.\n\n\
                ### Taxonomy\n\
                - ACHIEVE: Desired future state, functional objective, or new capability.\n\
                - MAINTAIN: Operational constraint, invariant, compliance rule, or property that must remain true.\n\
                - AVOID: Anti-goal, failure mode, risk to prevent, or explicit non-goal.\n\
                - SOFTGOAL: Qualitative property or non-functional aspiration without clear-cut binary satisfaction criteria (e.g., developer joy, minimal disruption, high auditability).\n\n\
                ### Invariants & Traps\n\
                1. Technology-Agnosticism: Do NOT concretize technical mechanisms (e.g. database, dashboard, slack bot, react, microservice) unless the stakeholder explicitly requested that specific mechanism. If an analogy was used (\"like an Excel sheet\"), extract the underlying goal, NOT the metaphor.\n\
                2. Verbatim Grounding: Every goal MUST cite >= 1 verbatim quote copied character-for-character from the utterance text of a stakeholder turn.\n\
                3. Facilitator Trap: Do not extract a goal from a facilitator's leading question unless the stakeholder explicitly confirmed it.\n\
                4. Status Tracking: Mark each goal as CONFIRMED, TENTATIVE, or RETRACTED (if the speaker later retracted or changed their mind).\n\
                5. Links: Identify candidate relationships between goals (REFINES, CONFLICTS_WITH, CONTRIBUTES_TO).\n\n\
                ### Episode Topic:\n\
                {episode_topic}\n\n\
                ### Participants:\n\
                {participants}\n\n\
                ### Episode Text:\n\
                {episode_text}\n\n\
                Submit the extracted goals via `submit_goal_candidates`."
            );

            Ok(McpPromptGetResult {
                description: "Extracts technology-agnostic goals, constraints, anti-goals, and soft goals from conversation turns.".to_string(),
                messages: vec![McpPromptMessage {
                    role: "user".to_string(),
                    content: McpPromptMessageContent {
                        content_type: "text".to_string(),
                        text,
                    },
                }],
            })
        }

        "elicitation/entailment-judge/v1" => {
            let premise_turns = args_map
                .get("premise_turns")
                .ok_or_else(|| "Missing required argument 'premise_turns'".to_string())?;
            let goal_statement = args_map
                .get("goal_statement")
                .ok_or_else(|| "Missing required argument 'goal_statement'".to_string())?;

            let text = format!(
                "You are an impartial natural language inference judge executing Pass 4 (Entailment Verification) of the TKS Intent Elicitation Pipeline.\n\n\
                ### Objective\n\
                Determine whether the candidate goal is genuinely supported by the stakeholder's statements or represents hallucination/scope creep.\n\n\
                ### Evaluation Criteria\n\
                1. Forward Entailment:\n\
                   - ENTAILED: The premise statements logically entail or directly express the goal.\n\
                   - NEUTRAL: The goal is plausible, common practice, or related, but NOT stated by the stakeholder (scope creep / phantom goal).\n\
                   - CONTRADICTED: The premise contradicts, rejects, or rules out the goal statement.\n\
                2. Reverse Over-constraint:\n\
                   - true: The goal statement adds specific constraints or mechanisms narrower than what the stakeholder stated.\n\
                   - false: The goal statement accurately reflects the scope of the premise.\n\n\
                ### Premise Turns:\n\
                {premise_turns}\n\n\
                ### Goal Statement:\n\
                {goal_statement}\n\n\
                Submit your verdict via `submit_verification_verdicts`."
            );

            Ok(McpPromptGetResult {
                description:
                    "Evaluates forward entailment and reverse over-constraint for candidate goals."
                        .to_string(),
                messages: vec![McpPromptMessage {
                    role: "user".to_string(),
                    content: McpPromptMessageContent {
                        content_type: "text".to_string(),
                        text,
                    },
                }],
            })
        }

        "elicitation/gap-questions/v1" => {
            let gap_id = args_map
                .get("gap_id")
                .ok_or_else(|| "Missing required argument 'gap_id'".to_string())?;
            let gap_detector = args_map
                .get("gap_detector")
                .ok_or_else(|| "Missing required argument 'gap_detector'".to_string())?;
            let gap_description = args_map
                .get("gap_description")
                .ok_or_else(|| "Missing required argument 'gap_description'".to_string())?;
            let context_turns = args_map
                .get("context_turns")
                .copied()
                .unwrap_or("(No additional context provided)");

            let text = format!(
                "You are an empathetic, neutral requirements facilitator executing Pass 5 (Gap Resolution Question Generation) of the TKS Intent Elicitation Pipeline.\n\n\
                ### Objective\n\
                Generate a neutral, open-ended question for the stakeholder to address the detected gap in the next conversation cycle.\n\n\
                ### Guidelines\n\
                1. Do NOT suggest specific technical solutions or lead the stakeholder.\n\
                2. Directly reference the context and what the stakeholder previously shared.\n\
                3. Allow the stakeholder to clarify boundaries, priorities, or missing criteria.\n\n\
                ### Gap Details:\n\
                - Gap ID: {gap_id}\n\
                - Detector: {gap_detector}\n\
                - Description: {gap_description}\n\
                - Context: {context_turns}\n\n\
                Formulate one concise, targeted question for the interview guide."
            );

            Ok(McpPromptGetResult {
                description: "Formulates a neutral, grounded, open-ended question to resolve an elicitation gap.".to_string(),
                messages: vec![McpPromptMessage {
                    role: "user".to_string(),
                    content: McpPromptMessageContent {
                        content_type: "text".to_string(),
                        text,
                    },
                }],
            })
        }

        other => Err(format!("Unknown prompt: '{other}'")),
    }
}
