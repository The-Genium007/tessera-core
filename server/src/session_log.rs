//! Journal de session JSONL du Gateway : la vérité autoritaire de « tout s'est bien passé »
//! pour un playtest — handoffs, zones tampons, stalls (spec playtest-shards §#4).
//! Écrit une ligne JSON par événement ; servi tel quel en HTTP (pattern metrics::serve).

use crate::handoff::Placement;
use serde::Serialize;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SessionEvent {
    SessionStart,
    Connect {
        client: u64,
    },
    Join {
        client: u64,
        name: String,
    },
    Disconnect {
        client: u64,
    },
    Handoff {
        client: u64,
        from: String,
        to: String,
        x: f32,
        y: f32,
        z: f32,
    },
    BufferEnter {
        client: u64,
        shard: String,
    },
    BufferExit {
        client: u64,
        shard: String,
    },
    TickStall {
        micros: u64,
    },
    /// Une commande de gestion admin exécutée avec succès (`/promote`, `/grant`...) — journal
    /// d'audit (spec admin-mode-permissions, Partie 2). `action` porte le texte tapé tel quel.
    AdminAction {
        actor: String,
        action: String,
    },
}

/// Une ligne du journal : horodatage unix (ms) + l'événement aplati (clé "event" + champs).
#[derive(Serialize)]
struct Line<'a> {
    ts_ms: u64,
    #[serde(flatten)]
    ev: &'a SessionEvent,
}

pub struct SessionLog {
    w: std::io::BufWriter<std::fs::File>,
}

impl SessionLog {
    /// Ouvre (en append) et écrit immédiatement un événement `session_start`.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let mut log = Self {
            w: std::io::BufWriter::new(f),
        };
        log.write(&SessionEvent::SessionStart);
        Ok(log)
    }

    /// Écrit une ligne JSON + flush. Les erreurs d'écriture sont volontairement avalées :
    /// le journal ne doit JAMAIS faire tomber la boucle du Gateway.
    pub fn write(&mut self, ev: &SessionEvent) {
        let ts_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if let Ok(line) = serde_json::to_string(&Line { ts_ms, ev }) {
            let _ = writeln!(self.w, "{line}");
            let _ = self.w.flush();
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum PlacementChange {
    Handoff { from: String, to: String },
    BufferEnter { shard: String },
    BufferExit { shard: String },
}

/// Compare deux placements successifs d'un client : bascule d'autoritaire → Handoff,
/// diff des zones tampons → BufferEnter/BufferExit. Pur et testable sans réseau.
pub fn diff_placement(prev: Option<&Placement>, next: &Placement) -> Vec<PlacementChange> {
    let mut out = Vec::new();
    match prev {
        None => {
            for s in &next.overlaps {
                out.push(PlacementChange::BufferEnter { shard: s.clone() });
            }
        }
        Some(p) => {
            if p.authoritative != next.authoritative {
                out.push(PlacementChange::Handoff {
                    from: p.authoritative.clone(),
                    to: next.authoritative.clone(),
                });
            }
            for s in &next.overlaps {
                if !p.overlaps.contains(s) {
                    out.push(PlacementChange::BufferEnter { shard: s.clone() });
                }
            }
            for s in &p.overlaps {
                if !next.overlaps.contains(s) {
                    out.push(PlacementChange::BufferExit { shard: s.clone() });
                }
            }
        }
    }
    out
}

/// Sert le fichier JSONL sur toute requête HTTP (même style minimal que `metrics::serve` :
/// route unique, contenu identique quelle que soit la requête).
pub async fn serve_file(addr: &str, path: std::path::PathBuf) -> std::io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind(addr).await?;
    loop {
        let (mut sock, _) = listener.accept().await?;
        let path = path.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await; // requête ignorée : route unique
            let body = tokio::fs::read(&path).await.unwrap_or_default();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(header.as_bytes()).await;
            let _ = sock.write_all(&body).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handoff::Placement;

    fn p(auth: &str, overlaps: &[&str]) -> Placement {
        Placement {
            authoritative: auth.to_string(),
            overlaps: overlaps.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn first_placement_yields_buffer_enters_only() {
        let next = p("A", &["B"]);
        let changes = diff_placement(None, &next);
        assert_eq!(
            changes,
            vec![PlacementChange::BufferEnter { shard: "B".into() }]
        );
    }

    #[test]
    fn authoritative_change_yields_a_handoff() {
        let prev = p("A", &["B"]);
        let next = p("B", &["A"]);
        let changes = diff_placement(Some(&prev), &next);
        assert!(changes.contains(&PlacementChange::Handoff {
            from: "A".into(),
            to: "B".into()
        }));
        assert!(changes.contains(&PlacementChange::BufferEnter { shard: "A".into() }));
        assert!(changes.contains(&PlacementChange::BufferExit { shard: "B".into() }));
    }

    #[test]
    fn identical_placement_yields_no_change() {
        let prev = p("A", &["B"]);
        assert!(diff_placement(Some(&prev), &p("A", &["B"])).is_empty());
    }

    #[test]
    fn leaving_the_buffer_yields_buffer_exit() {
        let prev = p("A", &["B"]);
        let changes = diff_placement(Some(&prev), &p("A", &[]));
        assert_eq!(
            changes,
            vec![PlacementChange::BufferExit { shard: "B".into() }]
        );
    }

    #[test]
    fn session_log_writes_one_valid_json_line_per_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        {
            let mut log = SessionLog::open(&path).unwrap();
            log.write(&SessionEvent::Handoff {
                client: 42,
                from: "A".into(),
                to: "B".into(),
                x: 1.0,
                y: 2.0,
                z: 3.0,
            });
        }
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        // open() écrit SessionStart, puis notre Handoff.
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["event"], "session_start");
        assert!(first["ts_ms"].as_u64().unwrap() > 0);
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["event"], "handoff");
        assert_eq!(second["client"], 42);
        assert_eq!(second["from"], "A");
        assert_eq!(second["to"], "B");
    }

    #[test]
    fn session_log_writes_admin_action_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        {
            let mut log = SessionLog::open(&path).unwrap();
            log.write(&SessionEvent::AdminAction {
                actor: "Root".into(),
                action: "/promote Compte1 moderator".into(),
            });
        }
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2); // session_start + admin_action
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["event"], "admin_action");
        assert_eq!(second["actor"], "Root");
        assert_eq!(second["action"], "/promote Compte1 moderator");
    }
}
