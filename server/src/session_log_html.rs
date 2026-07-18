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

const HTML_PAGE: &str = r#"<!doctype html>
<html lang="fr">
<head>
<meta charset="utf-8">
<title>Tessera — journal de session en direct</title>
<style>
  body { background: #0b0b0f; color: #d8d8e0; font-family: monospace; margin: 0; padding: 1rem; }
  h1 { font-size: 1rem; color: #8888aa; font-weight: normal; }
  #log { white-space: pre-wrap; word-break: break-word; }
  .handoff, .join, .connect { color: #7ce38b; }
  .tick_stall, .time_drift { color: #666677; }
  .admin_action { color: #e3c17c; }
  .line { border-bottom: 1px solid #1a1a22; padding: 2px 0; }
</style>
</head>
<body>
<h1>Journal de session — flux en direct</h1>
<div id="log"></div>
<script>
  const log = document.getElementById('log');
  const es = new EventSource('/events');
  es.onmessage = (e) => {
    const div = document.createElement('div');
    div.className = 'line';
    div.textContent = e.data;
    log.appendChild(div);
    window.scrollTo(0, document.body.scrollHeight);
  };
</script>
</body>
</html>"#;

/// Sert la page HTML (`GET /`) et le flux SSE (`GET /events`) du journal de session. Route
/// unique déterminée par la première ligne de la requête — pas de framework HTTP, même style
/// minimaliste que `session_log::serve_file`.
pub async fn serve_live(addr: &str, path: std::path::PathBuf) -> std::io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind(addr).await?;
    loop {
        let (mut sock, _) = listener.accept().await?;
        let path = path.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let n = match sock.read(&mut buf).await {
                Ok(n) => n,
                Err(_) => return,
            };
            let request = String::from_utf8_lossy(&buf[..n]);
            let is_events = request.starts_with("GET /events");

            if is_events {
                serve_events_stream(&mut sock, &path).await;
                return;
            }

            let body = HTML_PAGE.as_bytes();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(header.as_bytes()).await;
            let _ = sock.write_all(body).await;
        });
    }
}

/// Sert le flux SSE sur une connexion déjà acceptée : historique (200 dernières lignes) puis
/// poll toutes les 500ms pour les nouvelles lignes (spec session-log-live-view, approche A).
async fn serve_events_stream(sock: &mut tokio::net::TcpStream, path: &std::path::Path) {
    use tokio::io::AsyncWriteExt;
    let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
    if sock.write_all(header.as_bytes()).await.is_err() {
        return;
    }

    let mut sent_lines = tail_lines(path, 200).unwrap_or_default();
    for raw in &sent_lines {
        if let Some(rendered) = render_jsonl_line(raw) {
            if send_sse_line(sock, &rendered).await.is_err() {
                return;
            }
        }
    }
    let mut last_count = sent_lines.len();

    loop {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let all = match tail_lines(path, usize::MAX) {
            Ok(lines) => lines,
            Err(_) => continue,
        };
        if all.len() > last_count {
            for raw in &all[last_count..] {
                if let Some(rendered) = render_jsonl_line(raw) {
                    if send_sse_line(sock, &rendered).await.is_err() {
                        return;
                    }
                }
            }
        }
        last_count = all.len();
        sent_lines = all;
        let _ = &sent_lines; // conservé pour lisibilité du prochain diff, pas de state caché
    }
}

async fn send_sse_line(sock: &mut tokio::net::TcpStream, line: &str) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    // Une ligne SSE ne doit jamais contenir de saut de ligne brut dans `data:` — `render_jsonl_line`
    // ne produit qu'une seule ligne par événement donc ce remplacement est une garde défensive.
    let safe = line.replace('\n', " ");
    sock.write_all(format!("data: {safe}\n\n").as_bytes()).await
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

    #[tokio::test]
    async fn serve_live_returns_html_page_on_root() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, "").unwrap();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener); // libère le port pour serve_live, qui le rebind lui-même

        let addr_str = addr.to_string();
        let server_addr = addr_str.clone();
        let server_path = path.clone();
        tokio::spawn(async move {
            let _ = serve_live(&server_addr, server_path).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let mut stream = tokio::net::TcpStream::connect(&addr_str).await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf[..n]);
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("text/html"));
        assert!(response.contains("EventSource"));
    }

    #[tokio::test]
    async fn serve_live_streams_history_then_new_events_over_sse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"ts_ms":0,"event":"session_start"}
"#,
        )
        .unwrap();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let addr_str = addr.to_string();

        let server_addr = addr_str.clone();
        let server_path = path.clone();
        tokio::spawn(async move {
            let _ = serve_live(&server_addr, server_path).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let mut stream = tokio::net::TcpStream::connect(&addr_str).await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream
            .write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();

        // Historique : la ligne session_start déjà présente doit arriver immédiatement. Le
        // header SSE et la première ligne `data:` sont deux écritures TCP séparées côté serveur
        // (header puis boucle d'historique) : elles peuvent arriver en deux segments distincts
        // au lieu d'un seul, donc on accumule sur plusieurs `read()` jusqu'à voir les deux
        // marqueurs plutôt que de supposer qu'un seul `read()` suffit (race observée : ~2/3 des
        // runs ne recevaient que le header au premier read).
        let mut received = String::new();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let mut buf = vec![0u8; 4096];
            loop {
                let n = stream.read(&mut buf).await.unwrap();
                received.push_str(&String::from_utf8_lossy(&buf[..n]));
                if received.contains("text/event-stream") && received.contains("SessionStart") {
                    break;
                }
            }
        })
        .await
        .expect("le header SSE + l'historique auraient dû arriver sous 2s");
        assert!(received.contains("text/event-stream"));
        assert!(received.contains("SessionStart"));

        // Nouvel événement écrit après coup : doit arriver dans le flux via le prochain poll (500ms).
        use std::io::Write as _;
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(f, r#"{{"ts_ms":0,"event":"connect","client":9}}"#).unwrap();
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let mut buf2 = vec![0u8; 4096];
            loop {
                let n = stream.read(&mut buf2).await.unwrap();
                let chunk = String::from_utf8_lossy(&buf2[..n]);
                if chunk.contains("Connect · client 9") {
                    break;
                }
            }
        })
        .await
        .expect("le nouvel événement aurait dû arriver via le flux SSE sous 2s");
    }
}
