use anyhow::{Context, Result};
use yourmemory_core::storage::{Confidence, NewDrawer, Palace, Source, Storage};

fn open_palace() -> Result<Palace> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    Palace::open(&cwd).context("Palace not initialised — run `yourmemory init` first")
}

pub fn health() -> Result<()> {
    let palace = open_palace()?;
    let wings = palace.storage.list_wings()?;
    let mut room_count = 0usize;
    for wing in &wings {
        let rooms = palace.storage.list_rooms(wing.id)?;
        room_count += rooms.len();
    }
    let recent = palace.storage.get_recent_drawers(usize::MAX)?;
    let drawer_count = recent.len();

    let db_path = palace.root.join("palace.db");
    let db_size = std::fs::metadata(&db_path)
        .map(|m| m.len())
        .unwrap_or(0);

    println!("Palace health");
    println!("  Wings:   {}", wings.len());
    println!("  Rooms:   {room_count}");
    println!("  Drawers: {drawer_count}");
    println!("  DB size: {} bytes", db_size);
    println!("  Path:    {}", palace.root.display());
    Ok(())
}

pub fn search(query: &str) -> Result<()> {
    let palace = open_palace()?;
    let results = palace
        .storage
        .search_drawers(query, 10)
        .context("search failed")?;

    if results.is_empty() {
        println!("No results for {:?}", query);
        return Ok(());
    }

    for (i, d) in results.iter().enumerate() {
        let preview: String = d.content.chars().take(120).collect();
        println!("[{}] drawer#{} — {}", i + 1, d.id, preview);
    }
    Ok(())
}

pub fn persist(text: &str) -> Result<()> {
    let palace = open_palace()?;
    let wing = palace
        .storage
        .create_wing("general", Some("default wing for persisted facts"))
        .context("cannot create wing")?;
    let room = palace
        .storage
        .create_room(wing.id, "facts", Some("persisted facts"))
        .context("cannot create room")?;
    let drawer = palace
        .storage
        .store_drawer(&NewDrawer {
            wing_id: wing.id,
            room_id: room.id,
            content: text.to_string(),
            compressed_content: None,
            confidence: Confidence::High,
            source: Source::User,
        })
        .context("cannot store drawer")?;
    println!("Stored drawer#{}", drawer.id);
    Ok(())
}

pub fn wakeup() -> Result<()> {
    let palace = open_palace()?;
    let wings = palace.storage.list_wings()?;
    println!("=== Palace Wakeup (L0) ===");
    println!("Wings: {}", wings.len());
    for wing in &wings {
        let rooms = palace.storage.list_rooms(wing.id)?;
        println!("  Wing: {} ({} rooms)", wing.name, rooms.len());
        for room in &rooms {
            println!(
                "    Room: {} — {}",
                room.name,
                room.summary.as_deref().unwrap_or("")
            );
        }
    }
    println!("=== Recent drawers (L1) ===");
    let recent = palace.storage.get_recent_drawers(5)?;
    for d in &recent {
        let preview: String = d.content.chars().take(100).collect();
        println!("  [{}] {}", d.id, preview);
    }
    Ok(())
}

pub fn compact() -> Result<()> {
    let palace = open_palace()?;
    let recent = palace.storage.get_recent_drawers(usize::MAX)?;
    let mut compacted = 0usize;
    for drawer in &recent {
        if palace
            .storage
            .compact_drawer(drawer.id, 3, 30)
            .unwrap_or(false)
        {
            compacted += 1;
        }
    }
    println!("Compacted {compacted} drawers.");
    Ok(())
}
