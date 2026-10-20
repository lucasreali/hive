use super::*;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, Channel, InvokeBody, InvokeResponseBody};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::webview::InvokeRequest;

fn agent(id: &str, title: &str, state: AgentState, space: Option<&str>) -> Agent {
    (id.into(), title.into(), state, space.map(Into::into))
}

/// An enabled menu item.
fn item(id: &str, text: &str) -> Entry {
    Entry::Item {
        id: id.into(),
        text: text.into(),
        enabled: true,
    }
}

/// A disabled line of text, its id `note:<name>`.
fn text(name: &str, text: &str) -> Entry {
    Entry::Item {
        id: format!("note:{name}"),
        text: text.into(),
        enabled: false,
    }
}

fn show() -> Entry {
    item("show", "Show Hive")
}

#[test]
fn the_menu_lists_the_pending_agents_then_the_usage_then_show_hive() {
    let agents = [
        agent("a1", "fix login", AgentState::WaitingPermission, None),
        agent("a2", "Claude", AgentState::WaitingYou, Some("Work")),
    ];
    let usage = [
        "Session 42% · resets 14:30".to_owned(),
        "Week 41% · resets Mon 09:00".to_owned(),
    ];
    assert_eq!(
        entries(&agents, &usage),
        [
            item("agent:a1", "fix login — waiting for permission"),
            item("agent:a2", "Claude — Work · waiting for you"),
            Entry::Separator,
            text("usage0", "Session 42% · resets 14:30"),
            text("usage1", "Week 41% · resets Mon 09:00"),
            Entry::Separator,
            show(),
        ]
    );
}

#[test]
fn with_nobody_pending_and_no_usage_the_menu_says_so() {
    assert_eq!(
        entries(&[], &[]),
        [text("none", "No agent needs you"), Entry::Separator, show()]
    );
}

#[test]
fn the_menu_lists_at_most_max_agents_and_counts_the_rest() {
    let many: Vec<Agent> = (0..MAX_AGENTS + 2)
        .map(|i| agent(&format!("a{i}"), "Claude", AgentState::Error, None))
        .collect();
    let all = entries(&many[..MAX_AGENTS], &[]);
    assert_eq!(all.len(), MAX_AGENTS + 2, "no count when every one fits");
    assert_eq!(all[MAX_AGENTS - 1], item("agent:a19", "Claude — error"));
    let cut = entries(&many, &[]);
    assert_eq!(cut.len(), MAX_AGENTS + 3);
    assert_eq!(cut[MAX_AGENTS], text("more", "2 more in Hive"));
}

#[test]
fn the_menu_shows_at_most_max_usage_lines() {
    let usage: Vec<String> = (0..=MAX_USAGE).map(|i| format!("line {i}")).collect();
    let menu = entries(&[], &usage);
    assert_eq!(menu.len(), 1 + 1 + MAX_USAGE + 2);
    assert_eq!(menu[1 + MAX_USAGE], text("usage3", "line 3"));
}

#[test]
fn menu_texts_lose_control_characters_and_long_ones_are_cut() {
    let exact = "x".repeat(MAX_TEXT - 3);
    let agents = [
        agent("a1", "fix\u{7} login\n", AgentState::Error, None),
        agent("a2", &exact, AgentState::Idle, None),
        agent("a3", &"y".repeat(MAX_TEXT), AgentState::Idle, None),
    ];
    let menu = entries(&agents, &["Session\u{1b} 1%".to_owned()]);
    assert_eq!(menu[0], item("agent:a1", "fix login — error"));
    // MAX_TEXT - 3 characters and " — idle" are four too many: cut to MAX_TEXT, "…" last.
    let Entry::Item { text: long, .. } = &menu[1] else {
        panic!("an item")
    };
    assert_eq!(long.chars().count(), MAX_TEXT);
    assert!(long.ends_with("x —…"), "{long}");
    let Entry::Item { text: cut, .. } = &menu[2] else {
        panic!("an item")
    };
    assert_eq!(*cut, format!("{}…", "y".repeat(MAX_TEXT - 1)));
    assert_eq!(menu[4], text("usage0", "Session 1%"));
    // A text of exactly MAX_TEXT characters is kept whole.
    let whole = "z".repeat(MAX_TEXT);
    assert_eq!(
        entries(&[], std::slice::from_ref(&whole))[2],
        text("usage0", &whole)
    );
}

#[test]
fn each_state_reads_as_the_sidebar_says_it() {
    let texts = [
        (AgentState::WaitingPermission, "waiting for permission"),
        (AgentState::WaitingPlan, "waiting for plan approval"),
        (AgentState::WaitingAnswer, "waiting for your answer"),
        (AgentState::Error, "error"),
        (AgentState::WaitingYou, "waiting for you"),
        (AgentState::Working, "working"),
        (AgentState::WithSubagents, "running subagents"),
        (AgentState::Idle, "idle"),
        (AgentState::Ended, "ended"),
    ];
    for (state, text) in texts {
        assert_eq!(state_text(state), text);
    }
}

#[test]
fn the_icon_is_tinted_by_the_most_urgent_state_or_a_template_when_calm() {
    let yellow = Some([0xa4, 0x88, 0x19]);
    for state in [
        AgentState::WaitingPermission,
        AgentState::WaitingPlan,
        AgentState::WaitingAnswer,
    ] {
        assert_eq!(tint(state, true), yellow);
    }
    assert_eq!(tint(AgentState::Error, true), Some([0xd3, 0x61, 0x51]));
    assert_eq!(tint(AgentState::WaitingYou, true), Some([0xad, 0x6e, 0x25]));
    assert_eq!(tint(AgentState::Working, false), Some([0x5c, 0x78, 0xe2]));
    assert_eq!(
        tint(AgentState::WithSubagents, false),
        Some([0xa4, 0x49, 0xab])
    );
    // A seen 🟠 is drawn muted (15.6), so calm; idle and ended too.
    assert_eq!(tint(AgentState::WaitingYou, false), None);
    assert_eq!(tint(AgentState::Idle, false), None);
    assert_eq!(tint(AgentState::Ended, false), None);
}

/// The alpha of the pixel at (`x`, `y`).
fn alpha(icon: &Icon, x: u32, y: u32) -> u8 {
    icon.rgba[((y * SIZE + x) * 4 + 3) as usize]
}

fn total_alpha(icon: &Icon) -> u64 {
    icon.rgba
        .iter()
        .skip(3)
        .step_by(4)
        .map(|a| u64::from(*a))
        .sum()
}

#[test]
fn a_calm_icon_is_a_template_hexagon_outline() {
    for calm in [None, Some((AgentState::Idle, false))] {
        let icon = icon(calm);
        assert!(icon.template);
        assert_eq!(icon.rgba.len(), (SIZE * SIZE * 4) as usize);
        assert!(icon.rgba.chunks(4).all(|p| p[..3] == [0, 0, 0]));
        // Hollow in the middle, solid on its outline, empty at the corners.
        assert_eq!(alpha(&icon, SIZE / 2, SIZE / 2), 0);
        assert_eq!(alpha(&icon, SIZE / 2, 3), 255, "the top corner's outline");
        assert_eq!(alpha(&icon, 5, SIZE / 2), 255, "the left side");
        assert_eq!(alpha(&icon, 0, 0), 0);
        assert_eq!(alpha(&icon, SIZE - 1, SIZE - 1), 0);
        // Left-right and top-bottom symmetric.
        for (x, y) in [(5, 10), (8, 4), (16, 2), (2, 18)] {
            assert_eq!(alpha(&icon, x, y), alpha(&icon, SIZE - 1 - x, y));
            assert_eq!(alpha(&icon, x, y), alpha(&icon, x, SIZE - 1 - y));
        }
        assert_eq!(total_alpha(&icon), 72_600, "the outline's area");
    }
}

#[test]
fn an_urgent_icon_is_a_filled_hexagon_in_its_colour() {
    let icon = icon(Some((AgentState::Error, true)));
    assert!(!icon.template);
    assert!(icon.rgba.chunks(4).all(|p| p[..3] == [0xd3, 0x61, 0x51]));
    assert_eq!(alpha(&icon, SIZE / 2, SIZE / 2), 255);
    assert_eq!(alpha(&icon, SIZE / 2, 3), 255);
    assert_eq!(alpha(&icon, 0, 0), 0);
    // Anti-aliased: the edge of a slanted side is partly covered.
    assert!((1..255).contains(&alpha(&icon, 9, 6)));
    assert_eq!(total_alpha(&icon), 168_768, "the hexagon's area");
}

#[test]
fn the_tooltip_counts_the_pending_agents() {
    assert_eq!(tooltip(0), "Hive");
    assert_eq!(tooltip(1), "Hive — 1 agent needs you");
    assert_eq!(tooltip(3), "Hive — 3 agents need you");
}

/// The views a test tray was asked to show.
type Shown = Arc<Mutex<Vec<Tray>>>;

fn with_tray() -> (Hive, Shown) {
    let shown = Shown::default();
    let seen = Arc::clone(&shown);
    let hive = Hive::new("/nonexistent/hive-test".into(), vec![]).with_tray(move |view| {
        seen.lock().unwrap().push(view);
        Ok(())
    });
    (hive, shown)
}

#[test]
fn set_tray_shows_the_view_or_removes_the_tray_when_the_setting_is_off() {
    let (hive, shown) = with_tray();
    let agents = [agent("a1", "fix login", AgentState::Error, None)];
    let urgent = Some((AgentState::Error, true));
    assert_eq!(hive.set_tray(true, urgent, &agents, &[]), Ok(()));
    assert_eq!(hive.set_tray(false, urgent, &agents, &[]), Ok(()));
    let shown = shown.lock().unwrap();
    assert_eq!(
        *shown,
        [Tray::Show(view(urgent, &agents, &[])), Tray::Remove]
    );
    let Tray::Show(view) = &shown[0] else {
        panic!("shown")
    };
    assert_eq!(view.tooltip, "Hive — 1 agent needs you");
    assert!(!view.icon.template);
    assert_eq!(view.entries[0], item("agent:a1", "fix login — error"));
}

#[test]
fn set_tray_refuses_an_agent_id_longer_than_max_id() {
    let (hive, shown) = with_tray();
    let fits = [agent(
        &"a".repeat(MAX_ID),
        "Claude",
        AgentState::Error,
        None,
    )];
    assert_eq!(hive.set_tray(true, None, &fits, &[]), Ok(()));
    let long = [
        agent("a1", "Claude", AgentState::Error, None),
        agent(&"a".repeat(MAX_ID + 1), "Claude", AgentState::Error, None),
    ];
    assert_eq!(
        hive.set_tray(true, None, &long, &[]),
        Err(format!("an agent id is longer than {MAX_ID} bytes"))
    );
    assert_eq!(shown.lock().unwrap().len(), 1, "nothing shown for it");
}

#[test]
fn a_view_updates_the_tray_found_or_creates_one() {
    // One instantiation of `place` for both cases, so its coverage counts both arms.
    let place_on = |found: Option<u8>| {
        place(
            found,
            view(None, &[], &[]),
            |tray, v| Err(format!("updated {tray} {}", v.tooltip)),
            |v| Err(format!("created {}", v.tooltip)),
        )
    };
    assert_eq!(place_on(None), Err("created Hive".into()));
    assert_eq!(place_on(Some(7)), Err("updated 7 Hive".into()));
}

#[test]
fn only_the_agents_and_show_hive_can_be_chosen() {
    let agents = [agent("a1", "Claude", AgentState::Error, None)];
    let menu = entries(&agents, &["Session 1%".to_owned()]);
    let enabled: Vec<(&str, bool)> = menu
        .iter()
        .filter_map(|entry| match entry {
            Entry::Item { id, enabled, .. } => Some((id.as_str(), *enabled)),
            Entry::Separator => None,
        })
        .collect();
    assert_eq!(
        enabled,
        [("agent:a1", true), ("note:usage0", false), ("show", true)]
    );
    let none = entries(&[], &[]);
    assert!(matches!(&none[0], Entry::Item { enabled: false, .. }));
}

#[test]
fn set_tray_fails_without_a_tray_or_when_the_os_refuses_it() {
    let none = Hive::new("/nonexistent/hive-test".into(), vec![]);
    let no_tray = Err("this app has no menu bar icon".to_owned());
    assert_eq!(none.set_tray(true, None, &[], &[]), no_tray);
    let refused = Hive::new("/nonexistent/hive-test".into(), vec![])
        .with_tray(|_| Err("no status bar".into()));
    assert_eq!(
        refused.set_tray(true, None, &[], &[]),
        Err("no status bar".into())
    );
}

/// A UI control channel and what it received.
fn ui() -> (Channel<Value>, std::sync::mpsc::Receiver<Value>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let channel = Channel::new(move |body| {
        let InvokeResponseBody::Json(json) = body else {
            panic!("control messages are JSON")
        };
        tx.send(serde_json::from_str(&json).unwrap()).unwrap();
        Ok(())
    });
    (channel, rx)
}

#[test]
fn the_ui_sets_the_tray_through_its_command() {
    let (hive, shown) = with_tray();
    let app = mock_builder()
        .manage(hive)
        .invoke_handler(tauri::generate_handler![crate::commands::set_tray])
        .build(mock_context(noop_assets()))
        .unwrap();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let args = json!({
        "shown": true,
        "urgent": ["waiting_permission", true],
        "agents": [["a1", "fix login", "waiting_permission", "Work"]],
        "usage": ["Session 42% · resets 14:30"],
    });
    let request = InvokeRequest {
        cmd: "set_tray".into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: "tauri://localhost".parse().unwrap(),
        body: InvokeBody::Json(args),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_string(),
    };
    let answer = get_ipc_response(&webview, request).map(|b| b.deserialize::<Value>().unwrap());
    assert_eq!(answer, Ok(Value::Null));
    let shown = shown.lock().unwrap();
    let Tray::Show(view) = &shown[0] else {
        panic!("shown")
    };
    assert_eq!(
        view.entries[..3],
        [
            item("agent:a1", "fix login — Work · waiting for permission"),
            Entry::Separator,
            text("usage0", "Session 42% · resets 14:30"),
        ]
    );
}

#[test]
fn choosing_an_agent_brings_hive_up_on_it_and_show_hive_only_brings_it_up() {
    let (channel, rx) = ui();
    let hive = Hive::new("/nonexistent/hive-test".into(), vec![]);
    crate::lock(&hive.link).ui = Some(channel);
    let app = mock_builder().build(mock_context(noop_assets())).unwrap();
    tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    // The window comes up; the mock runtime keeps no window state to check more with.
    assert!(chosen(app.handle(), &hive, "show"));
    assert!(rx.try_recv().is_err(), "nothing goes to the UI");
    assert!(!chosen(app.handle(), &hive, "elsewhere"));
    assert!(rx.try_recv().is_err());
    assert!(chosen(app.handle(), &hive, "agent:a1"));
    let clicked = json!({"type": "tray_clicked", "agent": "a1"});
    assert_eq!(rx.try_recv().unwrap(), clicked);
    // Without the window, the agent still reaches the UI (a reload brings it back).
    let windowless = mock_builder().build(mock_context(noop_assets())).unwrap();
    assert!(!chosen(windowless.handle(), &hive, "show"));
    assert!(!chosen(windowless.handle(), &hive, "agent:a1"));
    assert_eq!(rx.try_recv().unwrap(), clicked);
}
