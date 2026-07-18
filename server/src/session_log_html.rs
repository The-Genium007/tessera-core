//! Formatage lisible + service HTTP (page HTML + SSE) du journal de session pour consultation
//! en direct pendant un playtest, sans accès SSH au volume monté (spec session-log-live-view,
//! 2026-07-18). Se limite à la présentation ; l'écriture/lecture brute reste dans
//! `session_log.rs`.

use crate::session_log::SessionEvent;

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
}
