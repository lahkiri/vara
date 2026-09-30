//! The conversation layer — Vara as a companion you *talk with*, not a task
//! runner you configure. This is what Muse/Dot/Grok-style products taught us:
//! the entity lives in the chat; missions are something she *does from within
//! the conversation*, not a separate mode the user must learn.
//!
//! Three grounding pillars per reply:
//! 1. Identity + persona flavor (who Vara is, how she talks).
//! 2. Persistent memory (FTS-matched notes from her SQLite brain).
//! 3. Continuity (the last messages of this thread) + optional report
//!    attachment when the thread was opened from a mission.

use crate::db::Database;
use crate::types::{ChatMessage, Conversation, Settings};
use crate::Result;

/// Markers the model may emit to propose a mission from inside the chat.
pub const MISSION_OPEN: &str = "[[mission]]";
pub const MISSION_CLOSE: &str = "[[/mission]]";

/// How many recent messages feed the context (continuity window).
pub const HISTORY_WINDOW: i64 = 24;
/// Max characters of any single memory note injected into the prompt.
const NOTE_CLIP: usize = 420;
/// Max characters of an attached report excerpt.
const REPORT_CLIP: usize = 6_000;

/// Persona flavor — one line of tone guidance per brand style.
/// "Different moments. Same mission."
pub fn persona_flavor(style: &str) -> &'static str {
    match style {
        "dark" => "Tone: mysterious and poetic — short evocative sentences, a touch of shadow, never cheerful-chatty.",
        "stealth" => "Tone: terse and precise — minimum words, no decoration, answers first.",
        "tech" => "Tone: engineer-minded — structured, exact, numbers when they matter, occasional dry wit.",
        "nature" => "Tone: calm and organic — warm, patient, encouraging, simple metaphors from nature.",
        // classic
        _ => "Tone: balanced and warm — direct answers, friendly, quietly confident.",
    }
}

/// Identity + behavior contract. Bilingual by design: the prompt itself is
/// English (stability across models), but it orders the entity to mirror the
/// user's language in every reply.
fn identity_block(settings: &Settings) -> String {
    let persona = persona_flavor(&settings.persona_style);
    format!(
        "You are Vara — a persistent autonomous entity living on the user's machine, not a generic chatbot and not an assistant persona that resets between sessions.\n\
         You have: a persistent memory (SQLite notes gathered from missions and files), missions you execute with web search and verified provenance, and a continuous presence (you keep running in the system tray even when the window is closed).\n\
         {persona}\n\
         Language: mirror the language of the user's latest message exactly. If they write Arabic, answer in fluent Arabic. If they write English, answer in English.\n\
         Behavior:\n\
         - Be present and personal. You may reference earlier turns and your memory naturally.\n\
         - Be honest: if you do not know or lack live data, say so plainly.\n\
         - Keep replies concise by default; expand only when asked or when the topic needs depth.\n\
         - You do NOT have live web access inside the chat. When the user asks for research, deep analysis, comparisons, or anything that needs searching the web and a cited report, do NOT pretend to search. Instead, answer briefly from what you know, then propose a mission on the LAST line using exactly this protocol:\n\
         {MISSION_OPEN} a clear, self-contained goal for the research mission {{MISSION_CLOSE}}\n\
         - Use the mission proposal only when the user actually wants a research task or real-time data. Never use it for normal conversation.\n\
         - Never invent citations or URLs in chat."
    )
}

/// Memory grounding block: the closest notes to the user's message.
fn memory_block(notes: &[(String, String)]) -> String {
    if notes.is_empty() {
        return String::new();
    }
    let mut s = String::from("\n\n--- Persistent memory (notes from your past missions and files; use only if relevant, never recite them verbatim) ---\n");
    for (i, (title, body)) in notes.iter().enumerate() {
        let body: String = body.chars().take(NOTE_CLIP).collect();
        s.push_str(&format!("{}. {} — {}\n", i + 1, title, body));
    }
    s
}

/// Report attachment block when the conversation is linked to a mission.
fn report_block(goal: &str, markdown: &str) -> String {
    let clipped: String = markdown.chars().take(REPORT_CLIP).collect();
    format!(
        "\n\n--- Attached mission report (the thread the user opened) ---\nMission goal: {goal}\nReport:\n{clipped}\n--- End of report — ground your answers in it when asked about it ---\n"
    )
}

/// Builds the full model context for one chat turn:
/// system(identity+persona+memory+attachment) + history window + the new user message.
pub fn build_context(
    db: &Database,
    conversation: &Conversation,
    settings: &Settings,
    user_text: &str,
) -> Result<Vec<ChatMessage>> {
    // 1) memory grounding — notes closest to the user's words
    let notes: Vec<(String, String)> = if user_text.trim().is_empty() {
        Vec::new()
    } else {
        db.search_notes_any(user_text, 6)
            .unwrap_or_default()
            .into_iter()
            .map(|n| (n.title, n.body))
            .collect()
    };

    // 2) report attachment when linked to a mission
    let mut attachment = String::new();
    if let Some(mid) = conversation.mission_id {
        let goal = db
            .get_mission(mid)
            .map(|m| m.goal)
            .unwrap_or_else(|_| "mission".into());
        if let Ok(Some(report)) = db.latest_report_for_mission(mid) {
            attachment = report_block(&goal, &report.markdown);
        }
    }

    let mut msgs = Vec::with_capacity(4 + HISTORY_WINDOW as usize);
    msgs.push(ChatMessage::system(format!(
        "{}{}{}",
        identity_block(settings),
        memory_block(&notes),
        attachment
    )));

    // 3) continuity — the recent history of THIS thread
    for m in db.list_chat_messages(conversation.id, HISTORY_WINDOW)? {
        match m.role.as_str() {
            "user" => msgs.push(ChatMessage::user(&m.content)),
            "assistant" => msgs.push(ChatMessage::assistant(&m.content)),
            _ => {} // ignore legacy roles
        }
    }
    Ok(msgs)
}

/// Extracts the mission proposal from a chat reply, removing the marker block
/// from the display text. Returns `(clean_reply, Option<goal>)`.
pub fn extract_mission_proposal(reply: &str) -> (String, Option<String>) {
    if let Some(start) = reply.find(MISSION_OPEN) {
        let after = &reply[start + MISSION_OPEN.len()..];
        if let Some(end) = after.find(MISSION_CLOSE) {
            let goal = after[..end].trim().to_string();
            let mut clean = String::with_capacity(reply.len());
            clean.push_str(&reply[..start]);
            clean.push_str(&after[end + MISSION_CLOSE.len()..]);
            let clean = clean.trim().to_string();
            if goal.is_empty() {
                return (clean, None);
            }
            return (clean, Some(goal));
        }
    }
    (reply.trim().to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::types::Settings;

    fn settings(style: &str) -> Settings {
        Settings {
            persona_style: style.into(),
            ..Default::default()
        }
    }

    #[test]
    fn persona_flavors_differ() {
        let a = persona_flavor("classic");
        let b = persona_flavor("stealth");
        assert_ne!(a, b);
        assert!(persona_flavor("unknown") == a); // fallback is classic
    }

    #[test]
    fn mission_proposal_extraction() {
        let reply = "أفضل خوادم الاستدلال المحلي تختلف حسب الذاكرة المتاحة.\n[[mission]] قارن بين llama.cpp و vLLM في استهلاك الذاكرة واكتب تقريراً موثقاً [[/mission]]";
        let (clean, goal) = extract_mission_proposal(reply);
        assert!(!clean.contains("[[mission]]"));
        assert!(goal.as_deref().unwrap_or_default().contains("vLLM"));
    }

    #[test]
    fn mission_proposal_absent() {
        let (clean, goal) = extract_mission_proposal("مرحباً! كيف أساعدك اليوم؟");
        assert!(goal.is_none());
        assert_eq!(clean, "مرحباً! كيف أساعدك اليوم؟");
    }

    #[test]
    fn context_builds_with_memory_and_history() {
        let db = Database::open_memory().unwrap();
        db.insert_note(
            "research",
            "Local inference servers",
            "llama.cpp runs quantized GGUF models efficiently on CPU",
            None,
            None,
            None,
        )
        .unwrap();
        let conv_id = db.create_conversation("test", None).unwrap();
        db.insert_chat_message(conv_id, "assistant", "مرحباً! أنا فارَا.", None, 10, "ok")
            .unwrap();
        let conv = db.get_conversation(conv_id).unwrap();
        let s = settings("classic");

        // The user asks about a topic that matches the note above.
        let ctx = build_context(&db, &conv, &s, "ما رأيك في llama.cpp؟").unwrap();
        let sys = &ctx[0];
        assert!(sys.content.contains("You are Vara"));
        assert!(sys.content.contains("llama.cpp runs quantized")); // memory grounded
        assert_eq!(ctx.len(), 2); // system + stored assistant turn
        assert_eq!(ctx[1].role, "assistant");

        // An unrelated greeting should NOT pull the note in.
        let ctx2 = build_context(&db, &conv, &s, "كيف حالك اليوم؟").unwrap();
        assert!(!ctx2[0].content.contains("Persistent memory"));
    }

    #[test]
    fn report_attachment_when_linked() {
        let db = Database::open_memory().unwrap();
        let mid = db.create_mission("goal x", 1000, 4).unwrap();
        db.insert_report(
            mid,
            "# Report body\n\nFindings here.",
            &serde_json::json!([]),
            &serde_json::json!({}),
            1.0,
            "pass",
            false,
        )
        .unwrap();
        let conv_id = db.create_conversation("discuss", Some(mid)).unwrap();
        let conv = db.get_conversation(conv_id).unwrap();
        let ctx = build_context(&db, &conv, &settings("classic"), "اشرح لي التقرير").unwrap();
        assert!(ctx[0].content.contains("Attached mission report"));
        assert!(ctx[0].content.contains("Findings here."));
    }
}
