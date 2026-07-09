//! Cœur de relai du Gateway : traduit les événements client (transport GNS) en frames
//! internes vers le Shard, et les `ServerSend` du Shard en envois client. Générique sur le
//! transport client → testable avec `InMemoryTransport`, branché sur `GnsTransport` en prod.

use crate::framing::FrameReader;
use crate::internal_net::{decode_server_send, event_to_client_event_frame};
use crate::transport::{Transport, TransportEvent};
use protocol::{
    CommandResult, CommandResultArgs, Kicked, KickedArgs, PermissionSync, PermissionSyncArgs,
    ServerEnvelope, ServerEnvelopeArgs, ServerMsg, WorldState, WorldStateArgs,
};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Une connexion TCP interne vers un Shard, avec son `FrameReader` de lecture persistant.
pub struct ShardLink {
    sock: TcpStream,
    reader: FrameReader,
}

/// Écrit `frames` vers le shard à `shard_addr`, en connectant si besoin. Une connexion déjà
/// présente dans `shards` mais dont l'écriture échoue est évacuée avant de renvoyer l'erreur —
/// une entrée morte ne doit jamais bloquer une reconnexion au prochain appel.
///
/// Renvoie `true` si cet appel vient de créer la connexion (1re connexion, ou reconnexion après
/// une entrée morte évacuée) — signal utilisé par l'appelant pour re-semer l'état des clients
/// déjà chargés sur ce shard (cf. `reseed_frames_for_reconnected_shard`), puisqu'un shard qui
/// vient d'accepter une nouvelle connexion a perdu tout son état précédent.
pub async fn write_to_shard(
    shards: &mut HashMap<String, ShardLink>,
    shard_addr: &str,
    frames: &[Vec<u8>],
) -> std::io::Result<bool> {
    let created = if !shards.contains_key(shard_addr) {
        let sock = TcpStream::connect(shard_addr).await?;
        shards.insert(
            shard_addr.to_string(),
            ShardLink {
                sock,
                reader: FrameReader::new(),
            },
        );
        true
    } else {
        false
    };
    let result: std::io::Result<()> = async {
        let link = shards.get_mut(shard_addr).unwrap();
        for f in frames {
            link.sock.write_all(f).await?;
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        shards.remove(shard_addr);
        return Err(e);
    }
    Ok(created)
}

/// Lit tout ce qui est disponible sur chaque shard connecté et alimente
/// `latest[client][shard_addr]` avec le dernier `ServerSend` reçu. Pour un même shard, enchaîne
/// les lectures (chacune bornée par un timeout court, pour approcher un `read()` non bloquant)
/// tant que des octets arrivent, au lieu de s'arrêter après une seule — un unique appel de 8192
/// octets max par shard laissait le débit plafonné à ~160 KiB/s/lien, et le retard s'accumulait
/// sans borne dès qu'un shard dépassait ce débit (bug A.2 de l'audit prod du 2026-07-03). Une
/// lecture EOF (`n == 0`) ou en erreur évacue l'entrée du shard concerné — connexion morte, sera
/// recréée au prochain `write_to_shard` pour cette adresse — et purge de `latest`, pour tous les
/// clients, tout snapshot associé à cette adresse : un snapshot laissé en place y serait
/// rediffusé à chaque tick jusqu'à la reconnexion, comme s'il était encore à jour (bug A.1).
pub async fn read_from_shards(
    shards: &mut HashMap<String, ShardLink>,
    latest: &mut HashMap<u64, HashMap<String, Vec<u8>>>,
    current_tick: u64,
    snapshot_ticks: &mut HashMap<u64, HashMap<String, u64>>,
) {
    use crate::internal_net::decode_server_send;

    let addrs: Vec<String> = shards.keys().cloned().collect();
    let mut dead = Vec::new();
    let mut sbuf = [0u8; 8192];
    for addr in addrs {
        let link = shards.get_mut(&addr).unwrap();
        loop {
            match tokio::time::timeout(
                std::time::Duration::from_millis(1),
                link.sock.read(&mut sbuf),
            )
            .await
            {
                Ok(Ok(0)) => {
                    dead.push(addr.clone()); // EOF : le shard a fermé la connexion
                    break;
                }
                Ok(Ok(n)) => {
                    link.reader.push(&sbuf[..n]);
                    if link
                        .reader
                        .declared_len_exceeds(crate::framing::MAX_FRAME_LEN)
                    {
                        dead.push(addr.clone());
                        break;
                    }
                    while let Some(body) = link.reader.next_frame() {
                        if let Some((cid, payload)) = decode_server_send(&body) {
                            latest.entry(cid).or_default().insert(addr.clone(), payload);
                            snapshot_ticks
                                .entry(cid)
                                .or_default()
                                .insert(addr.clone(), current_tick);
                        }
                    }
                    // Continue la boucle : peut-être encore plus à lire sur ce même shard.
                }
                Ok(Err(_)) => {
                    dead.push(addr.clone()); // erreur de lecture : connexion morte
                    break;
                }
                Err(_) => break, // timeout : plus rien à lire pour l'instant sur ce shard
            }
        }
    }
    for addr in &dead {
        shards.remove(addr);
        for per_shard in latest.values_mut() {
            per_shard.remove(addr);
        }
        for per_shard in snapshot_ticks.values_mut() {
            per_shard.remove(addr);
        }
    }
}

/// Calcule l'âge (en ticks) du plus vieux snapshot connu de `snapshot_ticks` par rapport à
/// `current_tick`, et le publie dans `metrics.max_snapshot_age_ticks` — détecte un shard gelé
/// mais toujours connecté (bug non couvert par la purge sur lien mort existante). Extrait en
/// fonction indépendante (plutôt qu'inlinée dans la boucle de `gateway_main`) pour être exercée
/// directement par les tests d'intégration contre le vrai `Metrics`, sans dupliquer le calcul.
pub fn update_snapshot_age_metric(
    snapshot_ticks: &HashMap<u64, HashMap<String, u64>>,
    current_tick: u64,
    metrics: &crate::metrics::Metrics,
) {
    let max_snapshot_age_ticks = snapshot_ticks
        .values()
        .flat_map(|per_shard| per_shard.values())
        .map(|&tick| current_tick.saturating_sub(tick))
        .max()
        .unwrap_or(0);
    metrics
        .max_snapshot_age_ticks
        .store(max_snapshot_age_ticks, std::sync::atomic::Ordering::Relaxed);
}

/// Poll le transport client et renvoie les frames `ClientEvent` à écrire au Shard.
pub fn drain_client_to_shard<T: Transport>(client: &mut T) -> Vec<Vec<u8>> {
    client
        .poll()
        .iter()
        .map(event_to_client_event_frame)
        .collect()
}

/// Encode un `ServerEnvelope{Kicked}` — envoyé à un client juste avant de le déconnecter
/// (serveur plein, flood soutenu...), pour qu'il voie un motif plutôt qu'une coupure muette.
pub fn encode_kicked(reason: &str) -> Vec<u8> {
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let reason = b.create_string(reason);
    let kicked = Kicked::create(
        &mut b,
        &KickedArgs {
            reason: Some(reason),
        },
    );
    let env = ServerEnvelope::create(
        &mut b,
        &ServerEnvelopeArgs {
            msg_type: ServerMsg::Kicked,
            msg: Some(kicked.as_union_value()),
        },
    );
    b.finish(env, None);
    b.finished_data().to_vec()
}

/// Encode un `ServerEnvelope{WorldState}` — horloge/météo monde partagée, diffusée à tous les
/// clients connectés indépendamment du shard (voir `world_clock.rs`).
pub fn encode_world_state(hour: u8, minute: u8, weather: &str) -> Vec<u8> {
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let weather = b.create_string(weather);
    let state = WorldState::create(
        &mut b,
        &WorldStateArgs {
            hour,
            minute,
            weather: Some(weather),
        },
    );
    let env = ServerEnvelope::create(
        &mut b,
        &ServerEnvelopeArgs {
            msg_type: ServerMsg::WorldState,
            msg: Some(state.as_union_value()),
        },
    );
    b.finish(env, None);
    b.finished_data().to_vec()
}

/// Encode un `ServerEnvelope{CommandResult}` — réponse à une commande admin tapée par le client.
pub fn encode_command_result(success: bool, message: &str) -> Vec<u8> {
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let message = b.create_string(message);
    let cr = CommandResult::create(
        &mut b,
        &CommandResultArgs {
            success,
            message: Some(message),
        },
    );
    let env = ServerEnvelope::create(
        &mut b,
        &ServerEnvelopeArgs {
            msg_type: ServerMsg::CommandResult,
            msg: Some(cr.as_union_value()),
        },
    );
    b.finish(env, None);
    b.finished_data().to_vec()
}

/// Encode un `ServerEnvelope{PermissionSync}` — poussé au Join puis à chaque changement de
/// permissions affectant ce compte, pour que le client mette à jour son menu sans reconnexion.
pub fn encode_permission_sync(nodes: &[String]) -> Vec<u8> {
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let node_strs: Vec<_> = nodes.iter().map(|s| b.create_string(s)).collect();
    let nodes_vec = b.create_vector(&node_strs);
    let sync = PermissionSync::create(
        &mut b,
        &PermissionSyncArgs {
            nodes: Some(nodes_vec),
        },
    );
    let env = ServerEnvelope::create(
        &mut b,
        &ServerEnvelopeArgs {
            msg_type: ServerMsg::PermissionSync,
            msg: Some(sync.as_union_value()),
        },
    );
    b.finish(env, None);
    b.finished_data().to_vec()
}

/// Comptes à re-synchroniser (nouveau `PermissionSync`) après une commande admin réussie — soit
/// le compte directement visé (`/promote`, `/grant`...), soit tous les comptes du groupe édité
/// (`/groupgrant`, `/grouprevoke` — leur ensemble effectif de permissions change sans qu'aucun
/// `affected_account` individuel ne soit rapporté par `execute()`). `/deletegroup` n'a jamais
/// besoin de resync : `execute()` le refuse tant qu'un compte porte encore ce groupe.
pub fn accounts_to_resync(
    outcome: &crate::admin_commands::ExecOutcome,
    group_affected: Option<&str>,
    admins: &[crate::permissions::AdminRecord],
) -> Vec<String> {
    if !outcome.success {
        return Vec::new();
    }
    if let Some(account) = &outcome.affected_account {
        return vec![account.clone()];
    }
    if let Some(group) = group_affected {
        return admins
            .iter()
            .filter(|a| a.group == group)
            .map(|a| a.display_name.clone())
            .collect();
    }
    Vec::new()
}

/// Vrai si `issuer` doit être traité comme admin racine (`*`, `Rank::GameMaster`) — soit listé
/// explicitement dans `root_admins` (`TESSERA_ROOT_ADMINS`), soit le bypass temporaire de
/// playtest est actif (`TESSERA_PLAYTEST_ALL_ADMIN=true`) : dans ce cas TOUT compte connecté est
/// root, sans lister le moindre `display_name` — pratique pour un petit groupe de testeurs, à
/// retirer de la variable d'environnement une fois le playtest terminé (jamais persisté, même
/// discipline que `root_admins`).
pub fn resolve_is_root(
    issuer: &str,
    root_admins: &std::collections::HashSet<String>,
    playtest_all_admin: bool,
) -> bool {
    playtest_all_admin || root_admins.contains(issuer)
}

/// Reconstruit, pour chaque client que le Gateway sait chargé sur `shard_addr`, les frames à
/// rejouer vers ce shard après une reconnexion. Le shard vient de perdre tout son état (nouveau
/// `Server::new()` recréé côté Shard à chaque connexion acceptée, cf. `shard_main`) et ne connaît
/// plus aucun de ces clients tant qu'on ne les re-sème pas : sans ça, ils restent invisibles pour
/// les autres joueurs du shard, indéfiniment (bug A.1 de l'audit prod du 2026-07-03). Un client
/// chargé mais sans position connue du Gateway (ne devrait pas arriver : `loaded` n'est peuplé
/// qu'après une 1re position) est ignoré plutôt que de semer une position inventée.
pub fn reseed_frames_for_reconnected_shard(
    loader: &crate::handoff::ShardLoader,
    shard_addr: &str,
    last_pos: &HashMap<u64, [f32; 3]>,
) -> Vec<(u64, Vec<Vec<u8>>)> {
    loader
        .clients_loaded_on(shard_addr)
        .into_iter()
        .filter_map(|cid| {
            let pos = *last_pos.get(&cid)?;
            let mut frames = loader.preamble_frames(cid);
            frames.push(event_to_client_event_frame(&TransportEvent::Message {
                from: cid,
                data: crate::gateway_routing::encode_position_update(pos),
            }));
            Some((cid, frames))
        })
        .collect()
}

/// Sauve la position actuelle de tous les clients connus par nom (rejoints via `keys`) — utilisé
/// à la fois par l'autosave périodique et le flush d'arrêt propre. Un client sans position
/// connue (jamais reçu de `PositionUpdate` depuis son `Join`) n'est pas sauvé.
pub fn save_all_known(
    store: &mut impl crate::persistence::PlayerStore,
    keys: &HashMap<u64, String>,
    last_pos: &HashMap<u64, [f32; 3]>,
    residence: &HashMap<u64, Option<[f32; 3]>>,
) {
    for (cid, name) in keys.iter() {
        if let Some(pos) = last_pos.get(cid).copied() {
            store.save(
                name,
                crate::persistence::PlayerRecord {
                    last_position: pos,
                    residence: residence.get(cid).copied().flatten(),
                },
            );
        }
    }
}

/// Décode un corps `ServerSend` (déjà déframé) reçu du Shard et l'envoie au bon client.
pub fn apply_shard_frame_to_client<T: Transport>(body: &[u8], client: &mut T) {
    if let Some((client_id, payload)) = decode_server_send(body) {
        client.send(client_id, &payload);
    }
}

/// Point d'entrée du Gateway (M4, handoff) : ouvre l'écoute GNS publique et, pour chaque client,
/// calcule à chaque position — via la `ShardTopology` locale + le rayon selon le rang — l'ensemble
/// de shards où le charger (autoritaire + zones tampon). Il diffuse les événements du client à tous
/// ses shards chargés, et **fusionne** les snapshots reçus de ces shards en un seul avant de les
/// renvoyer au client. Le double-chargement près d'une frontière élimine les saccades au transfert.
#[cfg(feature = "gns")]
pub async fn gateway_main(
    listen_addr: &str,
    topology: crate::handoff::ShardTopology,
    radius: crate::handoff::RadiusPolicy,
    mut store: crate::persistence::FileStore,
    mut admin_store: crate::admin_store::AdminStore,
    spawn: [f32; 3],
    max_players: u32,
) -> std::io::Result<()> {
    use crate::admin_commands::{execute as execute_admin_command, parse as parse_admin_command};
    use crate::gateway_routing::{
        extract_admin_command, extract_join_name, extract_position, extract_time_report,
    };
    use crate::gns_transport::GnsTransport;
    use crate::handoff::{LoadAction, Rank, ShardLoader};
    use crate::permissions::{derive_rank, resolve_permissions};
    use crate::persistence::{resolve_spawn, PlayerRecord, PlayerStore};
    use crate::rate_limit::{
        check_rate_limit, RateDecision, RateLimitState, DEFAULT_KICK_AFTER_WINDOWS,
        DEFAULT_LIMIT_PER_WINDOW,
    };
    use crate::shutdown::ShutdownSignal;
    use crate::snapshot_merge::merge_snapshots;
    use crate::transport::{Transport, TransportEvent};
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::time::Duration;

    let mut shards: HashMap<String, ShardLink> = HashMap::new();
    let mut loader = ShardLoader::new();
    // Dernier snapshot reçu de chaque shard, par client : latest[client][shard_addr] = payload.
    let mut latest: HashMap<u64, HashMap<String, Vec<u8>>> = HashMap::new();
    // Tick numéro de chaque snapshot : snapshot_ticks[client][shard_addr] = tick_received.
    let mut snapshot_ticks: HashMap<u64, HashMap<String, u64>> = HashMap::new();
    // Rang par client (Player par défaut, dérivé des permissions résolues via `root_admins`/
    // `admin_store` au Join — voir plus bas).
    let mut ranks: HashMap<u64, Rank> = HashMap::new();
    // Cache en mémoire des permissions résolues par client — chargé une fois au Join, jamais
    // relu du disque à chaque tick (coût nul sur la boucle anti-triche). Servira aux futures
    // vérifications de capacité (fly, noclip...) une fois ces chantiers choisis dans le
    // catalogue (spec admin-mode-permissions, Partie 5) — non consommé en phase 1 au-delà de la
    // dérivation de `Rank` et de l'affichage du menu client.
    let mut permissions: HashMap<u64, Vec<String>> = HashMap::new();
    // Persistance : clé (display_name), dernière position, et résidence chargée — par client.
    let mut keys: HashMap<u64, String> = HashMap::new();
    let mut last_pos: HashMap<u64, [f32; 3]> = HashMap::new();
    // Horodatage de la dernière PositionUpdate ACCEPTÉE par client (absent tant qu'aucune
    // position n'a encore été acceptée depuis le Join — sert de garde anti-triche).
    let mut last_pos_at: HashMap<u64, std::time::Instant> = HashMap::new();
    // Dernière fois qu'on a loggé le contournement anti-triche GameMaster pour ce client (2026-07-07,
    // rapporté en playtest) : sans throttle, un GameMaster en mouvement spamme un WARN à chaque
    // PositionUpdate (plusieurs par seconde) — noie le reste des logs, y compris les Handoff qu'on
    // veut justement pouvoir suivre. Une ligne au plus toutes les BYPASS_LOG_INTERVAL suffit à
    // documenter que le contournement est actif sans inonder la sortie.
    let mut bypass_warned_at: HashMap<u64, std::time::Instant> = HashMap::new();
    const BYPASS_LOG_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
    let mut residence: HashMap<u64, Option<[f32; 3]>> = HashMap::new();
    // Fenêtre de rate-limit par client (audit prod 2026-07-03 §5.4).
    let mut rate_states: HashMap<u64, RateLimitState> = HashMap::new();

    let sock: SocketAddr = listen_addr.parse().expect("adresse GNS invalide");
    let mut client =
        GnsTransport::listen(sock.ip(), sock.port()).expect("GnsTransport::listen failed");
    tracing::info!(
        "Gateway handoff : écoute GNS sur {listen_addr} ({} shards)",
        topology.shards.len()
    );

    let metrics = crate::metrics::Metrics::new();
    {
        let metrics = metrics.clone();
        let metrics_addr = std::env::var("TESSERA_GATEWAY_METRICS_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9100".to_string());
        tokio::spawn(async move {
            if let Err(e) = crate::metrics::serve(&metrics_addr, metrics).await {
                tracing::warn!("endpoint métriques indisponible ({metrics_addr}): {e}");
            }
        });
    }

    // Admins racine (spec admin-mode-permissions, Partie 1) : liste de comptes qui reçoivent
    // implicitement toutes les permissions (`*`), amorcée par variable d'environnement — jamais
    // stockée en base, jamais rétrogradable par une commande. Remplace le stub
    // `TESSERA_GAMEMASTER_NAMES` (2026-07-06/07) dont la portée dépassait maintenant le seul
    // bypass anti-triche. Vide par défaut (comportement inchangé) ; ne PAS committer de vrai nom
    // en dur, ça reste une variable d'environnement sur le déploiement de test uniquement.
    let root_admins: std::collections::HashSet<String> = std::env::var("TESSERA_ROOT_ADMINS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Bypass temporaire de playtest (2026-07-08) : tout compte connecté devient admin racine,
    // sans lister le moindre `display_name` — pratique le temps d'un petit groupe de testeurs.
    // Vide/absent par défaut (comportement inchangé) ; à retirer de l'environnement du
    // déploiement une fois le playtest terminé, même discipline que `TESSERA_ROOT_ADMINS`.
    let playtest_all_admin = std::env::var("TESSERA_PLAYTEST_ALL_ADMIN")
        .map(|v| v.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    // Journal de session (spec playtest-shards §#4) : vérité autoritaire des handoffs/stalls.
    let session_log_path =
        std::env::var("TESSERA_SESSION_LOG_PATH").unwrap_or_else(|_| "session.jsonl".to_string());
    let mut slog =
        match crate::session_log::SessionLog::open(std::path::Path::new(&session_log_path)) {
            Ok(l) => Some(l),
            Err(e) => {
                tracing::warn!("journal de session indisponible ({session_log_path}): {e}");
                None
            }
        };
    {
        let addr = std::env::var("TESSERA_GATEWAY_SESSIONLOG_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9102".to_string());
        let path = std::path::PathBuf::from(session_log_path.clone());
        tokio::spawn(async move {
            if let Err(e) = crate::session_log::serve_file(&addr, path).await {
                tracing::warn!("endpoint journal de session indisponible ({addr}): {e}");
            }
        });
    }
    // Dernier placement connu par client — pour détecter handoffs et zones tampons.
    let mut prev_placements: HashMap<u64, crate::handoff::Placement> = HashMap::new();

    let mut ticker = tokio::time::interval(Duration::from_millis(50));
    // Cf. shard.rs : Skip plutôt que le Burst par défaut — sauter un tick manqué au lieu de
    // rattraper en rafale, pour ne pas dépenser plus de CPU/réseau juste après un pic de charge.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_autosave = std::time::Instant::now();
    let autosave_interval = Duration::from_secs(30);

    // Horloge/météo monde partagée (spec M6 full-inventory §2). Valeurs de départ/échelle/météo
    // en dur pour l'instant (pas encore exposées au manifeste — amélioration future notée, comme
    // pour `DesossageConfig` côté client) : PIN IN-GAME, le nom de record météo exact reste à
    // confirmer avant que ça ait un effet visible (une chaîne invalide échoue proprement côté
    // client, `SetWeather` renvoie `false`, cf. `world_clock.rs`).
    let mut world_clock = crate::world_clock::WorldClock::new(12, 0);
    const WORLD_TIME_SCALE: f64 = 1.0; // 1 minute de jeu par seconde réelle (cycle 24h ~ 24 min réelles)
    const WORLD_WEATHER: &str = "Weather.Sunny01";
    const WORLD_TICK_DT: Duration = Duration::from_millis(50); // cadence fixe du ticker ci-dessous
    let mut last_world_broadcast = std::time::Instant::now();
    const WORLD_BROADCAST_INTERVAL: Duration = Duration::from_secs(2);
    // Diagnostic playtest (2026-07-07) : au-delà de cette dérive (secondes), on remonte un
    // tracing::warn en plus de la ligne session_log — en-deçà, une dérive de 1-2s est tolérée
    // (attendue : le client ré-applique l'heure toutes les WORLD_BROADCAST_INTERVAL, pas en continu).
    const TIME_DRIFT_WARN_THRESHOLD_SECS: i32 = 2;
    // Enregistré UNE SEULE FOIS avant la boucle : sur Unix, `tokio::signal::unix::signal(...)`
    // installe une registration OS qui ne bufferise rien tant qu'aucun récepteur n'existe — la
    // recréer à chaque itération (comme le faisait l'ancien `shutdown_signal()` appelé depuis le
    // `select!` ci-dessous) crée une fenêtre où un SIGTERM reçu pendant la partie synchrone de
    // l'itération (après le drop de l'ancien flux, avant la création du nouveau) est perdu — le
    // process attend alors un 2e signal. Le flux persistant survit à toutes les itérations.
    let mut shutdown = ShutdownSignal::new();
    let mut current_tick: u64 = 0;
    loop {
        // 1) Lire chaque shard connecté (évacue et laisse reconnecter les connexions mortes).
        read_from_shards(&mut shards, &mut latest, current_tick, &mut snapshot_ticks).await;

        // 2) Tick, avec une course contre le signal d'arrêt propre (SIGTERM/SIGINT).
        tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown.recv() => {
                save_all_known(&mut store, &keys, &last_pos, &residence);
                tracing::info!("Arrêt propre : positions sauvegardées, extinction du Gateway");
                return Ok(());
            }
        }
        let iter_start = std::time::Instant::now();
        world_clock.advance(WORLD_TICK_DT, WORLD_TIME_SCALE);
        for ev in client.poll() {
            let cid = match &ev {
                TransportEvent::Connected(id) | TransportEvent::Disconnected(id) => *id,
                TransportEvent::Message { from, .. } => *from,
            };
            let is_disconnect = matches!(ev, TransportEvent::Disconnected(_));

            if let Some(sl) = slog.as_mut() {
                match &ev {
                    TransportEvent::Connected(id) => {
                        sl.write(&crate::session_log::SessionEvent::Connect { client: *id })
                    }
                    TransportEvent::Disconnected(id) => {
                        sl.write(&crate::session_log::SessionEvent::Disconnect { client: *id })
                    }
                    TransportEvent::Message { .. } => {}
                }
            }

            // Rate-limit : chaque message compte contre la fenêtre de CE client, avant tout
            // autre traitement — sinon un flood de PositionUpdate amplifie gratuitement vers
            // l'interne (locate() + écritures shards par message, audit prod 2026-07-03 §5.4).
            if matches!(ev, TransportEvent::Message { .. }) {
                let now = std::time::Instant::now();
                let state = rate_states
                    .entry(cid)
                    .or_insert_with(|| RateLimitState::new(now));
                match check_rate_limit(
                    state,
                    now,
                    DEFAULT_LIMIT_PER_WINDOW,
                    DEFAULT_KICK_AFTER_WINDOWS,
                ) {
                    RateDecision::Accept => {}
                    RateDecision::Drop => {
                        tracing::warn!(client = cid, "message ignoré (rate-limit)");
                        continue;
                    }
                    RateDecision::Kick => {
                        tracing::warn!(client = cid, "kick : flood soutenu (rate-limit)");
                        client.send(cid, &encode_kicked("flood"));
                        client.disconnect(cid);
                        if let Some(name) = keys.remove(&cid) {
                            if let Some(pos) = last_pos.get(&cid).copied() {
                                store.save(
                                    &name,
                                    PlayerRecord {
                                        last_position: pos,
                                        residence: residence.get(&cid).copied().flatten(),
                                    },
                                );
                            }
                        }
                        last_pos.remove(&cid);
                        last_pos_at.remove(&cid);
                        bypass_warned_at.remove(&cid);
                        residence.remove(&cid);
                        ranks.remove(&cid);
                        permissions.remove(&cid);
                        rate_states.remove(&cid);
                        loader.forget(cid);
                        latest.remove(&cid);
                        prev_placements.remove(&cid);
                        continue;
                    }
                }
            }

            // Décoder ce que porte un message client : Join → identité + résolution de spawn ;
            // PositionUpdate → placement (topologie + rang) et mémorisation de la dernière position.
            let mut placement = None;
            if let TransportEvent::Message { data, .. } = &ev {
                if let Some(name) = extract_join_name(data) {
                    if !name.is_empty() {
                        if !keys.contains_key(&cid) && keys.len() >= max_players as usize {
                            tracing::warn!(client = cid, max_players, "kick : serveur plein");
                            client.send(cid, &encode_kicked("serveur plein"));
                            client.disconnect(cid);
                            rate_states.remove(&cid);
                            continue;
                        }
                        let record = store.load(&name);
                        let (pos, source) = resolve_spawn(record.as_ref(), spawn);
                        tracing::info!(
                            "Connexion de {name} : placement décidé {pos:?} (source: {source:?})"
                        );
                        residence.insert(cid, record.and_then(|r| r.residence));
                        last_pos.insert(cid, pos); // départ tant qu'aucune position réelle
                        let is_root = resolve_is_root(&name, &root_admins, playtest_all_admin);
                        let admin_record = admin_store
                            .admins
                            .iter()
                            .find(|a| a.display_name == name)
                            .cloned();
                        let resolved = resolve_permissions(
                            is_root,
                            admin_record.as_ref(),
                            &admin_store.groups,
                        );
                        let rank = derive_rank(&resolved);
                        if rank != Rank::Player {
                            tracing::info!(client = cid, %name, ?rank, "rang attribué");
                            ranks.insert(cid, rank);
                        }
                        if !resolved.is_empty() {
                            client.send(cid, &encode_permission_sync(&resolved));
                        }
                        permissions.insert(cid, resolved);
                        if let Some(sl) = slog.as_mut() {
                            sl.write(&crate::session_log::SessionEvent::Join {
                                client: cid,
                                name: name.clone(),
                            });
                        }
                        keys.insert(cid, name);
                    }
                } else if let Some((x, y, z)) = extract_position(data) {
                    let now = std::time::Instant::now();
                    let bypassed = matches!(ranks.get(&cid), Some(Rank::GameMaster));
                    let plausible = if bypassed {
                        let should_log = match bypass_warned_at.get(&cid) {
                            Some(at) => now.duration_since(*at) >= BYPASS_LOG_INTERVAL,
                            None => true,
                        };
                        if should_log {
                            bypass_warned_at.insert(cid, now);
                            tracing::warn!(
                                client = cid,
                                "PositionUpdate accepté sans vérification (contournement anti-triche playtest, log throttled {BYPASS_LOG_INTERVAL:?})"
                            );
                        }
                        true
                    } else {
                        match (last_pos.get(&cid).copied(), last_pos_at.get(&cid).copied()) {
                            (Some(prev), Some(at)) => crate::anticheat::is_plausible_move(
                                prev,
                                [x, y, z],
                                crate::anticheat::cap_elapsed(
                                    now.duration_since(at),
                                    crate::anticheat::MAX_ELAPSED_WINDOW,
                                ),
                                crate::anticheat::MAX_PLAYER_SPEED_MPS,
                            ),
                            // Pas encore de référence temporelle (1re position après Join) : accepté.
                            _ => true,
                        }
                    };
                    if !plausible {
                        tracing::warn!(client = cid, "PositionUpdate rejeté (vitesse implausible)");
                        continue;
                    }
                    last_pos.insert(cid, [x, y, z]);
                    last_pos_at.insert(cid, now);
                    let r = radius.radius_for(*ranks.get(&cid).unwrap_or(&Rank::Player));
                    placement = Some(topology.locate(x, y, r));
                    if let (Some(sl), Some(next)) = (slog.as_mut(), placement.as_ref()) {
                        for c in crate::session_log::diff_placement(prev_placements.get(&cid), next)
                        {
                            use crate::session_log::{PlacementChange, SessionEvent};
                            let ev = match c {
                                PlacementChange::Handoff { from, to } => SessionEvent::Handoff {
                                    client: cid,
                                    from,
                                    to,
                                    x,
                                    y,
                                    z,
                                },
                                PlacementChange::BufferEnter { shard } => {
                                    SessionEvent::BufferEnter { client: cid, shard }
                                }
                                PlacementChange::BufferExit { shard } => {
                                    SessionEvent::BufferExit { client: cid, shard }
                                }
                            };
                            // En plus du journal JSONL (fichier, pas exploitable sans accès au
                            // volume monté), une ligne tracing pour ce même événement : visible
                            // dans les logs stdout du conteneur, donc récupérable à distance via
                            // l'API Dokploy (compose.readLogs) sans SSH — utile pour suivre les
                            // franchissements de shard en direct pendant un playtest.
                            let name = keys.get(&cid).map(String::as_str).unwrap_or("?");
                            match &ev {
                                crate::session_log::SessionEvent::Handoff { from, to, .. } => {
                                    tracing::info!(
                                        client = cid,
                                        %name,
                                        "Handoff : {name} passe de {from} à {to} ({x:.1}, {y:.1}, {z:.1})"
                                    );
                                }
                                crate::session_log::SessionEvent::BufferEnter { shard, .. } => {
                                    tracing::info!(client = cid, %name, "{name} entre en zone tampon de {shard}");
                                }
                                crate::session_log::SessionEvent::BufferExit { shard, .. } => {
                                    tracing::info!(client = cid, %name, "{name} sort de la zone tampon de {shard}");
                                }
                                _ => {}
                            }
                            sl.write(&ev);
                        }
                        prev_placements.insert(cid, next.clone());
                    }
                } else if let Some((h, m, s)) = extract_time_report(data) {
                    // Diagnostic playtest, pas un mécanisme correctif (cf. constante ci-dessus) :
                    // compare l'heure rapportée par CE client à l'horloge autoritaire du serveur.
                    let server_secs = world_clock.total_seconds_since_midnight();
                    let client_secs = (h as u32) * 3600 + (m as u32) * 60 + (s as u32);
                    let delta = client_secs as i32 - server_secs as i32;
                    if delta.unsigned_abs() as i32 > TIME_DRIFT_WARN_THRESHOLD_SECS {
                        let name = keys.get(&cid).map(String::as_str).unwrap_or("?");
                        tracing::warn!(
                            client = cid,
                            %name,
                            delta,
                            "dérive horloge monde au-delà de la tolérance ({TIME_DRIFT_WARN_THRESHOLD_SECS}s)"
                        );
                    }
                    if let Some(sl) = slog.as_mut() {
                        sl.write(&crate::session_log::SessionEvent::TimeDrift {
                            client: cid,
                            server_seconds: server_secs,
                            client_seconds: client_secs,
                            delta_seconds: delta,
                        });
                    }
                } else if let Some(text) = extract_admin_command(data) {
                    let issuer = keys.get(&cid).cloned().unwrap_or_default();
                    let is_root = resolve_is_root(&issuer, &root_admins, playtest_all_admin);
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    let parsed = parse_admin_command(&text);
                    let group_affected: Option<String> = match &parsed {
                        Ok(crate::admin_commands::ParsedCommand::GroupGrant { group, .. })
                        | Ok(crate::admin_commands::ParsedCommand::GroupRevoke { group, .. }) => {
                            Some(group.clone())
                        }
                        _ => None,
                    };
                    let outcome = match parsed {
                        Ok(cmd) => execute_admin_command(
                            cmd,
                            is_root,
                            &mut admin_store.groups,
                            &mut admin_store.admins,
                            now_ms,
                            &issuer,
                        ),
                        Err(_) => crate::admin_commands::ExecOutcome {
                            success: false,
                            message: "commande invalide".to_string(),
                            affected_account: None,
                        },
                    };
                    if outcome.success {
                        admin_store.save_groups();
                        admin_store.save_admins();
                        if let Some(sl) = slog.as_mut() {
                            sl.write(&crate::session_log::SessionEvent::AdminAction {
                                actor: issuer.clone(),
                                action: text.clone(),
                            });
                        }
                        tracing::info!(client = cid, actor = %issuer, ?text, "commande admin exécutée");
                    } else {
                        tracing::warn!(
                            client = cid, actor = %issuer, ?text, message = ?outcome.message,
                            "commande admin refusée"
                        );
                    }
                    client.send(
                        cid,
                        &encode_command_result(outcome.success, &outcome.message),
                    );
                    let to_resync = accounts_to_resync(
                        &outcome,
                        group_affected.as_deref(),
                        &admin_store.admins,
                    );
                    for target in &to_resync {
                        if let Some((&target_cid, _)) = keys.iter().find(|(_, n)| *n == target) {
                            let is_target_root =
                                resolve_is_root(target, &root_admins, playtest_all_admin);
                            let target_record = admin_store
                                .admins
                                .iter()
                                .find(|a| &a.display_name == target)
                                .cloned();
                            let resolved = resolve_permissions(
                                is_target_root,
                                target_record.as_ref(),
                                &admin_store.groups,
                            );
                            ranks.insert(target_cid, derive_rank(&resolved));
                            permissions.insert(target_cid, resolved.clone());
                            client.send(target_cid, &encode_permission_sync(&resolved));
                        }
                    }
                }
            }

            for LoadAction::Forward { shard, frames } in loader.feed(ev, placement) {
                if let Ok(true) = write_to_shard(&mut shards, &shard, &frames).await {
                    // Le shard vient de (re)connecter : côté Shard, `Server::new()` est recréé à
                    // chaque connexion acceptée (cf. `shard_main`) — tout son état précédent est
                    // perdu. Re-semer le préambule + dernière position connue de chaque client que
                    // le Gateway sait chargé sur ce shard, sinon ils y restent invisibles pour
                    // toujours (bug A.1, audit prod 2026-07-03). Idempotent : `World::add_player`
                    // (`or_default`) et `set_pose` tolèrent un double envoi sans effet de bord.
                    let reseed_frames =
                        reseed_frames_for_reconnected_shard(&loader, &shard, &last_pos);
                    if !reseed_frames.is_empty() {
                        tracing::warn!(
                            shard = %shard,
                            reseeded_clients = reseed_frames.len(),
                            "shard réinitialisé après reconnexion : clients re-semés"
                        );
                    }
                    for (_, seed_frames) in reseed_frames {
                        let _ = write_to_shard(&mut shards, &shard, &seed_frames).await;
                    }
                }
            }

            if is_disconnect {
                // Sauver la dernière position connue avant d'oublier le client.
                if let Some(name) = keys.remove(&cid) {
                    if let Some(pos) = last_pos.get(&cid).copied() {
                        store.save(
                            &name,
                            PlayerRecord {
                                last_position: pos,
                                residence: residence.get(&cid).copied().flatten(),
                            },
                        );
                        tracing::info!("Sauvegarde de {name} à {pos:?}");
                    }
                }
                last_pos.remove(&cid);
                last_pos_at.remove(&cid);
                bypass_warned_at.remove(&cid);
                ranks.remove(&cid);
                permissions.remove(&cid);
                residence.remove(&cid);
                rate_states.remove(&cid);
                loader.forget(cid);
                latest.remove(&cid);
                snapshot_ticks.remove(&cid);
                prev_placements.remove(&cid);
            } else if let Some(per_shard) = latest.get_mut(&cid) {
                // Élaguer les snapshots des shards qui ne sont plus chargés pour ce client.
                let loaded = loader.loaded_shards(cid);
                per_shard.retain(|s, _| loaded.contains(s));
                if let Some(ticks) = snapshot_ticks.get_mut(&cid) {
                    ticks.retain(|s, _| loaded.contains(s));
                }
            }
        }

        // 3) Pour chaque client, fusionner les derniers snapshots de ses shards chargés → envoi.
        for (cid, per_shard) in latest.iter() {
            let snaps: Vec<Vec<u8>> = per_shard.values().cloned().collect();
            if let Some(merged) = merge_snapshots(&snaps) {
                client.send(*cid, &merged);
            }
        }
        metrics
            .players
            .store(latest.len() as u64, std::sync::atomic::Ordering::Relaxed);
        metrics
            .shards_loaded
            .store(shards.len() as u64, std::sync::atomic::Ordering::Relaxed);

        // Calculer l'âge du plus vieux snapshot rediffusé — détecte un shard gelé mais toujours
        // connecté (bug non couvert par la purge sur lien mort existante).
        update_snapshot_age_metric(&snapshot_ticks, current_tick, &metrics);

        // 3bis) Horloge/météo monde — diffusion périodique à tous les clients connus (pas à
        // 20 Hz comme les snapshots : l'heure/la météo n'a pas besoin de cette fréquence).
        if last_world_broadcast.elapsed() >= WORLD_BROADCAST_INTERVAL {
            let payload =
                encode_world_state(world_clock.hour(), world_clock.minute(), WORLD_WEATHER);
            for cid in latest.keys() {
                client.send(*cid, &payload);
            }
            last_world_broadcast = std::time::Instant::now();
        }

        // 4) Autosave périodique — ne dépend pas d'une déconnexion propre.
        if last_autosave.elapsed() >= autosave_interval {
            save_all_known(&mut store, &keys, &last_pos, &residence);
            last_autosave = std::time::Instant::now();
        }

        // Stall : une itération complète (poll + routage + merge + envois) au-delà de 100 ms
        // (2× le budget de tick 50 ms) mérite une trace — c'est le « gel » vécu par les joueurs.
        let iter_micros = iter_start.elapsed().as_micros() as u64;
        if iter_micros > 100_000 {
            if let Some(sl) = slog.as_mut() {
                sl.write(&crate::session_log::SessionEvent::TickStall {
                    micros: iter_micros,
                });
            }
        }

        current_tick += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::FrameReader;
    use crate::internal_net::decode_client_event;
    use crate::transport::{InMemoryTransport, Transport, TransportEvent};

    #[test]
    fn encode_kicked_produces_a_server_envelope_carrying_the_reason() {
        let payload = encode_kicked("serveur plein");
        let env = flatbuffers::root::<protocol::ServerEnvelope>(&payload).unwrap();
        assert_eq!(env.msg_type(), ServerMsg::Kicked);
        let kicked = env.msg_as_kicked().unwrap();
        assert_eq!(kicked.reason(), Some("serveur plein"));
    }

    #[test]
    fn encode_world_state_carries_hour_minute_and_weather() {
        let payload = encode_world_state(14, 30, "Weather.Sunny01");
        let env = flatbuffers::root::<protocol::ServerEnvelope>(&payload).unwrap();
        assert_eq!(env.msg_type(), ServerMsg::WorldState);
        let state = env.msg_as_world_state().unwrap();
        assert_eq!(state.hour(), 14);
        assert_eq!(state.minute(), 30);
        assert_eq!(state.weather(), Some("Weather.Sunny01"));
    }

    #[test]
    fn encode_command_result_round_trips() {
        let bytes = encode_command_result(true, "Compte1 promu");
        let env = flatbuffers::root::<protocol::ServerEnvelope>(&bytes).unwrap();
        assert_eq!(env.msg_type(), protocol::ServerMsg::CommandResult);
        let cr = env.msg_as_command_result().unwrap();
        assert!(cr.success());
        assert_eq!(cr.message().unwrap(), "Compte1 promu");
    }

    #[test]
    fn encode_permission_sync_round_trips() {
        let bytes = encode_permission_sync(&["admin.fly".to_string(), "admin.noclip".to_string()]);
        let env = flatbuffers::root::<protocol::ServerEnvelope>(&bytes).unwrap();
        assert_eq!(env.msg_type(), protocol::ServerMsg::PermissionSync);
        let sync = env.msg_as_permission_sync().unwrap();
        let nodes: Vec<&str> = sync.nodes().unwrap().iter().collect();
        assert_eq!(nodes, vec!["admin.fly", "admin.noclip"]);
    }

    #[test]
    fn drains_client_events_into_shard_frames() {
        let mut client = InMemoryTransport::new();
        client.inject(TransportEvent::Connected(1));
        client.inject(TransportEvent::Message {
            from: 1,
            data: vec![4, 2],
        });

        let frames = drain_client_to_shard(&mut client);
        assert_eq!(frames.len(), 2);

        // Chaque frame est un ClientEvent décodable.
        let mut r = FrameReader::new();
        for f in &frames {
            r.push(f);
        }
        assert_eq!(
            decode_client_event(&r.next_frame().unwrap()),
            Some(TransportEvent::Connected(1))
        );
        assert_eq!(
            decode_client_event(&r.next_frame().unwrap()),
            Some(TransportEvent::Message {
                from: 1,
                data: vec![4, 2]
            })
        );
    }

    #[test]
    fn applies_shard_serversend_to_the_right_client() {
        // Un ServerSend{client 9, payload [7,7]} arrive du Shard ; il doit partir au client 9.
        let mut shard_side = InMemoryTransport::new(); // sert juste à produire un ServerSend framé
        use crate::internal_net::InternalTransport;
        let mut it = InternalTransport::new();
        it.send(9, &[7, 7]);
        let framed = it.take_outbound().remove(0);
        let mut r = FrameReader::new();
        r.push(&framed);
        let body = r.next_frame().unwrap();

        let mut client = InMemoryTransport::new();
        apply_shard_frame_to_client(&body, &mut client);
        assert_eq!(client.take_sent(9), vec![vec![7, 7]]);
        let _ = &mut shard_side;
    }

    fn join_payload() -> Vec<u8> {
        let mut b = flatbuffers::FlatBufferBuilder::new();
        let name = b.create_string("v");
        let join = protocol::Join::create(
            &mut b,
            &protocol::JoinArgs {
                display_name: Some(name),
            },
        );
        let env = protocol::ClientEnvelope::create(
            &mut b,
            &protocol::ClientEnvelopeArgs {
                msg_type: protocol::ClientMsg::Join,
                msg: Some(join.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn reseed_frames_reconstruct_preamble_and_last_position_for_every_loaded_client() {
        use crate::handoff::Placement;
        use crate::transport::TransportEvent;

        let mut loader = crate::handoff::ShardLoader::new();
        loader.feed(TransportEvent::Connected(1), None);
        loader.feed(
            TransportEvent::Message {
                from: 1,
                data: join_payload(),
            },
            None,
        );
        loader.feed(
            TransportEvent::Message {
                from: 1,
                data: crate::gateway_routing::encode_position_update([500.0, 0.0, 0.0]),
            },
            Some(Placement {
                authoritative: "A".to_string(),
                overlaps: vec![],
            }),
        );

        let mut last_pos = HashMap::new();
        last_pos.insert(1u64, [500.0, 0.0, 0.0]);

        let seeded = reseed_frames_for_reconnected_shard(&loader, "A", &last_pos);
        assert_eq!(seeded.len(), 1);
        let (cid, frames) = &seeded[0];
        assert_eq!(*cid, 1);
        assert_eq!(frames.len(), 3); // Connected + Join + Position
    }

    #[test]
    fn reseed_frames_skips_a_loaded_client_with_no_known_position() {
        use crate::handoff::Placement;
        use crate::transport::TransportEvent;

        let mut loader = crate::handoff::ShardLoader::new();
        loader.feed(
            TransportEvent::Message {
                from: 1,
                data: crate::gateway_routing::encode_position_update([500.0, 0.0, 0.0]),
            },
            Some(Placement {
                authoritative: "A".to_string(),
                overlaps: vec![],
            }),
        );

        let last_pos: HashMap<u64, [f32; 3]> = HashMap::new(); // aucune position connue du Gateway
        assert!(reseed_frames_for_reconnected_shard(&loader, "A", &last_pos).is_empty());
    }

    #[tokio::test]
    async fn write_to_shard_reports_whether_it_created_a_new_connection() {
        use std::collections::HashMap;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 64];
                loop {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            }
        });

        let mut shards: HashMap<String, ShardLink> = HashMap::new();

        let created_first = write_to_shard(&mut shards, &addr, &[b"a".to_vec()])
            .await
            .expect("1re écriture doit réussir");
        assert!(created_first, "la 1re écriture crée forcément la connexion");

        let created_second = write_to_shard(&mut shards, &addr, &[b"b".to_vec()])
            .await
            .expect("2e écriture doit réussir");
        assert!(
            !created_second,
            "une connexion déjà vivante ne doit pas être signalée comme nouvelle"
        );
    }

    /// Bug A.2 de l'audit prod 2026-07-03 : `read_from_shards` ne faisait qu'UN SEUL `read()`
    /// (max 8192 octets) par shard et par appel. Sous un débit soutenu, le retard s'accumule
    /// sans borne au fil des ticks — ce test le prouve en un seul appel : le "shard" envoie
    /// d'un coup bien plus de 8192 octets de frames avant que le Gateway ne lise quoi que ce
    /// soit ; un seul appel à `read_from_shards` doit malgré tout TOUT drainer.
    #[tokio::test]
    async fn read_from_shards_drains_more_than_one_socket_buffer_in_a_single_call() {
        use crate::internal_net::InternalTransport;
        use std::collections::HashMap;
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        const N: u64 = 300; // ~300 × (32 + enveloppe) octets ≫ 8192

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut it = InternalTransport::new();
            for cid in 0..N {
                it.send(cid, &[0u8; 32]);
            }
            for frame in it.take_outbound() {
                sock.write_all(&frame).await.unwrap();
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await; // garde la connexion ouverte
        });

        let mut shards: HashMap<String, ShardLink> = HashMap::new();
        write_to_shard(&mut shards, &addr, &[]).await.unwrap();

        // Laisse le temps aux 300 frames d'atterrir dans le buffer kernel du socket Gateway
        // AVANT le premier (et unique) appel à read_from_shards.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut latest: HashMap<u64, HashMap<String, Vec<u8>>> = HashMap::new();
        let mut snapshot_ticks: HashMap<u64, HashMap<String, u64>> = HashMap::new();
        read_from_shards(&mut shards, &mut latest, 0, &mut snapshot_ticks).await;

        assert_eq!(
            latest.len(),
            N as usize,
            "un seul appel doit drainer TOUTES les frames disponibles, pas juste ~8192 octets"
        );
    }

    #[tokio::test]
    async fn dead_shard_link_purges_its_stale_snapshots_from_latest_for_every_client() {
        use std::collections::HashMap;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            drop(sock); // ferme aussitôt : simule un shard qui vient de crasher
        });

        let mut shards: HashMap<String, ShardLink> = HashMap::new();
        write_to_shard(&mut shards, &addr, &[b"a".to_vec()])
            .await
            .unwrap();

        // Deux clients ont chacun un snapshot périmé en attente pour ce shard, plus un snapshot
        // d'un AUTRE shard qui doit survivre à la purge (seule l'adresse morte est concernée).
        let mut latest: HashMap<u64, HashMap<String, Vec<u8>>> = HashMap::new();
        latest
            .entry(1u64)
            .or_default()
            .insert(addr.clone(), b"perime-1".to_vec());
        latest
            .entry(1u64)
            .or_default()
            .insert("autre-shard".to_string(), b"toujours-valide".to_vec());
        latest
            .entry(2u64)
            .or_default()
            .insert(addr.clone(), b"perime-2".to_vec());

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mut snapshot_ticks: HashMap<u64, HashMap<String, u64>> = HashMap::new();
        read_from_shards(&mut shards, &mut latest, 0, &mut snapshot_ticks).await;

        assert!(
            !latest.get(&1).unwrap().contains_key(&addr),
            "le snapshot périmé du client 1 pour le shard mort doit être purgé"
        );
        assert!(
            latest.get(&1).unwrap().contains_key("autre-shard"),
            "le snapshot d'un shard toujours vivant ne doit pas être touché"
        );
        assert!(
            !latest.contains_key(&2) || !latest.get(&2).unwrap().contains_key(&addr),
            "le snapshot périmé du client 2 pour le shard mort doit être purgé"
        );
    }

    /// Reproduit le bug A.1 de bout en bout, sans GNS : un shard "crashe" (ferme sa connexion),
    /// redémarre sur la même adresse (comme un conteneur Docker relancé), et un 2e client
    /// déclenche une nouvelle écriture vers ce shard. Le shard frais ne connaît plus le 1er
    /// client — il doit être re-semé (Connected+Join+Position), sinon il reste invisible pour
    /// toujours pour les autres joueurs de ce shard, silencieusement.
    #[tokio::test]
    async fn shard_reconnect_reseeds_every_previously_loaded_client() {
        use crate::handoff::{LoadAction, Placement, ShardLoader};
        use crate::transport::TransportEvent;
        use std::collections::HashMap;
        use tokio::net::TcpListener;

        let listener1 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener1.local_addr().unwrap().to_string();
        let accept1 = tokio::spawn(async move {
            let (sock, _) = listener1.accept().await.unwrap();
            drop(sock); // le shard "crashe" aussitôt après avoir accepté le client 1
        });

        let mut shards: HashMap<String, ShardLink> = HashMap::new();
        let mut loader = ShardLoader::new();
        let mut last_pos: HashMap<u64, [f32; 3]> = HashMap::new();

        // Client 1 rejoint et se place sur le shard "A" — écriture normale vers le shard n°1.
        loader.feed(TransportEvent::Connected(1), None);
        loader.feed(
            TransportEvent::Message {
                from: 1,
                data: join_payload(),
            },
            None,
        );
        last_pos.insert(1, [500.0, 0.0, 0.0]);
        for LoadAction::Forward { shard, frames } in loader.feed(
            TransportEvent::Message {
                from: 1,
                data: crate::gateway_routing::encode_position_update([500.0, 0.0, 0.0]),
            },
            Some(Placement {
                authoritative: addr.clone(),
                overlaps: vec![],
            }),
        ) {
            write_to_shard(&mut shards, &shard, &frames).await.unwrap();
        }
        accept1.await.unwrap();

        // Le shard n°1 meurt : EOF détecté, connexion évacuée.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mut latest: HashMap<u64, HashMap<String, Vec<u8>>> = HashMap::new();
        let mut snapshot_ticks: HashMap<u64, HashMap<String, u64>> = HashMap::new();
        read_from_shards(&mut shards, &mut latest, 0, &mut snapshot_ticks).await;
        assert!(!shards.contains_key(&addr));

        // Le shard redémarre sur la MÊME adresse et capture tout ce qu'il reçoit. Le Gateway
        // écrit en 2 appels séparés (frames du client 2, puis re-seed du client 1) qui peuvent
        // arriver en 2 segments TCP distincts : accumuler jusqu'à 6 frames décodables (3+3) ou
        // un timeout, plutôt qu'un seul `read()` qui capturerait parfois seulement le 1er lot.
        let listener2 = TcpListener::bind(&addr).await.unwrap();
        let (recv_tx, recv_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut sock, _) = listener2.accept().await.unwrap();
            let mut reader = FrameReader::new();
            let mut events = Vec::new();
            let mut buf = [0u8; 4096];
            while events.len() < 6 {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                reader.push(&buf[..n]);
                while let Some(body) = reader.next_frame() {
                    if let Some(ev) = crate::internal_net::decode_client_event(&body) {
                        events.push(ev);
                    }
                }
            }
            let _ = recv_tx.send(events);
        });

        // Client 2 arrive et se place aussi sur le shard "A" — déclenche la reconnexion.
        loader.feed(TransportEvent::Connected(2), None);
        loader.feed(
            TransportEvent::Message {
                from: 2,
                data: join_payload(),
            },
            None,
        );
        last_pos.insert(2, [510.0, 0.0, 0.0]);
        for LoadAction::Forward { shard, frames } in loader.feed(
            TransportEvent::Message {
                from: 2,
                data: crate::gateway_routing::encode_position_update([510.0, 0.0, 0.0]),
            },
            Some(Placement {
                authoritative: addr.clone(),
                overlaps: vec![],
            }),
        ) {
            let reconnected = write_to_shard(&mut shards, &shard, &frames).await.unwrap();
            if reconnected {
                for (_, seed_frames) in
                    reseed_frames_for_reconnected_shard(&loader, &shard, &last_pos)
                {
                    write_to_shard(&mut shards, &shard, &seed_frames)
                        .await
                        .unwrap();
                }
            }
        }

        let events = tokio::time::timeout(std::time::Duration::from_secs(2), recv_rx)
            .await
            .expect("le shard frais doit recevoir les 6 frames attendues (3+3) sous 2s")
            .unwrap();
        assert!(
            events.contains(&TransportEvent::Connected(1)),
            "le client 1 (jamais revenu lui-même) doit être re-semé au shard frais par le Gateway ; reçu {events:?}"
        );
    }

    #[tokio::test]
    async fn evicts_dead_shard_link_and_reconnects_once_a_new_listener_is_up() {
        use std::collections::HashMap;
        use std::time::Duration;
        use tokio::net::TcpListener;

        let listener1 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener1.local_addr().unwrap().to_string();

        // "Shard" n°1 : accepte une connexion puis se ferme aussitôt (simule un crash).
        tokio::spawn(async move {
            let (sock, _) = listener1.accept().await.unwrap();
            drop(sock);
            drop(listener1); // libère le port pour le "redémarrage" ci-dessous
        });

        let mut shards: HashMap<String, ShardLink> = HashMap::new();
        let mut latest: HashMap<u64, HashMap<String, Vec<u8>>> = HashMap::new();
        let mut snapshot_ticks: HashMap<u64, HashMap<String, u64>> = HashMap::new();

        write_to_shard(&mut shards, &addr, &[b"a".to_vec()])
            .await
            .expect("1re connexion doit réussir");
        assert!(shards.contains_key(&addr));

        // Laisse le "shard" fermer, puis détecte l'EOF côté lecture.
        tokio::time::sleep(Duration::from_millis(100)).await;
        read_from_shards(&mut shards, &mut latest, 0, &mut snapshot_ticks).await;
        assert!(
            !shards.contains_key(&addr),
            "la connexion morte doit être évacuée après EOF"
        );

        // "Shard" n°2 redémarre à la MÊME adresse (comme un conteneur Docker relancé).
        let listener2 = TcpListener::bind(&addr)
            .await
            .expect("le port doit être libre après le drop du 1er listener");
        let accept2 = tokio::spawn(async move {
            listener2.accept().await.unwrap();
        });

        // La prochaine écriture doit reconnecter automatiquement, sans intervention.
        write_to_shard(&mut shards, &addr, &[b"b".to_vec()])
            .await
            .expect("la reconnexion automatique doit réussir");
        assert!(shards.contains_key(&addr));
        accept2.await.unwrap();
    }

    fn resync_test_admin(name: &str, group: &str) -> crate::permissions::AdminRecord {
        crate::permissions::AdminRecord {
            display_name: name.to_string(),
            group: group.to_string(),
            extra_permissions: vec![],
            revoked_permissions: vec![],
            granted_at: 0,
            granted_by: "Root".to_string(),
        }
    }

    #[test]
    fn accounts_to_resync_returns_nothing_on_failed_outcome() {
        let outcome = crate::admin_commands::ExecOutcome {
            success: false,
            message: "x".into(),
            affected_account: None,
        };
        let admins = vec![resync_test_admin("A", "moderator")];
        assert!(accounts_to_resync(&outcome, Some("moderator"), &admins).is_empty());
    }

    #[test]
    fn accounts_to_resync_returns_only_the_directly_affected_account() {
        let outcome = crate::admin_commands::ExecOutcome {
            success: true,
            message: "x".into(),
            affected_account: Some("Compte1".to_string()),
        };
        let admins = vec![
            resync_test_admin("Compte1", "moderator"),
            resync_test_admin("Compte2", "moderator"),
        ];
        assert_eq!(
            accounts_to_resync(&outcome, None, &admins),
            vec!["Compte1".to_string()]
        );
    }

    #[test]
    fn accounts_to_resync_returns_every_member_of_an_edited_group() {
        let outcome = crate::admin_commands::ExecOutcome {
            success: true,
            message: "x".into(),
            affected_account: None,
        };
        let admins = vec![
            resync_test_admin("Compte1", "moderator"),
            resync_test_admin("Compte2", "moderator"),
            resync_test_admin("Compte3", "admin"),
        ];
        let mut resynced = accounts_to_resync(&outcome, Some("moderator"), &admins);
        resynced.sort();
        assert_eq!(resynced, vec!["Compte1".to_string(), "Compte2".to_string()]);
    }

    #[test]
    fn accounts_to_resync_returns_nothing_when_no_account_matches_the_edited_group() {
        let outcome = crate::admin_commands::ExecOutcome {
            success: true,
            message: "x".into(),
            affected_account: None,
        };
        let admins = vec![resync_test_admin("Compte1", "admin")];
        assert!(accounts_to_resync(&outcome, Some("moderator"), &admins).is_empty());
    }

    #[test]
    fn resolve_is_root_grants_listed_root_admins() {
        let root_admins: std::collections::HashSet<String> =
            ["Compte1".to_string()].into_iter().collect();
        assert!(resolve_is_root("Compte1", &root_admins, false));
    }

    #[test]
    fn resolve_is_root_denies_unlisted_accounts_by_default() {
        let root_admins: std::collections::HashSet<String> =
            ["Compte1".to_string()].into_iter().collect();
        assert!(!resolve_is_root("Compte2", &root_admins, false));
    }

    #[test]
    fn resolve_is_root_grants_everyone_when_playtest_bypass_is_active() {
        let root_admins: std::collections::HashSet<String> = std::collections::HashSet::new();
        assert!(resolve_is_root("AnyoneAtAll", &root_admins, true));
    }

    #[test]
    fn resolve_is_root_bypass_does_not_remove_the_listed_root_admins() {
        let root_admins: std::collections::HashSet<String> =
            ["Compte1".to_string()].into_iter().collect();
        assert!(resolve_is_root("Compte1", &root_admins, true));
    }

    #[test]
    fn save_all_known_saves_every_client_with_a_known_position() {
        use crate::persistence::{MemoryStore, PlayerRecord, PlayerStore};

        let mut store = MemoryStore::new();
        let mut keys = HashMap::new();
        keys.insert(1u64, "Alice".to_string());
        keys.insert(2u64, "Bob".to_string());
        let mut last_pos = HashMap::new();
        last_pos.insert(1u64, [10.0, 20.0, 30.0]);
        // Bob n'a jamais bougé depuis le Join : pas de position connue, pas sauvé.
        let residence: HashMap<u64, Option<[f32; 3]>> = HashMap::new();

        save_all_known(&mut store, &keys, &last_pos, &residence);

        assert_eq!(
            store.load("Alice"),
            Some(PlayerRecord {
                last_position: [10.0, 20.0, 30.0],
                residence: None,
            })
        );
        assert_eq!(
            store.load("Bob"),
            None,
            "un client sans position connue ne doit pas être sauvé"
        );
    }

    /// Teste qu'un shard gelé (connecté mais sans nouvelles données) est détecté via la métrique
    /// de péremption des snapshots — cas non couvert par la purge existante sur lien mort.
    #[tokio::test]
    async fn snapshot_age_metric_increases_when_shard_stops_updating() {
        use crate::internal_net::InternalTransport;
        use std::collections::HashMap;
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        // Le "shard" envoie un seul snapshot puis s'arrête sans fermer la connexion.
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();

            // Envoyer un snapshot pour le client 1
            let mut it = InternalTransport::new();
            it.send(1, &[42u8; 32]); // snapshot simple
            for frame in it.take_outbound() {
                sock.write_all(&frame).await.unwrap();
            }

            // Garder la connexion ouverte sans envoyer d'autres données (le shard est gelé)
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        });

        let mut shards: HashMap<String, ShardLink> = HashMap::new();
        let mut latest: HashMap<u64, HashMap<String, Vec<u8>>> = HashMap::new();
        let mut snapshot_ticks: HashMap<u64, HashMap<String, u64>> = HashMap::new();

        // Tick 0 : lire le snapshot initial
        write_to_shard(&mut shards, &addr, &[]).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        read_from_shards(&mut shards, &mut latest, 0, &mut snapshot_ticks).await;

        // Vérifier que nous avons le snapshot
        assert!(latest.contains_key(&1), "snapshot devrait être reçu");
        assert_eq!(
            snapshot_ticks
                .get(&1)
                .unwrap()
                .get(&addr)
                .copied()
                .unwrap_or(0),
            0,
            "snapshot devrait être marqué avec le tick 0"
        );

        // Ticks suivants : relire sans recevoir de nouvelles données
        // Le snapshot devient plus ancien à chaque tick
        for tick in 1..=5 {
            read_from_shards(&mut shards, &mut latest, tick, &mut snapshot_ticks).await;

            // Le snapshot doit toujours être présent (pas de EOF, juste pas de nouvelles données)
            assert!(
                latest.contains_key(&1),
                "snapshot ne doit pas être purgé juste parce qu'il est vieux"
            );

            let snapshot_tick = snapshot_ticks
                .get(&1)
                .and_then(|per_shard| per_shard.get(&addr))
                .copied()
                .unwrap_or(0);
            assert_eq!(
                snapshot_tick, 0,
                "snapshot ne doit pas être mis à jour sans nouvelles données"
            );
        }

        // Exercer le vrai chemin de code de production : le même `update_snapshot_age_metric`
        // appelé depuis la boucle de `gateway_main`, contre un vrai `Metrics`. Un test qui
        // recalculerait l'âge localement ici ne détecterait pas une régression dans le calcul
        // réel (mauvais `Ordering`, mauvais champ, min/max inversé, `.store()` supprimé...).
        let metrics = crate::metrics::Metrics::new();
        update_snapshot_age_metric(&snapshot_ticks, 5, &metrics);
        let max_age = metrics
            .max_snapshot_age_ticks
            .load(std::sync::atomic::Ordering::Relaxed);

        assert!(
            max_age > 0,
            "snapshot age should be > 0 after 5 ticks without updates (got {max_age})"
        );
        assert_eq!(
            max_age, 5,
            "snapshot age should be exactly 5 ticks (tick 5 - tick 0)"
        );
    }
}
