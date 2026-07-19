//! Cœur de relai du Gateway : traduit les événements client (transport GNS) en frames
//! internes vers le Shard, et les `ServerSend` du Shard en envois client. Générique sur le
//! transport client → testable avec `InMemoryTransport`, branché sur `GnsTransport` en prod.

use crate::framing::FrameReader;
use crate::internal_net::{decode_server_send, event_to_client_event_frame};
use crate::transport::{Transport, TransportEvent};
use protocol::{
    CommandResult, CommandResultArgs, Kicked, KickedArgs, PermissionSync, PermissionSyncArgs,
    PositionCorrection, PositionCorrectionArgs, ServerEnvelope, ServerEnvelopeArgs, ServerMsg,
    ShardAssignment, ShardAssignmentArgs, Vec3, WorldState, WorldStateArgs,
};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Client_id OIDC du client natif `launcher` de l'instance officielle TesseraSynth — audience
/// (`aud`) attendue dans l'`id_token` au Join sur un serveur public. Un client_id OIDC public/natif
/// n'est **pas un secret** : il transite en clair dans l'URL d'autorisation ZITADEL (visible côté
/// navigateur à chaque login). Il est donc épinglé en dur, exactement comme `ZITADEL_JWKS_URL`
/// (bin/gateway.rs) — l'auth du jeu est déjà mono-instance. Surchargeable par
/// `TESSERA_ZITADEL_LAUNCHER_CLIENT_ID` pour un opérateur tiers ayant sa propre app ZITADEL (cf.
/// `docs/architecture/0010-launcher-oidc-audience-pinned.md`). DOIT rester égal au secret de build
/// `TESSERASYNTH_ZITADEL_CLIENT_ID` du launcher (c'est ce même client_id qui signe l'`aud`).
///
/// `#[cfg(feature = "gns")]` : consommé uniquement par `gateway_main` (lui-même gns-gated) —
/// sans ce garde, la const serait « dead code » en build par défaut.
#[cfg(feature = "gns")]
const DEFAULT_LAUNCHER_CLIENT_ID: &str = "381763954952634746";

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

/// Encode un `ServerEnvelope{PositionCorrection}`. `reason` : 0=Spawn, 1=AntiCheat, 2=Resync —
/// TOUJOURS explicite (le défaut FlatBuffers 0 mentirait silencieusement). Le client SE PLACE à
/// `position`/`yaw` à la réception (téléportation), quel que soit `reason`.
pub fn encode_position_correction(pos: [f32; 3], yaw: f32, reason: u8) -> Vec<u8> {
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let v = Vec3::new(pos[0], pos[1], pos[2]);
    let pc = PositionCorrection::create(
        &mut b,
        &PositionCorrectionArgs {
            position: Some(&v),
            yaw,
            reason,
        },
    );
    let env = ServerEnvelope::create(
        &mut b,
        &ServerEnvelopeArgs {
            msg_type: ServerMsg::PositionCorrection,
            msg: Some(pc.as_union_value()),
        },
    );
    b.finish(env, None);
    b.finished_data().to_vec()
}

/// Encode un `ServerEnvelope{ShardAssignment}` — le placement autoritaire décidé par le serveur
/// pour CE client (topology.locate), poussé au HUD pour qu'il compare à son calcul local et
/// détecte un décalage (spec HUD moniteur de cohérence, 2026-07-18).
pub fn encode_shard_assignment(authoritative: &str, overlaps: &[String]) -> Vec<u8> {
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let authoritative_str = b.create_string(authoritative);
    let overlap_strs: Vec<_> = overlaps.iter().map(|s| b.create_string(s)).collect();
    let overlaps_vec = b.create_vector(&overlap_strs);
    let sa = ShardAssignment::create(
        &mut b,
        &ShardAssignmentArgs {
            authoritative: Some(authoritative_str),
            overlaps: Some(overlaps_vec),
        },
    );
    let env = ServerEnvelope::create(
        &mut b,
        &ServerEnvelopeArgs {
            msg_type: ServerMsg::ShardAssignment,
            msg: Some(sa.as_union_value()),
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

/// Vrai si le compte doit être traité comme admin racine, en priorisant le `sub` OIDC vérifié
/// quand il est disponible (serveur public), avec repli sur `display_name` sinon (Task D3 —
/// migration de l'indexation admin vers `sub`, ferme le bug playtest 1 : deux comptes distincts
/// avec le même `display_name` ne doivent jamais partager d'autorité admin). `sub` est `None` sur
/// un serveur privé (`identity.public = false`) — comportement alors strictement identique à
/// `resolve_is_root` seule. Le repli sur `display_name` reste actif même quand `sub` est fourni :
/// migration progressive, un opérateur qui a listé un display_name dans `TESSERA_ROOT_ADMINS`
/// avant cette tâche ne perd pas son accès root tant qu'il n'a pas basculé vers le `sub`.
pub fn is_root_by_sub_or_display_name(
    sub: Option<&str>,
    display_name: &str,
    root_admins: &std::collections::HashSet<String>,
    playtest_all_admin: bool,
) -> bool {
    if let Some(sub) = sub {
        if resolve_is_root(sub, root_admins, playtest_all_admin) {
            return true;
        }
    }
    resolve_is_root(display_name, root_admins, playtest_all_admin)
}

/// Résout l'`AdminRecord` d'un compte en priorisant une recherche par `sub` OIDC vérifié quand il
/// est disponible, avec repli sur `display_name` (Task D3 — même logique et même justification
/// que `is_root_by_sub_or_display_name`). Le repli couvre le cas d'un compte promu par `/promote`
/// avant son premier Join sur un serveur public : son `AdminRecord` existe déjà (créé par
/// `admin_commands.rs`, toujours avec `sub: None`) mais n'a pas encore été enrichi du `sub`
/// découvert au Join — sans repli, cet admin perdrait son autorité tant qu'il n'a pas rejoint.
///
/// **Garde anti-collision (root cause du bug playtest 1)** : le repli sur `display_name` ne
/// matche JAMAIS un enregistrement dont le `sub` est déjà renseigné ET différent du `sub`
/// recherché — sinon un second compte revendiquant un display_name déjà lié à un autre `sub`
/// hériterait silencieusement de l'`AdminRecord` (et donc des permissions/rang) du premier. Le
/// repli ne s'applique que quand l'enregistrement candidat n'a pas encore de `sub` connu
/// (`sub: None`, admin jamais vu sur un serveur public) ou quand on ne connaît pas nous-mêmes de
/// `sub` pour ce client (serveur privé).
pub fn resolve_admin_record<'a>(
    sub: Option<&str>,
    display_name: &str,
    admins: &'a [crate::permissions::AdminRecord],
) -> Option<&'a crate::permissions::AdminRecord> {
    if let Some(s) = sub {
        if let Some(found) = admins.iter().find(|a| a.sub.as_deref() == Some(s)) {
            return Some(found);
        }
        return admins
            .iter()
            .find(|a| a.display_name == display_name && a.sub.is_none());
    }
    admins.iter().find(|a| a.display_name == display_name)
}

/// Décide, au moment du `Join`, la clé effective de persistance (`store.load`/`store.save`) pour
/// ce client (design 2026-07-09, launcher-server-auth §4) :
/// - Serveur privé (`identity_public = false`, défaut) : comportement historique inchangé, le
///   `display_name` fourni devient la clé, `token` est ignoré intégralement.
/// - Serveur public (`identity_public = true`) : un token JWT non vide est exigé et vérifié via
///   `JwksCache::verify` (Task C1) ; le `sub` OIDC vérifié devient la clé — jamais le
///   `display_name` libre non vérifié, root cause du bug playtest 1 (deux comptes distincts avec
///   le même display_name partageaient silencieusement un enregistrement).
///
/// `expected_aud` est l'audience OIDC attendue dans le token : le **client_id ZITADEL du launcher**
/// (config `TESSERA_ZITADEL_LAUNCHER_CLIENT_ID`, lue dans `gateway_main`). Le launcher envoie son
/// `id_token`, dont le `aud` vaut son propre client_id — d'où la nécessité de le passer en config
/// plutôt que de coder une chaîne en dur (l'ancien `"launcher"` était un placeholder jamais
/// réconcilié avec un vrai client_id, donc tout token réel était rejeté en `WrongAudience`).
///
/// `Err` porte le message `Kicked` à renvoyer au client avant de couper la connexion (token
/// absent ou invalide) — jamais un timeout silencieux.
/// Identité résolue au Join. `key` = clé de persistance (`sub` OIDC vérifié sur serveur public,
/// `display_name` brut du client sur serveur privé). `display` = nom d'affichage
/// **server-autoritaire** : dérivé du JWT vérifié sur serveur public (`Claims::display_name`,
/// jamais le username Windows envoyé par le client), = `display_name` brut sur serveur privé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinIdentity {
    pub key: String,
    pub display: String,
}

pub fn resolve_join_key(
    identity_public: bool,
    name: &str,
    token: &str,
    expected_aud: &str,
    jwks_cache: &crate::jwks::JwksCache,
) -> Result<JoinIdentity, &'static str> {
    if !identity_public {
        // Serveur privé (petit groupe de confiance) : pas de JWT ; le `display_name` brut du client
        // sert de clé ET de nom affiché — comportement historique strictement inchangé.
        return Ok(JoinIdentity {
            key: name.to_string(),
            display: name.to_string(),
        });
    }
    let token = token.trim();
    if token.is_empty() {
        return Err("compte requis sur ce serveur");
    }
    match jwks_cache.verify(token, expected_aud) {
        // Serveur public : clé = `sub` (unique par compte), nom affiché = dérivé du JWT vérifié
        // (`name` > `preferred_username` > `sub`). Le `Join.display_name` du client (username
        // Windows via GetUserNameA côté netcode) est IGNORÉ : non fiable, usurpable, collisionnant.
        Ok(claims) => Ok(JoinIdentity {
            display: claims.display_name(),
            key: claims.sub,
        }),
        Err(_) => Err("session invalide, reconnectez-vous"),
    }
}

/// Décide, au moment du `Join`, la position de spawn d'un client — en essayant D'ABORD le
/// `HotStateCache` (état chaud, reprise rapide après un redémarrage/reconnexion du Gateway,
/// Décision 3 du design stockage 2026-07-09) avant de retomber sur le store froid
/// (`store.load` → `resolve_spawn`, comportement historique inchangé).
///
/// Une entrée hot-cache présente pour `effective_key` GAGNE toujours sur le store froid : elle
/// est par construction plus récente (écrite en continu à chaque `PositionUpdate` accepté,
/// TTL 120s) alors que le store froid n'est mis à jour qu'au Join/Leave/déconnexion/autosave.
/// Absence de hot-cache (jamais écrit, expiré, ou erreur Redis quelconque — connexion perdue,
/// etc.) dégrade gracieusement vers le repli habituel, jamais un échec du Join : une erreur de
/// lecture Redis est donc traitée exactement comme une absence, pas remontée à l'appelant.
///
/// Extrait de `gateway_main` pour être testable indépendamment de la boucle complète — voir le
/// test `resolve_join_spawn` de reprise ci-dessous (Redis réel peuplé, store froid vide).
pub async fn resolve_join_spawn(
    effective_key: &str,
    hot_state: &crate::hot_state_cache::HotStateCache,
    cold_record: Option<&crate::persistence::PlayerRecord>,
    spawn: [f32; 3],
) -> ([f32; 3], crate::persistence::SpawnSource) {
    match hot_state.read(effective_key).await {
        Ok(Some(pos)) => (pos, crate::persistence::SpawnSource::LastPosition),
        Ok(None) => crate::persistence::resolve_spawn(cold_record, spawn),
        Err(e) => {
            tracing::warn!(
                "HotStateCache::read échoué (subject={effective_key}): {e:?} — repli sur le store froid"
            );
            crate::persistence::resolve_spawn(cold_record, spawn)
        }
    }
}

/// Décide, au moment du `Join`, si la version de protocole annoncée par le client (Task C3) est
/// compatible avec ce serveur. `Err` porte le message `Kicked` à renvoyer — un client dont le
/// protocole a dérivé (launcher pas à jour) doit recevoir une explication lisible plutôt qu'un
/// comportement indéfini ou une coupure muette.
pub fn resolve_protocol_version(received: u32) -> Result<(), &'static str> {
    if received != crate::gateway_routing::CURRENT_PROTOCOL_VERSION {
        Err("version du jeu incompatible, mettez à jour votre launcher")
    } else {
        Ok(())
    }
}

/// Décide, au moment du `Join`, si `name` (display_name brut, jamais la clé de persistance
/// effective) est autorisé à rejoindre au regard de la whitelist du manifeste (`runtime.whitelist`
/// et `runtime.whitelist_names`, Task C3). `whitelist` désactivée (défaut) : toujours `Ok` — la
/// whitelist ne change AUCUN comportement tant que l'opérateur ne l'active pas explicitement.
/// `Err` porte le message `Kicked` à renvoyer.
pub fn resolve_whitelist(
    whitelist_enabled: bool,
    whitelist_names: &std::collections::HashSet<String>,
    name: &str,
) -> Result<(), &'static str> {
    if whitelist_enabled && !whitelist_names.contains(name) {
        Err("accès restreint (whitelist)")
    } else {
        Ok(())
    }
}

/// Nettoie tout l'état par-client (`cid`) que le Gateway maintient en mémoire, et sauvegarde sa
/// dernière position connue avant de l'oublier — chemin PARTAGÉ entre une déconnexion (crash/coupure
/// réseau, `TransportEvent::Disconnected`) et un départ volontaire (`Leave`, Task C3) : les deux
/// libèrent aujourd'hui le slot du client de façon identique et immédiate. Le jour où palier 2
/// distingue les deux (réservation de slot 5 min après un `Disconnected`, libération immédiate
/// après un `Leave`), seul l'appelant du chemin `Disconnected` ajoutera cette réservation — ce
/// nettoyage per-cid, lui, restera commun aux deux.
#[allow(clippy::too_many_arguments)] // état per-cid éclaté en plusieurs HashMaps (même discipline que gateway_main)
pub fn cleanup_client_state(
    cid: u64,
    store: &mut impl crate::persistence::PlayerStore,
    keys: &mut HashMap<u64, String>,
    display_names: &mut HashMap<u64, String>,
    last_pos: &mut HashMap<u64, [f32; 3]>,
    last_pos_at: &mut HashMap<u64, std::time::Instant>,
    bypass_warned_at: &mut HashMap<u64, std::time::Instant>,
    anomaly_trackers: &mut HashMap<u64, AnomalyTracker>,
    ranks: &mut HashMap<u64, crate::handoff::Rank>,
    permissions: &mut HashMap<u64, Vec<String>>,
    residence: &mut HashMap<u64, Option<[f32; 3]>>,
    rate_states: &mut HashMap<u64, crate::rate_limit::RateLimitState>,
    loader: &mut crate::handoff::ShardLoader,
    latest: &mut HashMap<u64, HashMap<String, Vec<u8>>>,
    prev_placements: &mut HashMap<u64, crate::handoff::Placement>,
) {
    if let Some(name) = keys.remove(&cid) {
        if let Some(pos) = last_pos.get(&cid).copied() {
            store.save(
                &name,
                crate::persistence::PlayerRecord {
                    last_position: pos,
                    residence: residence.get(&cid).copied().flatten(),
                },
            );
        }
    }
    display_names.remove(&cid);
    last_pos.remove(&cid);
    last_pos_at.remove(&cid);
    bypass_warned_at.remove(&cid);
    anomaly_trackers.remove(&cid);
    ranks.remove(&cid);
    permissions.remove(&cid);
    residence.remove(&cid);
    rate_states.remove(&cid);
    loader.forget(cid);
    latest.remove(&cid);
    prev_placements.remove(&cid);
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

/// Applique une `RateDecision` déjà calculée par `check_rate_limit` (rate_limit.rs, seuils
/// inchangés) : ne réévalue rien, se contente de la traduire en effets observables. `Accept` ne
/// fait rien ; `Drop` ignore le message (log warn) sans kick ni métrique — comportement identique
/// à aujourd'hui ; `Kick` (flood soutenu) logge, incrémente `metrics.rejected_messages_total`
/// (métrique F3), envoie `encode_kicked("flood")` puis déconnecte le client via `T: Transport`.
/// Renvoie `true` si le message qui a produit cette décision doit être considéré comme consommé
/// (l'appelant doit `continue` sans le traiter plus loin) — c'est le cas pour `Drop` et `Kick`,
/// pas pour `Accept`. Ne touche PAS l'état per-cid au-delà du rate-limit lui-même (`rate_states`
/// est géré par l'appelant, comme avant) ; le nettoyage complet (keys/last_pos/ranks/...) après un
/// `Kick` reste à la charge de l'appelant via `cleanup_client_state`, pour garder cette fonction
/// scopée à "décision + métrique + kick réseau" (voir gateway_main).
pub fn apply_rate_limit_decision<T: Transport>(
    decision: crate::rate_limit::RateDecision,
    cid: u64,
    metrics: &crate::metrics::Metrics,
    client: &mut T,
) -> bool {
    use crate::rate_limit::RateDecision;
    match decision {
        RateDecision::Accept => false,
        RateDecision::Drop => {
            tracing::warn!(client = cid, "message ignoré (rate-limit)");
            true
        }
        RateDecision::Kick => {
            tracing::warn!(client = cid, "kick : flood soutenu (rate-limit)");
            metrics
                .rejected_messages_total
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            client.send(cid, &encode_kicked("flood"));
            client.disconnect(cid);
            true
        }
    }
}

/// Décide si un `Join` doit être refusé parce que le serveur est plein : `cid` n'a pas encore de
/// slot (`!keys_contains_cid`) et le nombre de joueurs actuels a déjà atteint `max_players`. En
/// cas de refus, logge, incrémente `metrics.rejected_messages_total` (métrique F3), envoie
/// `encode_kicked("serveur plein")` puis déconnecte le client via `T: Transport`. Ne touche PAS
/// `rate_states` (géré par l'appelant, comme avant) ni aucune autre map per-cid : un serveur plein
/// refuse un client qui n'a jamais eu de slot, donc rien d'autre à nettoyer.
pub fn reject_join_if_server_full<T: Transport>(
    keys_contains_cid: bool,
    keys_len: usize,
    max_players: u32,
    cid: u64,
    metrics: &crate::metrics::Metrics,
    client: &mut T,
) -> bool {
    if !keys_contains_cid && keys_len >= max_players as usize {
        tracing::warn!(client = cid, max_players, "kick : serveur plein");
        metrics
            .rejected_messages_total
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        client.send(cid, &encode_kicked("serveur plein"));
        client.disconnect(cid);
        true
    } else {
        false
    }
}

/// Nombre d'anomalies (zone orange) dans `ANOMALY_WINDOW` au-delà duquel le client est kické —
/// tolère le jitter/faux positif isolé, sanctionne le speedhack soutenu. Ajustable au playtest.
pub const ANOMALY_KICK_THRESHOLD: u32 = 20;
/// Fenêtre glissante d'accumulation des anomalies (voir `AnomalyTracker`).
pub const ANOMALY_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);

/// Compteur glissant d'anomalies pour UN client. Réinitialisé dès que la dernière anomalie
/// enregistrée sort de `ANOMALY_WINDOW` (fenêtre glissante simple, pas un ring buffer : suffisant
/// pour une escalade, pas une métrique de précision).
#[derive(Debug, Clone)]
pub struct AnomalyTracker {
    count: u32,
    window_start: Option<std::time::Instant>,
}

impl AnomalyTracker {
    pub fn new() -> Self {
        Self {
            count: 0,
            window_start: None,
        }
    }
}

impl Default for AnomalyTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Enregistre une anomalie à `now`. Renvoie `true` si le seuil de kick est atteint.
pub fn record_anomaly(tracker: &mut AnomalyTracker, now: std::time::Instant) -> bool {
    match tracker.window_start {
        Some(start) if now.duration_since(start) <= ANOMALY_WINDOW => {
            tracker.count += 1;
        }
        _ => {
            tracker.window_start = Some(now);
            tracker.count = 1;
        }
    }
    tracker.count >= ANOMALY_KICK_THRESHOLD
}

/// Verdict d'un déplacement pour un client de rang `rank`. Un `GameMaster` est toujours vert
/// (bypass playtest voulu, staff/MJ — Moderator et Player n'en bénéficient jamais). Sinon,
/// `classify_move` tranche (une 1re position, `last` = `None`, est verte). Remplace
/// `resolve_move_plausibility` (retirée au recâblage de la boucle).
pub fn resolve_move_verdict(
    rank: crate::handoff::Rank,
    last: Option<([f32; 3], std::time::Duration)>,
    current: [f32; 3],
) -> crate::anticheat::MoveVerdict {
    use crate::anticheat::MoveVerdict;
    if rank == crate::handoff::Rank::GameMaster {
        return MoveVerdict::Green;
    }
    match last {
        Some((prev, elapsed)) => crate::anticheat::classify_move(prev, current, elapsed),
        None => MoveVerdict::Green,
    }
}

/// Point d'entrée du Gateway (M4, handoff) : ouvre l'écoute GNS publique et, pour chaque client,
/// calcule à chaque position — via la `ShardTopology` locale + le rayon selon le rang — l'ensemble
/// de shards où le charger (autoritaire + zones tampon). Il diffuse les événements du client à tous
/// ses shards chargés, et **fusionne** les snapshots reçus de ces shards en un seul avant de les
/// renvoyer au client. Le double-chargement près d'une frontière élimine les saccades au transfert.
#[cfg(feature = "gns")]
#[allow(clippy::too_many_arguments)] // config de boot (manifeste éclaté en paramètres), pas un point d'appel répété
pub async fn gateway_main(
    listen_addr: &str,
    topology: crate::handoff::ShardTopology,
    radius: crate::handoff::RadiusPolicy,
    mut store: crate::player_store_impl::PlayerStoreImpl,
    mut admin_store: crate::admin_store::AdminStore,
    spawn: [f32; 3],
    max_players: u32,
    jwks_cache: std::sync::Arc<crate::jwks::JwksCache>,
    identity_public: bool,
    whitelist_enabled: bool,
    whitelist_names: std::collections::HashSet<String>,
    hot_state: crate::hot_state_cache::HotStateCache,
) -> std::io::Result<()> {
    use crate::admin_commands::{execute as execute_admin_command, parse as parse_admin_command};
    use crate::gateway_routing::{
        extract_admin_command, extract_join_fields, extract_leave, extract_position,
        extract_position_yaw, extract_time_report,
    };
    use crate::gns_transport::GnsTransport;
    use crate::handoff::{LoadAction, Rank, ShardLoader};
    use crate::permissions::{derive_rank, resolve_permissions};
    use crate::persistence::{PlayerRecord, PlayerStore};
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
    // Persistance : clé EFFECTIVE (display_name sur serveur privé, `sub` OIDC vérifié sur
    // serveur public — Task C2), dernière position, et résidence chargée — par client. Alimente
    // `store.load`/`store.save`/`save_all_known`. Depuis Task D3, cette map sert AUSSI de source
    // du `sub` vérifié pour la résolution d'autorité admin sur serveur public (voir
    // `is_root_by_sub_or_display_name`/`resolve_admin_record`, lus via `keys.get(&cid)` UNIQUEMENT
    // quand `identity_public` est vrai — sur serveur privé cette map porte le display_name, pas un
    // `sub`, et ne doit jamais être lue à cette fin).
    let mut keys: HashMap<u64, String> = HashMap::new();
    // Pseudo affiché (display_name) — toujours le nom brut du Join, jamais le `sub` OIDC vérifié
    // même sur un serveur public. Depuis Task D3, `admin_store`/`root_admins` sont résolus en
    // priorisant le `sub` (via `keys`, cf. ci-dessus) quand disponible, avec repli sur ce
    // display_name sinon (serveur privé, ou compte encore jamais vu sur ce serveur public) — les
    // deux maps restent séparées : `keys` porte la clé de PERSISTANCE (effective_key), qui diffère
    // du display_name sur un serveur public ; ne pas les confondre.
    let mut display_names: HashMap<u64, String> = HashMap::new();
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
    // Anomalies de mouvement (zone orange) par client : escalade vers un kick au-delà de
    // ANOMALY_KICK_THRESHOLD dans ANOMALY_WINDOW (cf. AnomalyTracker/record_anomaly).
    let mut anomaly_trackers: HashMap<u64, AnomalyTracker> = HashMap::new();
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

    // Audience OIDC attendue au Join sur un serveur public : le client_id ZITADEL du client
    // `launcher`. Le launcher envoie son `id_token`, dont le `aud` vaut CE client_id — jamais la
    // chaîne littérale "launcher" (ancien placeholder qui rejetait tout token réel en
    // WrongAudience). Défaut EN DUR sur le client_id de l'instance officielle
    // (`DEFAULT_LAUNCHER_CLIENT_ID`, non secret — cf. sa doc) : aucune config requise pour le
    // déploiement officiel, exactement comme `ZITADEL_JWKS_URL`. `TESSERA_ZITADEL_LAUNCHER_CLIENT_ID`
    // reste un OVERRIDE optionnel (opérateur tiers, ADR 0010), lu par `std::env::var` (comme
    // root_admins/playtest_all_admin) pour ne pas changer la signature de `gateway_main` (rebuild
    // `--features gns` évité, cf. CLAUDE.md). Une valeur vide/blanche retombe sur le défaut.
    let launcher_audience = std::env::var("TESSERA_ZITADEL_LAUNCHER_CLIENT_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_LAUNCHER_CLIENT_ID.to_string());

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
    // Page de logs en direct (HTML + SSE) — spec session-log-live-view, 2026-07-18. Port
    // SÉPARÉ de TESSERA_GATEWAY_SESSIONLOG_ADDR ci-dessus (JSONL brut, non publié) : celui-ci
    // est fait pour être PUBLIÉ (voir docker-compose.yml) et consulté depuis un navigateur
    // pendant un playtest, sans SSH. ⚠️ Aucune authentification (décision explicite, spec
    // §Sécurité) : ne jamais élargir son usage au-delà d'un playtest restreint et connu sans
    // revisiter ce choix.
    {
        let addr = std::env::var("TESSERA_GATEWAY_SESSIONLOG_HTML_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9103".to_string());
        let path = std::path::PathBuf::from(session_log_path.clone());
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                // `addr` est l'adresse de BIND (souvent 0.0.0.0 en Docker — toutes les interfaces
                // du conteneur), pas l'adresse à laquelle un navigateur peut se connecter. Le
                // message de boot doit afficher un lien réellement joignable : si
                // TESSERA_GATEWAY_SESSIONLOG_HTML_PUBLIC_HOST est fournie, on l'utilise avec le
                // même port que `addr` ; sinon on retombe sur `addr` tel quel (utile en dev local
                // hors Docker, où le défaut 127.0.0.1 est déjà correct). Piège vécu (2026-07-18) :
                // sans ça, le boot annonçait "http://0.0.0.0:9103/", techniquement exact comme
                // adresse de bind mais inutilisable tel quel dans un navigateur.
                let display_addr = match std::env::var("TESSERA_GATEWAY_SESSIONLOG_HTML_PUBLIC_HOST")
                {
                    Ok(host) if !host.trim().is_empty() => {
                        let port = addr.rsplit(':').next().unwrap_or("9103");
                        format!("{host}:{port}")
                    }
                    _ => addr.clone(),
                };
                tracing::info!("logs de session en direct disponibles sur http://{display_addr}/");
                tokio::spawn(async move {
                    if let Err(e) = crate::session_log_html::serve_live(listener, path).await {
                        tracing::warn!("page de logs en direct indisponible ({addr}): {e}");
                    }
                });
            }
            Err(e) => {
                tracing::warn!("page de logs en direct indisponible (bind {addr} échoué): {e}");
            }
        }
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
                let decision = check_rate_limit(
                    state,
                    now,
                    DEFAULT_LIMIT_PER_WINDOW,
                    DEFAULT_KICK_AFTER_WINDOWS,
                );
                let was_kick = decision == RateDecision::Kick;
                let consumed = apply_rate_limit_decision(decision, cid, &metrics, &mut client);
                if was_kick {
                    // Nettoyage complet de l'état per-cid — même liste de maps que le nettoyage
                    // partagé Disconnected/Leave (cf. `cleanup_client_state`), y compris la
                    // sauvegarde de la dernière position connue.
                    cleanup_client_state(
                        cid,
                        &mut store,
                        &mut keys,
                        &mut display_names,
                        &mut last_pos,
                        &mut last_pos_at,
                        &mut bypass_warned_at,
                        &mut anomaly_trackers,
                        &mut ranks,
                        &mut permissions,
                        &mut residence,
                        &mut rate_states,
                        &mut loader,
                        &mut latest,
                        &mut prev_placements,
                    );
                }
                if consumed {
                    continue;
                }
            }

            // Décoder ce que porte un message client : Join → identité + résolution de spawn ;
            // PositionUpdate → placement (topologie + rang) et mémorisation de la dernière position.
            let mut placement = None;
            if let TransportEvent::Message { data, .. } = &ev {
                if let Some((name, token, protocol_version)) = extract_join_fields(data) {
                    if !name.is_empty() || !token.is_empty() {
                        if let Err(reason) = resolve_protocol_version(protocol_version) {
                            tracing::warn!(
                                client = cid,
                                received = protocol_version,
                                expected = crate::gateway_routing::CURRENT_PROTOCOL_VERSION,
                                "kick : version protocole incompatible"
                            );
                            client.send(cid, &encode_kicked(reason));
                            client.disconnect(cid);
                            rate_states.remove(&cid);
                            continue;
                        }
                        if let Err(reason) =
                            resolve_whitelist(whitelist_enabled, &whitelist_names, &name)
                        {
                            tracing::warn!(client = cid, %name, "kick : non présent sur la whitelist");
                            client.send(cid, &encode_kicked(reason));
                            client.disconnect(cid);
                            rate_states.remove(&cid);
                            continue;
                        }
                        // NB: on renomme le champ `display` en `disp_name` — une variable locale
                        // nommée `display` entre en COLLISION avec `tracing::field::display` dans les
                        // macros `tracing::info!` (`%display` / `{display}` résolvent alors vers la
                        // fonction, pas la variable → E0277 "doesn't implement Display"). Invisible
                        // sans `--features gns` (gateway_main est gns-gated, non compilé par la CI) —
                        // a cassé le build Docker du gateway (merge display-name-from-jwt, PR #9).
                        let JoinIdentity {
                            key: effective_key,
                            display: disp_name,
                        } = match resolve_join_key(
                            identity_public,
                            &name,
                            &token,
                            &launcher_audience,
                            &jwks_cache,
                        ) {
                            Ok(identity) => identity,
                            Err(reason) => {
                                tracing::warn!(client = cid, %reason, "kick : Join refusé");
                                client.send(cid, &encode_kicked(reason));
                                client.disconnect(cid);
                                rate_states.remove(&cid);
                                continue;
                            }
                        };
                        if reject_join_if_server_full(
                            keys.contains_key(&cid),
                            keys.len(),
                            max_players,
                            cid,
                            &metrics,
                            &mut client,
                        ) {
                            rate_states.remove(&cid);
                            continue;
                        }
                        store.note_display_name(&effective_key, &disp_name);
                        let record = store.load(&effective_key);
                        let (pos, source) =
                            resolve_join_spawn(&effective_key, &hot_state, record.as_ref(), spawn)
                                .await;
                        tracing::info!(
                            "Connexion de {disp_name} : placement décidé {pos:?} (source: {source:?})"
                        );
                        residence.insert(cid, record.and_then(|r| r.residence));
                        last_pos.insert(cid, pos); // départ tant qu'aucune position réelle

                        // Résolution d'autorité admin (Task D3) : sur un serveur public,
                        // `effective_key` EST le `sub` OIDC vérifié (cf. `resolve_join_key`) — on
                        // le priorise pour retrouver l'admin, avec repli sur `name` (display_name
                        // brut) sinon. Sur un serveur privé, `sub_for_admin` est `None` et le
                        // comportement est strictement celui d'avant cette tâche.
                        let sub_for_admin: Option<&str> =
                            identity_public.then_some(effective_key.as_str());
                        let is_root = is_root_by_sub_or_display_name(
                            sub_for_admin,
                            &disp_name,
                            &root_admins,
                            playtest_all_admin,
                        );
                        let admin_record =
                            resolve_admin_record(sub_for_admin, &disp_name, &admin_store.admins)
                                .cloned();
                        // Backfill (Task D3) : un admin attribué par `/promote` avant son premier
                        // Join sur un serveur public (ou avant cette migration) a un `AdminRecord`
                        // encore résolu par repli display_name (`sub: None`). Dès qu'on connaît
                        // son vrai `sub` vérifié, on l'enregistre pour que les Join suivants (et
                        // toute future collision de pseudo) le résolvent directement par `sub` —
                        // migration progressive sans perdre l'admin en cours de route. Sûr même en
                        // cas de collision de display_name : `/promote` (admin_commands.rs) fait
                        // un `find` (jamais `find_all`) avant d'insérer, donc `admin_store.admins`
                        // ne peut jamais contenir deux enregistrements avec le même display_name —
                        // ce `find` ne peut matcher qu'un seul enregistrement au plus.
                        if let (Some(sub), Some(record)) = (sub_for_admin, admin_record.as_ref()) {
                            if record.sub.is_none() {
                                if let Some(stored) = admin_store
                                    .admins
                                    .iter_mut()
                                    .find(|a| a.display_name == record.display_name)
                                {
                                    stored.sub = Some(sub.to_string());
                                    admin_store.save_admins();
                                }
                            }
                        }
                        let resolved = resolve_permissions(
                            is_root,
                            admin_record.as_ref(),
                            &admin_store.groups,
                        );
                        let rank = derive_rank(&resolved);
                        if rank != Rank::Player {
                            tracing::info!(client = cid, %disp_name, ?rank, "rang attribué");
                            ranks.insert(cid, rank);
                        }
                        if !resolved.is_empty() {
                            client.send(cid, &encode_permission_sync(&resolved));
                        }
                        permissions.insert(cid, resolved);
                        if let Some(sl) = slog.as_mut() {
                            sl.write(&crate::session_log::SessionEvent::Join {
                                client: cid,
                                name: display.clone(),
                            });
                        }
                        keys.insert(cid, effective_key);
                        display_names.insert(cid, display);
                    }
                } else if let Some((x, y, z)) = extract_position(data) {
                    let now = std::time::Instant::now();
                    let rank = ranks.get(&cid).copied().unwrap_or(Rank::Player);
                    let bypassed = rank == Rank::GameMaster;
                    if bypassed {
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
                    }
                    let last = match (last_pos.get(&cid).copied(), last_pos_at.get(&cid).copied()) {
                        (Some(prev), Some(at)) => Some((prev, now.duration_since(at))),
                        _ => None,
                    };
                    match resolve_move_verdict(rank, last, [x, y, z]) {
                        crate::anticheat::MoveVerdict::Green => {
                            last_pos.insert(cid, [x, y, z]);
                            last_pos_at.insert(cid, now);
                        }
                        crate::anticheat::MoveVerdict::Orange => {
                            // On SUIT le client (RP-safe : jamais de rubber-band sur un faux
                            // positif), on avance last_pos (le joueur bouge aux yeux des autres,
                            // la sauvegarde reste fraîche — corrige P3), et on compte l'anomalie.
                            last_pos.insert(cid, [x, y, z]);
                            last_pos_at.insert(cid, now);
                            metrics
                                .rejected_messages_total
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let tracker = anomaly_trackers.entry(cid).or_default();
                            if record_anomaly(tracker, now) {
                                tracing::warn!(
                                    client = cid,
                                    "kick : anomalies de mouvement répétées (speedhack probable)"
                                );
                                client.send(cid, &encode_kicked("mouvement incohérent répété"));
                                client.disconnect(cid);
                                anomaly_trackers.remove(&cid);
                                continue;
                            }
                            tracing::warn!(
                                client = cid,
                                "PositionUpdate anomalie modérée (acceptée, comptée)"
                            );
                        }
                        crate::anticheat::MoveVerdict::Red => {
                            // Téléport franc : on NE met PAS à jour last_pos (la dernière position
                            // valide est celle qu'on réimpose) et on renvoie une correction au client.
                            metrics
                                .rejected_messages_total
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let anchor = last_pos.get(&cid).copied().unwrap_or([x, y, z]);
                            let yaw = extract_position_yaw(data).unwrap_or(0.0);
                            client.send(
                                cid,
                                &encode_position_correction(anchor, yaw, 1 /* AntiCheat */),
                            );
                            tracing::warn!(
                                client = cid,
                                "PositionUpdate rejeté (téléport) → correction envoyée"
                            );
                            continue;
                        }
                    }
                    // Écriture hot-state (Décision 3, design stockage 2026-07-09) : à chaque
                    // PositionUpdate ACCEPTÉ, taguée par la clé effective (jamais display_name).
                    // Un client sans clé effective connue (ne devrait pas arriver après un Join
                    // réussi) est silencieusement ignoré plutôt que de paniquer.
                    if let Some(effective_key) = keys.get(&cid) {
                        if let Err(e) = hot_state.write(effective_key, [x, y, z]).await {
                            tracing::warn!(
                                client = cid,
                                "HotStateCache::write échoué (subject={effective_key}): {e:?}"
                            );
                        }
                    }
                    let r = radius.radius_for(*ranks.get(&cid).unwrap_or(&Rank::Player));
                    placement = Some(topology.locate(x, y, r));
                    if let Some(next) = placement.as_ref() {
                        let changes =
                            crate::session_log::diff_placement(prev_placements.get(&cid), next);
                        // Poussé au client dès qu'un changement de placement est détecté (y compris
                        // le tout premier après Join, où prev_placements.get(&cid) == None) : le HUD
                        // compare ce placement autoritaire à son calcul local et signale un décalage
                        // persistant (spec HUD moniteur de cohérence, 2026-07-18).
                        if !changes.is_empty() {
                            client.send(
                                cid,
                                &encode_shard_assignment(&next.authoritative, &next.overlaps),
                            );
                        }
                        if let Some(sl) = slog.as_mut() {
                            for c in changes {
                                use crate::session_log::{PlacementChange, SessionEvent};
                                let ev = match c {
                                    PlacementChange::Handoff { from, to } => {
                                        SessionEvent::Handoff {
                                            client: cid,
                                            from,
                                            to,
                                            x,
                                            y,
                                            z,
                                        }
                                    }
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
                                let name =
                                    display_names.get(&cid).map(String::as_str).unwrap_or("?");
                                match &ev {
                                    crate::session_log::SessionEvent::Handoff {
                                        from, to, ..
                                    } => {
                                        tracing::info!(
                                            client = cid,
                                            %name,
                                            "Handoff : {name} passe de {from} à {to} ({x:.1}, {y:.1}, {z:.1})"
                                        );
                                    }
                                    crate::session_log::SessionEvent::BufferEnter {
                                        shard, ..
                                    } => {
                                        tracing::info!(client = cid, %name, "{name} entre en zone tampon de {shard}");
                                    }
                                    crate::session_log::SessionEvent::BufferExit {
                                        shard, ..
                                    } => {
                                        tracing::info!(client = cid, %name, "{name} sort de la zone tampon de {shard}");
                                    }
                                    _ => {}
                                }
                                sl.write(&ev);
                            }
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
                        let name = display_names.get(&cid).map(String::as_str).unwrap_or("?");
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
                    // Résolution d'identité admin (Task D3) : `issuer` (display_name brut, depuis
                    // `display_names`) reste le texte affiché/journalisé (`granted_by`, logs,
                    // `SessionEvent::AdminAction`) — jamais le `sub`, illisible pour un humain.
                    // L'AUTORITÉ (`is_root`), elle, se résout en priorisant le `sub` OIDC vérifié
                    // de CE client quand il est connu : `keys` porte le `sub` uniquement quand
                    // `identity_public` est vrai (cf. `resolve_join_key`, Task C2) — sur serveur
                    // privé `sub_for_admin` est `None` et le comportement est inchangé.
                    let issuer = display_names.get(&cid).cloned().unwrap_or_default();
                    let sub_for_admin: Option<&str> = if identity_public {
                        keys.get(&cid).map(String::as_str)
                    } else {
                        None
                    };
                    let is_root = is_root_by_sub_or_display_name(
                        sub_for_admin,
                        &issuer,
                        &root_admins,
                        playtest_all_admin,
                    );
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
                        if let Some((&target_cid, _)) =
                            display_names.iter().find(|(_, n)| *n == target)
                        {
                            // Même priorité sub-avec-repli que la résolution de l'issuer
                            // ci-dessus (Task D3) : le compte cible peut être connecté avec un
                            // `sub` OIDC vérifié différent de son display_name.
                            let target_sub: Option<&str> = if identity_public {
                                keys.get(&target_cid).map(String::as_str)
                            } else {
                                None
                            };
                            let is_target_root = is_root_by_sub_or_display_name(
                                target_sub,
                                target,
                                &root_admins,
                                playtest_all_admin,
                            );
                            let target_record =
                                resolve_admin_record(target_sub, target, &admin_store.admins)
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
                } else if extract_leave(data).is_some() {
                    // Départ volontaire (Task C3), distinct d'une coupure réseau/crash
                    // (`TransportEvent::Disconnected`, jamais annoncé par le client) : le même
                    // nettoyage per-cid que ce dernier (voir plus bas, bloc `is_disconnect`), mais
                    // appliqué immédiatement plutôt qu'attendu via un timeout de transport. La
                    // réservation de slot différenciée entre les deux chemins reste palier 2 —
                    // cette tâche pose seulement la distinction protocole.
                    tracing::info!(client = cid, "départ volontaire (Leave)");
                    cleanup_client_state(
                        cid,
                        &mut store,
                        &mut keys,
                        &mut display_names,
                        &mut last_pos,
                        &mut last_pos_at,
                        &mut bypass_warned_at,
                        &mut anomaly_trackers,
                        &mut ranks,
                        &mut permissions,
                        &mut residence,
                        &mut rate_states,
                        &mut loader,
                        &mut latest,
                        &mut prev_placements,
                    );
                    continue;
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
                display_names.remove(&cid);
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
    fn position_correction_roundtrips_position_yaw_reason() {
        let bytes = encode_position_correction([1.0, 2.0, 3.0], 90.0, 1);
        let env = flatbuffers::root::<protocol::ServerEnvelope>(&bytes).unwrap();
        assert_eq!(env.msg_type(), ServerMsg::PositionCorrection);
        let pc = env.msg_as_position_correction().unwrap();
        assert_eq!(pc.position().unwrap().x(), 1.0);
        assert_eq!(pc.position().unwrap().y(), 2.0);
        assert_eq!(pc.position().unwrap().z(), 3.0);
        assert_eq!(pc.yaw(), 90.0);
        assert_eq!(pc.reason(), 1);
    }

    #[test]
    fn shard_assignment_roundtrips_authoritative_and_overlaps() {
        let bytes =
            encode_shard_assignment("group-1", &["group-0".to_string(), "group-2".to_string()]);
        let env = flatbuffers::root::<protocol::ServerEnvelope>(&bytes).unwrap();
        assert_eq!(env.msg_type(), ServerMsg::ShardAssignment);
        let sa = env.msg_as_shard_assignment().unwrap();
        assert_eq!(sa.authoritative(), Some("group-1"));
        let overlaps: Vec<&str> = sa.overlaps().unwrap().iter().collect();
        assert_eq!(overlaps, vec!["group-0", "group-2"]);
    }

    #[test]
    fn shard_assignment_roundtrips_empty_overlaps() {
        let bytes = encode_shard_assignment("group-0", &[]);
        let env = flatbuffers::root::<protocol::ServerEnvelope>(&bytes).unwrap();
        let sa = env.msg_as_shard_assignment().unwrap();
        assert_eq!(sa.authoritative(), Some("group-0"));
        assert_eq!(sa.overlaps().unwrap().len(), 0);
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

    #[test]
    fn apply_rate_limit_decision_on_sustained_flood_kicks_and_counts_rejected_metric() {
        use crate::rate_limit::{
            check_rate_limit, RateLimitState, DEFAULT_KICK_AFTER_WINDOWS, DEFAULT_LIMIT_PER_WINDOW,
        };
        use std::time::{Duration, Instant};

        let t0 = Instant::now();
        let mut state = RateLimitState::new(t0);
        let mut client = InMemoryTransport::new();
        let metrics = crate::metrics::Metrics::new();
        let cid = 7u64;

        // Reproduit sustained_flooding_across_consecutive_windows_kicks (rate_limit.rs) : 3
        // fenêtres consécutives bien au-dessus de la limite → RateDecision::Kick à la dernière.
        // Comme dans gateway_main, un `Kick` fait sortir de la boucle de messages (`continue`
        // puis déconnexion) : on s'arrête donc au premier Kick rencontré.
        let mut last_decision = None;
        'windows: for window in 0..3 {
            let t = t0 + Duration::from_secs(window);
            for _ in 0..50 {
                let decision = check_rate_limit(
                    &mut state,
                    t,
                    DEFAULT_LIMIT_PER_WINDOW,
                    DEFAULT_KICK_AFTER_WINDOWS,
                );
                let is_kick = decision == crate::rate_limit::RateDecision::Kick;
                last_decision = Some(apply_rate_limit_decision(
                    decision,
                    cid,
                    &metrics,
                    &mut client,
                ));
                if is_kick {
                    break 'windows;
                }
            }
        }

        assert_eq!(
            last_decision,
            Some(true),
            "la dernière fenêtre doit être un Kick consommant le message"
        );
        assert_eq!(
            metrics
                .rejected_messages_total
                .load(std::sync::atomic::Ordering::Relaxed),
            1,
            "un seul rejet (le Kick final), pas un par message"
        );
        assert_eq!(
            client.take_disconnected(),
            vec![cid],
            "le client doit avoir été déconnecté"
        );
        let sent = client.take_sent(cid);
        assert_eq!(sent.len(), 1, "un seul Kicked envoyé");
        let env = flatbuffers::root::<protocol::ServerEnvelope>(&sent[0]).unwrap();
        assert_eq!(env.msg_type(), ServerMsg::Kicked);
        assert_eq!(env.msg_as_kicked().unwrap().reason(), Some("flood"));
    }

    #[test]
    fn apply_rate_limit_decision_on_accept_does_nothing() {
        use crate::rate_limit::RateDecision;

        let mut client = InMemoryTransport::new();
        let metrics = crate::metrics::Metrics::new();

        let consumed = apply_rate_limit_decision(RateDecision::Accept, 1, &metrics, &mut client);

        assert!(!consumed);
        assert_eq!(
            metrics
                .rejected_messages_total
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert!(client.take_sent(1).is_empty());
        assert!(client.take_disconnected().is_empty());
    }

    #[test]
    fn apply_rate_limit_decision_on_drop_ignores_without_metric_or_kick() {
        use crate::rate_limit::RateDecision;

        let mut client = InMemoryTransport::new();
        let metrics = crate::metrics::Metrics::new();

        let consumed = apply_rate_limit_decision(RateDecision::Drop, 1, &metrics, &mut client);

        assert!(
            consumed,
            "un message Drop est ignoré : le message est consommé"
        );
        assert_eq!(
            metrics
                .rejected_messages_total
                .load(std::sync::atomic::Ordering::Relaxed),
            0,
            "Drop n'incrémente pas la métrique aujourd'hui (comportement inchangé)"
        );
        assert!(client.take_sent(1).is_empty());
        assert!(client.take_disconnected().is_empty());
    }

    #[test]
    fn reject_join_if_server_full_kicks_new_client_when_at_capacity() {
        let mut client = InMemoryTransport::new();
        let metrics = crate::metrics::Metrics::new();

        let rejected = reject_join_if_server_full(false, 10, 10, 42, &metrics, &mut client);

        assert!(rejected);
        assert_eq!(
            metrics
                .rejected_messages_total
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(client.take_disconnected(), vec![42]);
        let sent = client.take_sent(42);
        assert_eq!(sent.len(), 1);
        let env = flatbuffers::root::<protocol::ServerEnvelope>(&sent[0]).unwrap();
        assert_eq!(env.msg_type(), ServerMsg::Kicked);
        assert_eq!(env.msg_as_kicked().unwrap().reason(), Some("serveur plein"));
    }

    #[test]
    fn reject_join_if_server_full_allows_room_below_capacity() {
        let mut client = InMemoryTransport::new();
        let metrics = crate::metrics::Metrics::new();

        let rejected = reject_join_if_server_full(false, 5, 10, 42, &metrics, &mut client);

        assert!(!rejected);
        assert_eq!(
            metrics
                .rejected_messages_total
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert!(client.take_sent(42).is_empty());
        assert!(client.take_disconnected().is_empty());
    }

    #[test]
    fn reject_join_if_server_full_allows_reconnecting_known_client_even_at_capacity() {
        // Un client déjà dans `keys` (re-Join) ne doit pas être rejeté même si keys.len() ==
        // max_players : `keys_contains_cid = true` désactive le rejet.
        let mut client = InMemoryTransport::new();
        let metrics = crate::metrics::Metrics::new();

        let rejected = reject_join_if_server_full(true, 10, 10, 42, &metrics, &mut client);

        assert!(!rejected);
        assert_eq!(
            metrics
                .rejected_messages_total
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert!(client.take_disconnected().is_empty());
    }

    fn join_payload() -> Vec<u8> {
        let mut b = flatbuffers::FlatBufferBuilder::new();
        let name = b.create_string("v");
        let join = protocol::Join::create(
            &mut b,
            &protocol::JoinArgs {
                display_name: Some(name),
                token: None,
                protocol_version: 1,
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
            sub: None,
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

    // --- is_root_by_sub_or_display_name / resolve_admin_record (Task D3) -------------------
    //
    // Migration de l'indexation admin vers `sub` OIDC vérifié, avec repli sur `display_name`
    // pour les serveurs privés (voir note de contexte du brief D3). Ferme le bug playtest 1 :
    // deux comptes distincts (`sub` différents) avec le même `display_name` ne doivent jamais
    // partager d'autorité admin sur un serveur public.

    #[test]
    fn is_root_by_sub_grants_when_sub_is_listed_even_if_display_name_is_not() {
        let root_admins: std::collections::HashSet<String> =
            ["oidc-sub-alice".to_string()].into_iter().collect();
        assert!(is_root_by_sub_or_display_name(
            Some("oidc-sub-alice"),
            "Lucas",
            &root_admins,
            false
        ));
    }

    #[test]
    fn is_root_by_sub_falls_back_to_display_name_when_sub_absent() {
        // Serveur privé : `sub` est `None`, repli sur `display_name` — comportement historique
        // inchangé.
        let root_admins: std::collections::HashSet<String> =
            ["Lucas".to_string()].into_iter().collect();
        assert!(is_root_by_sub_or_display_name(
            None,
            "Lucas",
            &root_admins,
            false
        ));
    }

    #[test]
    fn is_root_by_sub_falls_back_to_display_name_when_sub_not_listed() {
        // Serveur public mais `TESSERA_ROOT_ADMINS` liste encore un display_name (transition) :
        // le repli doit fonctionner tant que le sub lui-même n'est pas listé.
        let root_admins: std::collections::HashSet<String> =
            ["Lucas".to_string()].into_iter().collect();
        assert!(is_root_by_sub_or_display_name(
            Some("oidc-sub-alice"),
            "Lucas",
            &root_admins,
            false
        ));
    }

    #[test]
    fn is_root_by_sub_denies_when_neither_sub_nor_display_name_listed() {
        let root_admins: std::collections::HashSet<String> =
            ["Compte1".to_string()].into_iter().collect();
        assert!(!is_root_by_sub_or_display_name(
            Some("oidc-sub-mallory"),
            "Mallory",
            &root_admins,
            false
        ));
    }

    #[test]
    fn is_root_by_sub_respects_playtest_bypass() {
        let root_admins: std::collections::HashSet<String> = std::collections::HashSet::new();
        assert!(is_root_by_sub_or_display_name(
            Some("anyone"),
            "Anyone",
            &root_admins,
            true
        ));
    }

    fn admin_record_with_sub(
        display_name: &str,
        sub: Option<&str>,
    ) -> crate::permissions::AdminRecord {
        crate::permissions::AdminRecord {
            display_name: display_name.to_string(),
            sub: sub.map(str::to_string),
            group: "moderator".to_string(),
            extra_permissions: vec![],
            revoked_permissions: vec![],
            granted_at: 0,
            granted_by: "Root".to_string(),
        }
    }

    #[test]
    fn resolve_admin_record_finds_by_sub_first() {
        let admins = vec![
            admin_record_with_sub("Lucas", Some("oidc-sub-alice")),
            admin_record_with_sub("Lucas", Some("oidc-sub-bob")),
        ];
        // Deux comptes distincts partagent le même display_name (collision de pseudo, exactement
        // le scénario du playtest 1) : la résolution par `sub` doit retrouver le bon enregistrement
        // sans jamais confondre les deux comptes.
        let found = resolve_admin_record(Some("oidc-sub-bob"), "Lucas", &admins)
            .expect("le compte bob doit être trouvé par son sub");
        assert_eq!(found.sub.as_deref(), Some("oidc-sub-bob"));
    }

    #[test]
    fn resolve_admin_record_falls_back_to_display_name_when_sub_absent() {
        let admins = vec![admin_record_with_sub("Lucas", None)];
        let found = resolve_admin_record(None, "Lucas", &admins)
            .expect("repli display_name doit trouver l'enregistrement");
        assert_eq!(found.display_name, "Lucas");
    }

    #[test]
    fn resolve_admin_record_falls_back_to_display_name_when_sub_not_found() {
        // Compte promu par `/promote` avant son premier Join sur un serveur public : son
        // AdminRecord existe déjà (créé par admin_commands.rs) mais avec `sub: None` — tant
        // qu'aucun Join n'a enrichi l'enregistrement, la résolution par sub échoue et doit
        // replier sur le display_name plutôt que de perdre l'admin.
        let admins = vec![admin_record_with_sub("Lucas", None)];
        let found = resolve_admin_record(Some("oidc-sub-alice"), "Lucas", &admins)
            .expect("repli display_name doit trouver l'enregistrement même avec un sub inconnu");
        assert_eq!(found.display_name, "Lucas");
    }

    #[test]
    fn resolve_admin_record_returns_none_when_nothing_matches() {
        let admins = vec![admin_record_with_sub("Lucas", Some("oidc-sub-alice"))];
        assert!(resolve_admin_record(Some("oidc-sub-mallory"), "Mallory", &admins).is_none());
    }

    #[test]
    fn resolve_admin_record_never_falls_back_to_a_record_already_bound_to_another_sub() {
        // Garde anti-collision (root cause du bug playtest 1) : Alice est admin, son AdminRecord
        // a déjà un sub connu. Bob revendique le même display_name mais a un sub DIFFÉRENT — le
        // repli display_name ne doit jamais lui rendre l'enregistrement d'Alice, sinon Bob
        // hériterait silencieusement de son groupe/permissions.
        let admins = vec![admin_record_with_sub("Lucas", Some("oidc-sub-alice"))];
        assert!(
            resolve_admin_record(Some("oidc-sub-bob"), "Lucas", &admins).is_none(),
            "bob ne doit jamais résoudre l'AdminRecord d'alice via leur display_name partagé"
        );
    }

    // --- Test central de non-régression (brief D3, Step 1) : bug playtest 1 fermé --------------
    //
    // Deux comptes ZITADEL distincts (`sub` différents) avec le même `display_name` ne doivent
    // JAMAIS partager d'état ni d'autorité admin sur un serveur public — root cause exacte du bug
    // observé en playtest 1. `PostgresStore` (Task D1) n'étant pas câblé dans `gateway_main` (hors
    // scope de cette tâche, voir note de contexte du brief point 4), ce test exerce le pipeline
    // réellement câblé aujourd'hui : `resolve_join_key` (clé de persistance = `sub`, Task C2) +
    // `is_root_by_sub_or_display_name`/`resolve_admin_record` (autorité admin, Task D3) — la
    // même garantie de non-partage est déjà prouvée séparément pour `StoreError::DisplayNameConflict`
    // au niveau `postgres_store.rs` (`display_name_conflict_is_first_come_first_served`).
    #[tokio::test]
    async fn two_accounts_with_same_display_name_never_share_admin_state_on_public_server() {
        let (jwks_cache, material) = join_test_jwks_cache_with_valid_key().await;
        let display_name = "Lucas"; // collision de pseudo, exactement le scénario du playtest 1

        let claims_alice = crate::jwks::Claims {
            sub: "oidc-sub-alice".into(),
            aud: "launcher".into(),
            name: None,
            preferred_username: None,
            exp: far_future_timestamp(),
        };
        let token_alice = encode_join_test_token(&claims_alice, &material.encoding_key);
        let claims_bob = crate::jwks::Claims {
            sub: "oidc-sub-bob".into(),
            aud: "launcher".into(),
            name: None,
            preferred_username: None,
            exp: far_future_timestamp(),
        };
        let token_bob = encode_join_test_token(&claims_bob, &material.encoding_key);

        // Les deux Join sont traités (pas de kick sur la collision de display_name).
        let key_alice = resolve_join_key(true, display_name, &token_alice, "launcher", &jwks_cache)
            .expect("alice : join accepté malgré la collision de display_name")
            .key;
        let key_bob = resolve_join_key(true, display_name, &token_bob, "launcher", &jwks_cache)
            .expect("bob : join accepté malgré la collision de display_name")
            .key;

        // Clés de persistance distinctes : deux enregistrements Postgres/FileStore distincts.
        assert_ne!(
            key_alice, key_bob,
            "deux sub distincts doivent produire deux clés de persistance distinctes"
        );
        assert_eq!(key_alice, "oidc-sub-alice");
        assert_eq!(key_bob, "oidc-sub-bob");

        // Alice seule est admin (un seul AdminRecord, rattaché à son sub) : Bob, malgré le même
        // display_name, ne doit jamais hériter de son autorité.
        let admins = vec![admin_record_with_sub(display_name, Some("oidc-sub-alice"))];
        let root_admins: std::collections::HashSet<String> =
            ["oidc-sub-alice".to_string()].into_iter().collect();

        let alice_record = resolve_admin_record(Some(&key_alice), display_name, &admins)
            .expect("alice doit résoudre son propre AdminRecord");
        assert_eq!(alice_record.sub.as_deref(), Some("oidc-sub-alice"));
        let alice_is_root =
            is_root_by_sub_or_display_name(Some(&key_alice), display_name, &root_admins, false);
        assert!(
            alice_is_root,
            "alice doit être reconnue root admin par son sub"
        );

        // Bob revendique le même display_name qu'Alice mais a un sub DIFFÉRENT : la garde
        // anti-collision de `resolve_admin_record` (root cause du bug playtest 1) doit refuser de
        // lui rendre l'AdminRecord d'Alice — sans quoi Bob hériterait silencieusement de son
        // groupe/permissions via leur seul point commun, le display_name affiché.
        let bob_record = resolve_admin_record(Some(&key_bob), display_name, &admins);
        assert!(
            bob_record.is_none(),
            "bob ne doit jamais résoudre l'AdminRecord d'alice via leur display_name partagé"
        );
        let bob_is_root =
            is_root_by_sub_or_display_name(Some(&key_bob), display_name, &root_admins, false);
        assert!(
            !bob_is_root,
            "bob ne doit jamais hériter de l'autorité root d'alice via leur display_name partagé"
        );
    }

    // --- Test central de non-régression : visibilité AoI symétrique (bug playtest 1) -----------
    //
    // Le bug original observé en playtest 1 : deux joueurs connectés avec le même `display_name`
    // partageaient silencieusement le même enregistrement de persistance, produisant une
    // visibilité AoI ASYMÉTRIQUE (un joueur voyait l'autre, pas l'inverse). Ce test exerce les
    // DEUX étages réellement câblés en prod bout en bout :
    //   1. le Gateway (`resolve_join_key`, Task C2) : deux `sub` distincts avec le même
    //      `display_name` doivent produire deux clés de persistance (`keys`) distinctes, jamais
    //      partagées — exactement la racine du bug ;
    //   2. le Shard (`crate::server_loop::Server` + `crate::world::World::snapshot_for`, code de
    //      production réel, pas un mock) : une fois les deux connexions distinctes établies
    //      (`ClientId` assignés par le transport, jamais dérivés de `display_name`/`sub`), chacune
    //      envoie une `PositionUpdate` et le snapshot renvoyé par `Server::tick` à CHAQUE client
    //      doit contenir l'autre — symétriquement. Une régression qui referait fuiter
    //      `display_name` jusque dans l'identité de connexion (ex: un `ClientId` dérivé du nom
    //      plutôt que de la connexion transport) casserait cette symétrie et ce test le détecterait.
    #[tokio::test]
    async fn two_accounts_with_same_display_name_have_symmetric_aoi_visibility() {
        use crate::server_loop::Server;
        use crate::transport::InMemoryTransport;

        let (jwks_cache, material) = join_test_jwks_cache_with_valid_key().await;
        let display_name = "Lucas"; // collision de pseudo, exactement le scénario du playtest 1

        let claims_alice = crate::jwks::Claims {
            sub: "oidc-sub-alice".into(),
            aud: "launcher".into(),
            name: None,
            preferred_username: None,
            exp: far_future_timestamp(),
        };
        let token_alice = encode_join_test_token(&claims_alice, &material.encoding_key);
        let claims_bob = crate::jwks::Claims {
            sub: "oidc-sub-bob".into(),
            aud: "launcher".into(),
            name: None,
            preferred_username: None,
            exp: far_future_timestamp(),
        };
        let token_bob = encode_join_test_token(&claims_bob, &material.encoding_key);

        // --- Étage Gateway : les deux Join sont acceptés, avec des clés de persistance distinctes.
        let key_alice = resolve_join_key(true, display_name, &token_alice, "launcher", &jwks_cache)
            .expect("alice : join accepté malgré la collision de display_name")
            .key;
        let key_bob = resolve_join_key(true, display_name, &token_bob, "launcher", &jwks_cache)
            .expect("bob : join accepté malgré la collision de display_name")
            .key;
        assert_ne!(
            key_alice, key_bob,
            "deux sub distincts doivent produire deux clés de persistance distinctes"
        );

        // --- Étage Shard : deux connexions distinctes (ClientId 1 = alice, ClientId 2 = bob),
        // chacune bouge, puis on vérifie la symétrie du snapshot AoI.
        let alice_cid: u64 = 1;
        let bob_cid: u64 = 2;
        let mut server = Server::new(1000.0); // rayon large : la distance n'est pas ce qu'on teste
        let mut t = InMemoryTransport::new();

        t.inject(TransportEvent::Connected(alice_cid));
        t.inject(TransportEvent::Connected(bob_cid));
        t.inject(TransportEvent::Message {
            from: alice_cid,
            data: crate::gateway_routing::encode_position_update([10.0, 0.0, 0.0]),
        });
        t.inject(TransportEvent::Message {
            from: bob_cid,
            data: crate::gateway_routing::encode_position_update([20.0, 0.0, 0.0]),
        });

        server.tick(&mut t);

        // Alice doit voir Bob.
        let sent_to_alice = t.take_sent(alice_cid);
        assert_eq!(sent_to_alice.len(), 1, "un snapshot envoyé à alice");
        let env_alice = flatbuffers::root::<protocol::ServerEnvelope>(&sent_to_alice[0]).unwrap();
        let snap_alice = env_alice.msg_as_snapshot().unwrap();
        let players_alice = snap_alice.players().unwrap();
        assert_eq!(
            players_alice.len(),
            1,
            "alice doit voir exactement un autre joueur (bob)"
        );
        assert_eq!(players_alice.get(0).id(), bob_cid);

        // Bob doit voir Alice — symétriquement, malgré le display_name partagé.
        let sent_to_bob = t.take_sent(bob_cid);
        assert_eq!(sent_to_bob.len(), 1, "un snapshot envoyé à bob");
        let env_bob = flatbuffers::root::<protocol::ServerEnvelope>(&sent_to_bob[0]).unwrap();
        let snap_bob = env_bob.msg_as_snapshot().unwrap();
        let players_bob = snap_bob.players().unwrap();
        assert_eq!(
            players_bob.len(),
            1,
            "bob doit voir exactement un autre joueur (alice) — c'est la régression exacte \
             du playtest 1 (asymétrie de visibilité) qui ne doit plus jamais se reproduire"
        );
        assert_eq!(players_bob.get(0).id(), alice_cid);
    }

    // --- resolve_protocol_version / resolve_whitelist / Leave (Task C3) --------------------

    #[test]
    fn join_rejected_when_protocol_version_mismatches() {
        assert_eq!(
            resolve_protocol_version(999),
            Err("version du jeu incompatible, mettez à jour votre launcher")
        );
    }

    #[test]
    fn join_accepted_when_protocol_version_matches() {
        assert_eq!(
            resolve_protocol_version(crate::gateway_routing::CURRENT_PROTOCOL_VERSION),
            Ok(())
        );
    }

    #[test]
    fn join_rejected_when_whitelist_enabled_and_name_not_listed() {
        let names: std::collections::HashSet<String> = ["Alice".to_string()].into_iter().collect();
        assert_eq!(
            resolve_whitelist(true, &names, "Mallory"),
            Err("accès restreint (whitelist)")
        );
    }

    #[test]
    fn join_accepted_when_whitelist_enabled_and_name_listed() {
        let names: std::collections::HashSet<String> = ["Alice".to_string()].into_iter().collect();
        assert_eq!(resolve_whitelist(true, &names, "Alice"), Ok(()));
    }

    #[test]
    fn join_accepted_when_whitelist_disabled_regardless_of_name() {
        // Comportement inchangé (défaut) : whitelist désactivée → tout le monde passe, même une
        // liste vide de noms autorisés.
        let names: std::collections::HashSet<String> = std::collections::HashSet::new();
        assert_eq!(resolve_whitelist(false, &names, "AnyoneAtAll"), Ok(()));
    }

    #[test]
    fn leave_message_releases_slot_immediately_unlike_crash_disconnect() {
        use crate::handoff::{Placement, Rank, ShardLoader};
        use crate::persistence::{MemoryStore, PlayerRecord, PlayerStore};
        use crate::rate_limit::RateLimitState;
        use crate::transport::TransportEvent;

        let cid = 1u64;
        let mut store = MemoryStore::new();
        let mut keys = HashMap::new();
        keys.insert(cid, "Alice".to_string());
        let mut display_names = HashMap::new();
        display_names.insert(cid, "Alice".to_string());
        let mut last_pos = HashMap::new();
        last_pos.insert(cid, [1.0, 2.0, 3.0]);
        let mut last_pos_at = HashMap::new();
        last_pos_at.insert(cid, std::time::Instant::now());
        let mut bypass_warned_at = HashMap::new();
        bypass_warned_at.insert(cid, std::time::Instant::now());
        let mut anomaly_trackers = HashMap::new();
        anomaly_trackers.insert(cid, AnomalyTracker::new());
        let mut ranks = HashMap::new();
        ranks.insert(cid, Rank::Player);
        let mut permissions = HashMap::new();
        permissions.insert(cid, vec!["some.node".to_string()]);
        let mut residence = HashMap::new();
        residence.insert(cid, Some([1.0, 2.0, 3.0]));
        let mut rate_states = HashMap::new();
        rate_states.insert(cid, RateLimitState::new(std::time::Instant::now()));
        let mut loader = ShardLoader::new();
        loader.feed(TransportEvent::Connected(cid), None);
        let mut latest: HashMap<u64, HashMap<String, Vec<u8>>> = HashMap::new();
        latest.insert(cid, HashMap::new());
        let mut prev_placements = HashMap::new();
        prev_placements.insert(
            cid,
            Placement {
                authoritative: "A".to_string(),
                overlaps: vec![],
            },
        );

        cleanup_client_state(
            cid,
            &mut store,
            &mut keys,
            &mut display_names,
            &mut last_pos,
            &mut last_pos_at,
            &mut bypass_warned_at,
            &mut anomaly_trackers,
            &mut ranks,
            &mut permissions,
            &mut residence,
            &mut rate_states,
            &mut loader,
            &mut latest,
            &mut prev_placements,
        );

        assert!(keys.is_empty(), "keys doit être nettoyé immédiatement");
        assert!(display_names.is_empty());
        assert!(last_pos.is_empty());
        assert!(last_pos_at.is_empty());
        assert!(bypass_warned_at.is_empty());
        assert!(ranks.is_empty());
        assert!(permissions.is_empty());
        assert!(residence.is_empty());
        assert!(rate_states.is_empty());
        assert!(latest.is_empty());
        assert!(prev_placements.is_empty());
        assert_eq!(
            store.load("Alice"),
            Some(PlayerRecord {
                last_position: [1.0, 2.0, 3.0],
                residence: Some([1.0, 2.0, 3.0]),
            }),
            "la dernière position connue doit être sauvée avant l'oubli, comme au Disconnected"
        );
    }

    // --- resolve_join_key (Task C2, vérification JWT au Join) -------------------------------
    //
    // `JwksCache` (Task C1, `crate::jwks`) n'expose délibérément aucun constructeur de test
    // pré-rempli en dehors de son propre module — cette tâche n'a pas à toucher jwks.rs. La
    // seule voie publique pour peupler un cache de test est donc `refresh()` contre un mini
    // serveur HTTP mocké, exactement comme le fait déjà le test
    // `refresh_populates_cache_from_jwks_endpoint` dans jwks.rs — dupliqué ici à dessein plutôt
    // que rendu public depuis jwks.rs (hors périmètre C2).

    const JOIN_TEST_KID: &str = "gateway-join-test-key-1";

    struct JoinTestRsaKeyMaterial {
        encoding_key: jsonwebtoken::EncodingKey,
        n_b64: String,
        e_b64: String,
    }

    fn generate_join_test_rsa_key_material() -> JoinTestRsaKeyMaterial {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        use rsa::pkcs1::EncodeRsaPrivateKey;
        use rsa::traits::PublicKeyParts;
        use rsa::{RsaPrivateKey, RsaPublicKey};

        let mut rng = rand::rngs::OsRng;
        let private_key =
            RsaPrivateKey::new(&mut rng, 2048).expect("génération de la clé RSA de test");
        let public_key = RsaPublicKey::from(&private_key);
        let pem = private_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .expect("encodage PKCS1 PEM de la clé de test");
        let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes())
            .expect("clé RSA de test illisible par jsonwebtoken");
        let n_b64 = URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be());
        let e_b64 = URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be());
        JoinTestRsaKeyMaterial {
            encoding_key,
            n_b64,
            e_b64,
        }
    }

    fn encode_join_test_token(
        claims: &crate::jwks::Claims,
        key: &jsonwebtoken::EncodingKey,
    ) -> String {
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(JOIN_TEST_KID.to_string());
        jsonwebtoken::encode(&header, claims, key).expect("échec de l'encodage du token de test")
    }

    fn far_future_timestamp() -> u64 {
        9_999_999_999 // an. 2286 — largement suffisant pour ne jamais expirer en test
    }

    /// Démarre un serveur HTTP mocké servant un unique document JWKS ({ "keys": [...] }) et
    /// rafraîchit un `JwksCache` neuf contre lui. Renvoie le cache peuplé + la clé d'encodage
    /// correspondante (pour signer des tokens "valides" dans les tests).
    async fn join_test_jwks_cache_with_valid_key(
    ) -> (crate::jwks::JwksCache, JoinTestRsaKeyMaterial) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let material = generate_join_test_rsa_key_material();
        let body = format!(
            "{{\"keys\":[{{\"kid\":\"{kid}\",\"kty\":\"RSA\",\"n\":\"{n}\",\"e\":\"{e}\"}}]}}",
            kid = JOIN_TEST_KID,
            n = material.n_b64,
            e = material.e_b64,
        );

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind du serveur JWKS mocké");
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept du client HTTP");
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("écriture de la réponse JWKS mockée");
            socket.shutdown().await.ok();
        });

        let jwks_cache = crate::jwks::JwksCache::new();
        jwks_cache
            .refresh(&format!("http://{addr}/jwks"))
            .await
            .expect("refresh doit réussir contre le serveur mocké");

        (jwks_cache, material)
    }

    #[tokio::test]
    async fn join_rejected_when_public_server_receives_no_token() {
        // Serveur public, display_name rempli mais token vide : rejet propre, jamais un timeout.
        let jwks_cache = crate::jwks::JwksCache::new();
        let result = resolve_join_key(true, "SomeDisplayName", "", "launcher", &jwks_cache);
        assert_eq!(result, Err("compte requis sur ce serveur"));
    }

    #[tokio::test]
    async fn join_rejected_when_token_signature_invalid() {
        // Le cache ne connaît que la clé "légitime" ; le token est signé par une AUTRE paire —
        // simule un tiers non autorisé (ou une clé ZITADEL déjà rotée hors du cache).
        let (jwks_cache, _legit_material) = join_test_jwks_cache_with_valid_key().await;
        let other_material = generate_join_test_rsa_key_material();
        let claims = crate::jwks::Claims {
            sub: "user-attacker".into(),
            aud: "launcher".into(),
            name: None,
            preferred_username: None,
            exp: far_future_timestamp(),
        };
        let token = encode_join_test_token(&claims, &other_material.encoding_key);

        let result = resolve_join_key(true, "SomeDisplayName", &token, "launcher", &jwks_cache);
        assert_eq!(result, Err("session invalide, reconnectez-vous"));
    }

    #[tokio::test]
    async fn join_accepted_with_valid_token_uses_sub_as_key() {
        let (jwks_cache, material) = join_test_jwks_cache_with_valid_key().await;
        let claims = crate::jwks::Claims {
            sub: "oidc-user-abc".into(),
            aud: "launcher".into(),
            name: None,
            preferred_username: None,
            exp: far_future_timestamp(),
        };
        let token = encode_join_test_token(&claims, &material.encoding_key);

        // Le display_name fourni ("DisplayNameLibre") n'est PAS la clé retenue : la clé
        // effective doit être le `sub` vérifié, jamais le texte libre non vérifié (root cause
        // du bug playtest 1).
        let result = resolve_join_key(true, "DisplayNameLibre", &token, "launcher", &jwks_cache);
        assert_eq!(result.map(|i| i.key), Ok("oidc-user-abc".to_string()));
    }

    #[tokio::test]
    async fn public_server_display_name_comes_from_verified_jwt_not_windows_username() {
        // Sur un serveur public, le nom AFFICHÉ est dérivé du JWT vérifié (`name`), JAMAIS le
        // `Join.display_name` du client (= username Windows via GetUserNameA côté netcode, usurpable
        // et collisionnant). Deux joueurs « Lucas » (même nom Windows) affichent leur pseudo ZITADEL
        // distinct. Régression cible : « connexion de Lucas » au lieu du pseudo ZITADEL, 2026-07-19.
        let (jwks_cache, material) = join_test_jwks_cache_with_valid_key().await;
        let claims = crate::jwks::Claims {
            sub: "oidc-sub-xyz".into(),
            name: Some("Neo".into()),
            preferred_username: Some("neo_zitadel".into()),
            aud: "launcher".into(),
            exp: far_future_timestamp(),
        };
        let token = encode_join_test_token(&claims, &material.encoding_key);

        let identity = resolve_join_key(true, "LucasWindows", &token, "launcher", &jwks_cache)
            .expect("token valide → join accepté");
        assert_eq!(identity.key, "oidc-sub-xyz", "clé de persistance = sub vérifié");
        assert_eq!(
            identity.display, "Neo",
            "nom affiché = claim `name` du JWT, jamais le username Windows du client"
        );
    }

    #[tokio::test]
    async fn public_display_name_falls_back_preferred_username_then_sub() {
        let (jwks_cache, material) = join_test_jwks_cache_with_valid_key().await;
        // Pas de `name` → repli sur `preferred_username` (le pseudo unique ZITADEL).
        let claims_pu = crate::jwks::Claims {
            sub: "sub-1".into(),
            name: None,
            preferred_username: Some("pseudo_unique".into()),
            aud: "launcher".into(),
            exp: far_future_timestamp(),
        };
        let token_pu = encode_join_test_token(&claims_pu, &material.encoding_key);
        assert_eq!(
            resolve_join_key(true, "X", &token_pu, "launcher", &jwks_cache)
                .unwrap()
                .display,
            "pseudo_unique"
        );
        // Ni `name` ni `preferred_username` → repli ultime sur `sub` (nom jamais vide).
        let claims_sub = crate::jwks::Claims {
            sub: "sub-2".into(),
            name: None,
            preferred_username: None,
            aud: "launcher".into(),
            exp: far_future_timestamp(),
        };
        let token_sub = encode_join_test_token(&claims_sub, &material.encoding_key);
        assert_eq!(
            resolve_join_key(true, "X", &token_sub, "launcher", &jwks_cache)
                .unwrap()
                .display,
            "sub-2"
        );
    }

    #[test]
    fn private_server_display_name_stays_the_client_name() {
        // Serveur privé (pas de JWT) : comportement historique inchangé — clé ET nom = display_name
        // brut du client.
        let jwks_cache = crate::jwks::JwksCache::new();
        let identity = resolve_join_key(false, "Lucas", "", "launcher", &jwks_cache).unwrap();
        assert_eq!(identity.key, "Lucas");
        assert_eq!(identity.display, "Lucas");
    }

    #[tokio::test]
    async fn join_audience_must_match_configured_launcher_client_id() {
        // Régression du placeholder "launcher" jamais réconcilié (nuit du 2026-07-16) : un
        // id_token ZITADEL réel porte `aud` = client_id OIDC du launcher (ex. "340098...@tessera"),
        // JAMAIS la chaîne littérale "launcher". Le serveur doit vérifier ce client_id CONFIGURÉ
        // (`TESSERA_ZITADEL_LAUNCHER_CLIENT_ID`) — sinon tout token réel était rejeté en
        // WrongAudience et personne ne pouvait se connecter à un serveur public.
        let (jwks_cache, material) = join_test_jwks_cache_with_valid_key().await;
        let real_client_id = "340098765@tessera"; // forme d'un client_id ZITADEL réel
        let claims = crate::jwks::Claims {
            sub: "oidc-user-abc".into(),
            aud: real_client_id.into(),
            name: None,
            preferred_username: None,
            exp: far_future_timestamp(),
        };
        let token = encode_join_test_token(&claims, &material.encoding_key);

        // Audience configurée ≠ aud du token → rejet propre (jamais un timeout muet).
        let wrong = resolve_join_key(true, "N", &token, "un-autre-client-id", &jwks_cache);
        assert_eq!(wrong, Err("session invalide, reconnectez-vous"));

        // Audience configurée = le vrai client_id du token → accepté, clé = sub vérifié.
        let ok = resolve_join_key(true, "N", &token, real_client_id, &jwks_cache);
        assert_eq!(ok.map(|i| i.key), Ok("oidc-user-abc".to_string()));
    }

    #[tokio::test]
    async fn private_server_accepts_join_without_token_unchanged() {
        // identity.public = false (ou absent) : comportement historique, token ignoré
        // intégralement, même vide, même si le JwksCache est vide/jamais rafraîchi.
        let jwks_cache = crate::jwks::JwksCache::new();
        let result = resolve_join_key(false, "Lucas", "", "launcher", &jwks_cache);
        assert_eq!(result.map(|i| i.key), Ok("Lucas".to_string()));
    }

    // --- Migration D3 : identité admin résolue par sub OIDC vérifié sur serveur public ---------
    //
    // Task C2 a fait de `keys` la clé de PERSISTANCE effective (le `sub` OIDC vérifié sur un
    // serveur public). Jusqu'à Task D3, `root_admins`/`admin_store` restaient indexés
    // exclusivement par `display_name`, même sur serveur public — ce test reproduit exactement le
    // pipeline de la boucle Gateway (Join → maps `keys`/`display_names` → AdminCommand) et prouve
    // que, depuis Task D3, l'autorité admin est reconnue quand le `sub` vérifié est listé dans
    // `TESSERA_ROOT_ADMINS`, SANS qu'aucun display_name n'y figure — le comportement historique
    // (repli display_name) reste par ailleurs intact.
    #[tokio::test]
    async fn admin_command_resolves_issuer_by_sub_on_public_server() {
        use crate::admin_commands::{
            execute as execute_admin_command, parse as parse_admin_command,
        };

        let (jwks_cache, material) = join_test_jwks_cache_with_valid_key().await;
        let claims = crate::jwks::Claims {
            sub: "oidc-user-xyz".into(),
            aud: "launcher".into(),
            name: None,
            preferred_username: None,
            exp: far_future_timestamp(),
        };
        let token = encode_join_test_token(&claims, &material.encoding_key);
        let display_name = "AdminDisplayName";

        // Join sur un serveur public : la clé effective (persistance) est le `sub` vérifié, pas
        // le display_name — exactement ce que fait `resolve_join_key` (Task C2) dans la boucle.
        let effective_key = resolve_join_key(true, display_name, &token, "launcher", &jwks_cache)
            .expect("token valide, join accepté")
            .key;
        assert_ne!(
            effective_key, display_name,
            "précondition du test : le sub doit diverger du display_name"
        );

        // Reproduit ce que le Join fait dans la boucle : `keys` reçoit la clé effective (le sub),
        // `display_names` reçoit toujours le display_name brut.
        let cid = 42u64;
        let mut keys: HashMap<u64, String> = HashMap::new();
        let mut display_names: HashMap<u64, String> = HashMap::new();
        keys.insert(cid, effective_key.clone());
        display_names.insert(cid, display_name.to_string());

        // `root_admins` (TESSERA_ROOT_ADMINS) liste désormais le `sub`, jamais le display_name —
        // exactement le cas qu'un opérateur ayant fini sa migration vers Task D3 configure.
        let root_admins: std::collections::HashSet<String> =
            [effective_key.clone()].into_iter().collect();

        // Reproduit la résolution du call site Join/AdminCommand (Task D3) : `sub_for_admin` vient
        // de `keys` UNIQUEMENT parce que `identity_public` est vrai ici.
        let issuer = display_names.get(&cid).cloned().unwrap_or_default();
        let sub_for_admin = keys.get(&cid).map(String::as_str);
        let is_root = is_root_by_sub_or_display_name(sub_for_admin, &issuer, &root_admins, false);
        assert!(
            is_root,
            "le sub OIDC vérifié doit être reconnu comme root admin quand il est listé"
        );

        // Et la commande admin s'exécute réellement avec l'issuer = display_name (lisible dans
        // les logs/granted_by), comme au call site réel de `gateway.rs`.
        let mut groups = Vec::new();
        let mut admins = Vec::new();
        let parsed = parse_admin_command("/creategroup moderators").expect("commande valide");
        let outcome = execute_admin_command(parsed, is_root, &mut groups, &mut admins, 0, &issuer);
        assert!(
            outcome.success,
            "la commande admin doit réussir : sub reconnu root admin"
        );
    }

    #[tokio::test]
    async fn admin_command_still_falls_back_to_display_name_when_sub_not_listed() {
        // Repli (Task D3) : `TESSERA_ROOT_ADMINS` liste encore un display_name (transition, ou
        // serveur privé) — le sub vérifié ne doit rien casser, la résolution retombe sur le
        // display_name exactement comme avant la migration.
        let (jwks_cache, material) = join_test_jwks_cache_with_valid_key().await;
        let claims = crate::jwks::Claims {
            sub: "oidc-user-xyz".into(),
            aud: "launcher".into(),
            name: None,
            preferred_username: None,
            exp: far_future_timestamp(),
        };
        let token = encode_join_test_token(&claims, &material.encoding_key);
        let display_name = "AdminDisplayName";
        let effective_key = resolve_join_key(true, display_name, &token, "launcher", &jwks_cache)
            .expect("token valide, join accepté")
            .key;

        let root_admins: std::collections::HashSet<String> =
            [display_name.to_string()].into_iter().collect();

        let is_root = is_root_by_sub_or_display_name(
            Some(effective_key.as_str()),
            display_name,
            &root_admins,
            false,
        );
        assert!(
            is_root,
            "le repli display_name doit continuer de fonctionner"
        );
    }

    #[test]
    fn resolve_move_verdict_bypasses_gamemaster_but_not_moderator_or_player() {
        use crate::anticheat::MoveVerdict;
        use crate::handoff::Rank;
        // Exerce le VRAI code de décision (resolve_move_verdict) : un GameMaster qui téléporte
        // (10 km en 1 tick, distance > RED_TELEPORT_M) doit rester Green — bypass voulu (staff/MJ),
        // jamais "corrigé" par accident. Les autres rangs tombent en Red (téléport franc).
        let prev = [0.0, 0.0, 0.0];
        let teleport = [10_000.0, 0.0, 0.0];
        let elapsed = std::time::Duration::from_millis(50); // un seul tick à 20 Hz

        assert_eq!(
            resolve_move_verdict(Rank::GameMaster, Some((prev, elapsed)), teleport),
            MoveVerdict::Green,
            "GameMaster doit rester Green même sur un téléport implausible"
        );
        assert_eq!(
            resolve_move_verdict(Rank::Player, Some((prev, elapsed)), teleport),
            MoveVerdict::Red,
            "un joueur normal ne doit pas bénéficier du bypass"
        );
        assert_eq!(
            resolve_move_verdict(Rank::Moderator, Some((prev, elapsed)), teleport),
            MoveVerdict::Red,
            "un modérateur ne doit pas bénéficier du bypass (réservé au GameMaster)"
        );
    }

    #[test]
    fn anomalies_below_threshold_do_not_kick() {
        let mut t = AnomalyTracker::new();
        let now = std::time::Instant::now();
        for _ in 0..(ANOMALY_KICK_THRESHOLD - 1) {
            assert!(!record_anomaly(&mut t, now));
        }
    }

    #[test]
    fn anomaly_at_threshold_triggers_kick() {
        let mut t = AnomalyTracker::new();
        let now = std::time::Instant::now();
        let mut kicked = false;
        for _ in 0..ANOMALY_KICK_THRESHOLD {
            kicked = record_anomaly(&mut t, now);
        }
        assert!(
            kicked,
            "la N-ième anomalie dans la fenêtre doit déclencher le kick"
        );
    }

    #[test]
    fn anomalies_outside_window_do_not_accumulate() {
        let mut t = AnomalyTracker::new();
        let t0 = std::time::Instant::now();
        record_anomaly(&mut t, t0);
        let later = t0 + ANOMALY_WINDOW + std::time::Duration::from_secs(1);
        // Après expiration de la fenêtre, on repart de compteur bas : pas de kick sur une 2e isolée.
        assert!(!record_anomaly(&mut t, later));
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

    // --- resolve_join_spawn : reprise hot-cache-first (Décision 3, design stockage 2026-07-09) --
    //
    // Nécessite un vrai Redis local (`docker run -p 6379:6379 redis:7`), même pattern déjà en
    // place dans `hot_state_cache.rs` — pas de mock, conformément au brief.

    #[tokio::test]
    async fn resolve_join_spawn_prefers_hot_cache_over_cold_store_when_both_present() {
        use crate::hot_state_cache::HotStateCache;
        use crate::persistence::{PlayerRecord, SpawnSource};

        let hot_state = HotStateCache::connect("redis://127.0.0.1:6379")
            .await
            .expect("Redis local requis (docker run -p 6379:6379 redis:7)");
        let subject = "gateway-resume-test-hot-wins";
        hot_state.write(subject, [42.0, 43.0, 44.0]).await.unwrap();

        // Store froid VIDE (redémarrage/reconnexion simulé côté Gateway) — le hot cache doit
        // gagner malgré tout, PAS le spawn par défaut.
        let cold_record: Option<PlayerRecord> = None;
        let spawn = [0.0, 0.0, 0.0];

        let (pos, source) =
            resolve_join_spawn(subject, &hot_state, cold_record.as_ref(), spawn).await;
        assert_eq!(
            pos,
            [42.0, 43.0, 44.0],
            "la position chaude doit gagner sur le store froid vide et sur le spawn par défaut"
        );
        assert_eq!(source, SpawnSource::LastPosition);
    }

    #[tokio::test]
    async fn resolve_join_spawn_falls_back_to_cold_store_when_hot_cache_is_empty() {
        use crate::hot_state_cache::HotStateCache;
        use crate::persistence::PlayerRecord;

        let hot_state = HotStateCache::connect("redis://127.0.0.1:6379")
            .await
            .expect("Redis local requis (docker run -p 6379:6379 redis:7)");
        let subject = "gateway-resume-test-cold-fallback-never-written";

        let cold_record = PlayerRecord {
            last_position: [7.0, 8.0, 9.0],
            residence: None,
        };
        let spawn = [0.0, 0.0, 0.0];

        let (pos, _source) =
            resolve_join_spawn(subject, &hot_state, Some(&cold_record), spawn).await;
        assert_eq!(
            pos,
            [7.0, 8.0, 9.0],
            "sans entrée hot-cache, le repli habituel (store froid) doit s'appliquer"
        );
    }

    #[tokio::test]
    async fn resolve_join_spawn_falls_back_to_default_spawn_when_both_hot_and_cold_are_empty() {
        use crate::hot_state_cache::HotStateCache;
        use crate::persistence::{PlayerRecord, SpawnSource};

        let hot_state = HotStateCache::connect("redis://127.0.0.1:6379")
            .await
            .expect("Redis local requis (docker run -p 6379:6379 redis:7)");
        let subject = "gateway-resume-test-nothing-known-anywhere";
        let cold_record: Option<PlayerRecord> = None;
        let spawn = [1.0, 2.0, 3.0];

        let (pos, source) =
            resolve_join_spawn(subject, &hot_state, cold_record.as_ref(), spawn).await;
        assert_eq!(pos, spawn);
        assert_eq!(source, SpawnSource::Spawn);
    }
}
