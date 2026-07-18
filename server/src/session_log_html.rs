//! Formatage lisible + service HTTP (page HTML + SSE) du journal de session pour consultation
//! en direct pendant un playtest, sans accès SSH au volume monté (spec session-log-live-view,
//! 2026-07-18). Se limite à la présentation ; l'écriture/lecture brute reste dans
//! `session_log.rs`.

use crate::session_log::SessionEvent;

/// Lit les `n` dernières lignes non vides de `path`, dans l'ordre chronologique. Lecture
/// intégrale du fichier (taille attendue modeste en usage playtest — pas de lecture en flux
/// inversé optimisée, cf. spec session-log-live-view §Route /events).
pub fn tail_lines(path: &std::path::Path, n: usize) -> std::io::Result<Vec<String>> {
    let content = std::fs::read_to_string(path)?;
    let all: Vec<String> = content.lines().map(str::to_string).collect();
    let start = all.len().saturating_sub(n);
    Ok(all[start..].to_vec())
}

/// Formate un événement en une ligne lisible, préfixée par l'heure HH:MM:SS (UTC, dérivée de
/// `ts_ms`). Pur et testable sans réseau ni fichier.
pub fn format_event(ts_ms: u64, ev: &SessionEvent) -> String {
    let secs_of_day = (ts_ms / 1000) % 86_400;
    let h = secs_of_day / 3600;
    let m = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;
    let time = format!("{h:02}:{m:02}:{s:02}");
    let body = match ev {
        SessionEvent::SessionStart => "SessionStart".to_string(),
        SessionEvent::Connect { client } => format!("Connect · client {client}"),
        SessionEvent::Join { client, name } => format!("Join · client {client} ({name})"),
        SessionEvent::Disconnect { client } => format!("Disconnect · client {client}"),
        SessionEvent::Handoff {
            client,
            from,
            to,
            x,
            y,
            z,
        } => format!("Handoff · client {client} : {from} → {to} (x={x:.1}, y={y:.1}, z={z:.1})"),
        SessionEvent::BufferEnter { client, shard } => {
            format!("BufferEnter · client {client} → {shard}")
        }
        SessionEvent::BufferExit { client, shard } => {
            format!("BufferExit · client {client} ← {shard}")
        }
        SessionEvent::TickStall { micros } => format!("TickStall · {micros}µs"),
        SessionEvent::TimeDrift {
            client,
            server_seconds,
            client_seconds,
            delta_seconds,
        } => format!(
            "TimeDrift · client {client} : serveur={server_seconds}s client={client_seconds}s delta={delta_seconds}s"
        ),
        SessionEvent::AdminAction { actor, action } => format!("AdminAction · {actor} : {action}"),
    };
    format!("{time} — {body}")
}

/// Ligne désérialisée localement — miroir de `session_log::Line`, dupliqué ici plutôt
/// qu'exposé en `pub` depuis `session_log.rs` (pas de raison de rendre ce détail interne public
/// pour ce seul usage de présentation).
#[derive(serde::Deserialize)]
struct RawLine {
    ts_ms: u64,
    #[serde(flatten)]
    ev: SessionEvent,
}

/// Parse une ligne JSONL brute de `session.jsonl` et la formate via `format_event`. Renvoie
/// `None` sur une ligne non-JSON ou dont le format ne correspond à aucune variante connue —
/// ignorée plutôt que de faire planter le flux SSE (une ligne corrompue ne doit jamais arrêter
/// l'affichage des suivantes).
pub fn render_jsonl_line(raw: &str) -> Option<String> {
    let parsed: RawLine = serde_json::from_str(raw).ok()?;
    Some(format_event(parsed.ts_ms, &parsed.ev))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_handoff_with_time_and_positions() {
        let ev = SessionEvent::Handoff {
            client: 42,
            from: "shard-c".into(),
            to: "shard-d".into(),
            x: 120.4,
            y: 88.1,
            z: 3.0,
        };
        // 14:32:07 UTC == 52327 secondes après minuit
        let ts_ms: u64 = 52_327 * 1000;
        let line = format_event(ts_ms, &ev);
        assert_eq!(
            line,
            "14:32:07 — Handoff · client 42 : shard-c → shard-d (x=120.4, y=88.1, z=3.0)"
        );
    }

    #[test]
    fn formats_tick_stall() {
        let ev = SessionEvent::TickStall { micros: 340_000 };
        let line = format_event(0, &ev);
        assert_eq!(line, "00:00:00 — TickStall · 340000µs");
    }

    #[test]
    fn formats_buffer_enter_and_exit() {
        let enter = SessionEvent::BufferEnter {
            client: 7,
            shard: "shard-a".into(),
        };
        let exit = SessionEvent::BufferExit {
            client: 7,
            shard: "shard-a".into(),
        };
        assert!(format_event(0, &enter).contains("BufferEnter · client 7 → shard-a"));
        assert!(format_event(0, &exit).contains("BufferExit · client 7 ← shard-a"));
    }

    #[test]
    fn tail_lines_returns_last_n_in_chronological_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, "line1\nline2\nline3\nline4\nline5\n").unwrap();
        let lines = tail_lines(&path, 3).unwrap();
        assert_eq!(lines, vec!["line3", "line4", "line5"]);
    }

    #[test]
    fn tail_lines_returns_all_when_fewer_than_n() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, "only-one\n").unwrap();
        let lines = tail_lines(&path, 200).unwrap();
        assert_eq!(lines, vec!["only-one"]);
    }

    #[test]
    fn tail_lines_errors_on_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.jsonl");
        assert!(tail_lines(&path, 10).is_err());
    }

    #[test]
    fn render_jsonl_line_parses_a_real_handoff_line() {
        let raw = r#"{"ts_ms":52327000,"event":"handoff","client":42,"from":"shard-c","to":"shard-d","x":120.4,"y":88.1,"z":3.0}"#;
        let rendered = render_jsonl_line(raw).unwrap();
        assert_eq!(
            rendered,
            "14:32:07 — Handoff · client 42 : shard-c → shard-d (x=120.4, y=88.1, z=3.0)"
        );
    }

    #[test]
    fn render_jsonl_line_returns_none_on_garbage() {
        assert_eq!(render_jsonl_line("not json at all"), None);
    }

    #[test]
    fn render_jsonl_line_parses_session_start() {
        let raw = r#"{"ts_ms":0,"event":"session_start"}"#;
        assert_eq!(render_jsonl_line(raw).unwrap(), "00:00:00 — SessionStart");
    }
}
