use serde::{Deserialize, Serialize};

use crate::entropy;
use crate::knapsack::{self, Fragment};
use crate::storage::{Drawer, Room, Storage, Wing};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMeta {
    pub wing_name: String,
    pub room_name: String,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakeupContext {
    pub l0_wings: Vec<Wing>,
    pub l1_rooms: Vec<RoomMeta>,
    pub l2_drawers: Vec<Drawer>,
    pub tokens_used: usize,
    pub budget_tokens: usize,
}

fn approx_tokens(s: &str) -> usize {
    s.chars().count() / 4
}

pub fn build_wakeup(storage: &dyn Storage, budget_tokens: usize) -> anyhow::Result<WakeupContext> {
    let wings = storage.list_wings()?;

    // L0 — wing metadata always included; estimate token cost
    let l0_text: String = wings
        .iter()
        .map(|w| {
            format!(
                "{}: {}",
                w.name,
                w.description.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let l0_tokens = approx_tokens(&l0_text).max(1);
    let remaining = budget_tokens.saturating_sub(l0_tokens);

    // Collect all rooms across wings with their entropy scores
    let mut room_metas: Vec<(Wing, Room, String, f64)> = Vec::new();
    for wing in &wings {
        let rooms = storage.list_rooms(wing.id)?;
        for room in rooms {
            let text = format!(
                "{}/{}: {}",
                wing.name,
                room.name,
                room.summary.as_deref().unwrap_or("")
            );
            let e = entropy::score(&text);
            room_metas.push((wing.clone(), room, text, e));
        }
    }

    // Build fragments for knapsack — each room is one fragment
    let room_texts: Vec<String> = room_metas.iter().map(|(_, _, t, _)| t.clone()).collect();
    let fragments: Vec<Fragment> = room_metas
        .iter()
        .zip(room_texts.iter())
        .map(|((_, _, _, e), text)| Fragment {
            text: text.as_str(),
            tokens: approx_tokens(text).max(1),
            entropy: *e,
        })
        .collect();

    let selected_indices: Vec<usize> = {
        // select_optimal returns references; map back to indices
        let selected = knapsack::select_optimal(&fragments, remaining);
        selected
            .iter()
            .map(|sel| {
                fragments
                    .iter()
                    .position(|f| std::ptr::eq(f as *const Fragment, *sel as *const Fragment))
                    .unwrap_or(0)
            })
            .collect()
    };

    let mut l1_tokens = 0usize;
    let mut l1_rooms: Vec<RoomMeta> = Vec::new();
    for &idx in &selected_indices {
        let (wing, room, text, _) = &room_metas[idx];
        l1_tokens += approx_tokens(text).max(1);
        l1_rooms.push(RoomMeta {
            wing_name: wing.name.clone(),
            room_name: room.name.clone(),
            summary: room.summary.clone(),
        });
    }

    // L2 — fill remaining budget with recent drawers
    let drawer_budget = remaining.saturating_sub(l1_tokens);
    let mut l2_drawers: Vec<Drawer> = Vec::new();
    let mut drawer_tokens = 0usize;

    if drawer_budget > 0 {
        let candidates = storage.get_recent_drawers(50)?;
        for drawer in candidates {
            let t = approx_tokens(&drawer.content).max(1);
            if drawer_tokens + t > drawer_budget {
                break;
            }
            drawer_tokens += t;
            l2_drawers.push(drawer);
        }
    }

    let tokens_used = l0_tokens + l1_tokens + drawer_tokens;

    Ok(WakeupContext {
        l0_wings: wings,
        l1_rooms,
        l2_drawers,
        tokens_used,
        budget_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{Confidence, NewDrawer, Source, SqliteStorage};

    fn setup() -> SqliteStorage {
        SqliteStorage::open_in_memory().unwrap()
    }

    #[test]
    fn test_build_wakeup_empty_palace() {
        let s = setup();
        let ctx = build_wakeup(&s, 500).unwrap();
        assert!(ctx.l0_wings.is_empty());
        assert!(ctx.l1_rooms.is_empty());
        assert!(ctx.l2_drawers.is_empty());
        assert!(ctx.tokens_used <= 500);
    }

    #[test]
    fn test_build_wakeup_with_data() {
        let s = setup();
        let wing = s.create_wing("project", Some("main project context")).unwrap();
        let room = s.create_room(wing.id, "auth", Some("authentication and JWT handling")).unwrap();
        s.store_drawer(&NewDrawer {
            wing_id: wing.id,
            room_id: room.id,
            content: "The API uses RS256-signed JWT tokens with a 1-hour expiry".to_string(),
            compressed_content: None,
            confidence: Confidence::High,
            source: Source::User,
        })
        .unwrap();

        let ctx = build_wakeup(&s, 500).unwrap();
        assert_eq!(ctx.l0_wings.len(), 1);
        assert!(ctx.tokens_used <= 500);
        assert!(ctx.tokens_used <= ctx.budget_tokens);
    }

    #[test]
    fn test_budget_respected() {
        let s = setup();
        let wing = s.create_wing("w", None).unwrap();
        let room = s.create_room(wing.id, "r", None).unwrap();
        for i in 0..20 {
            s.store_drawer(&NewDrawer {
                wing_id: wing.id,
                room_id: room.id,
                content: format!("fact number {} with some additional detail to consume tokens in the budget test scenario", i),
                compressed_content: None,
                confidence: Confidence::Medium,
                source: Source::Conversation,
            })
            .unwrap();
        }

        let ctx = build_wakeup(&s, 100).unwrap();
        assert!(ctx.tokens_used <= 100);
    }
}
