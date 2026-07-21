//! Boucle serveur : draine les events transport, met à jour le World, diffuse les snapshots.
//! Générique sur `Transport` → testable avec `InMemoryTransport`, branché sur GNS en prod.

use crate::interaction_session::{SessionError, SessionRegistry};
use crate::metrics::Metrics;
use crate::named_npc_catalog::NamedNpcCatalog;
use crate::named_npc_registry::NamedNpcRegistry;
use crate::nav::plan_path;
use crate::nav_graph::{NavGraph, Vec3 as NavVec3};
use crate::nav_state::NavState;
use crate::npc::{
    behavior_to_u8, execute_transaction, EntityBehavior, NpcRecord, TransactionOutcome,
};
use crate::npc_catalog::NpcCatalog;
use crate::population_director::PopulationDirector;
use crate::transport::{ClientId, Transport, TransportEvent};
use crate::world::{Pose, World};
use flatbuffers::FlatBufferBuilder;
use protocol::*;
use std::sync::Arc;

pub struct Server {
    world: World,
    aoi_radius: f32,
    /// File d'événements one-shot du tick courant (actor, kind, action, param) — accumulée par
    /// `apply_client_message`, drainée et relayée aux voisins AoI en fin de `tick()`.
    pending_events: Vec<(ClientId, u8, u8, u32)>,
    /// Métriques partagées (buckets de durée de tick, overruns) — `None` pour tous les appels de
    /// test existants (`Server::new`), qui ne veulent pas dépendre de `Metrics`. `Some(..)`
    /// uniquement pour les `Server` construits via `Server::new_with_metrics` (câblage
    /// `shard_main`, Task 6 observabilité).
    metrics: Option<Arc<Metrics>>,
    /// `None` = aucune simulation PNJ sur ce Shard (comportement historique préservé — tous les
    /// appels `Server::new`/`Server::new_with_metrics` existants restent inchangés et n'activent
    /// jamais les PNJ). `Some(..)` uniquement via `Server::new_with_npcs`.
    npc_registry: Option<NpcRegistry>,
    /// `None` = aucun PNJ nominatif sur ce Shard (comportement historique préservé). `Some(..)`
    /// uniquement via `Server::new_with_named_npcs`.
    named_npc_registry: Option<NamedNpcRegistry>,
    session_registry: SessionRegistry,
    /// File one-shot (actor, target) des `EntityInteraction(kind=2)` sur un PNJ nominatif à
    /// arbitrer en session — accumulée par `apply_client_message`, drainée en début de `tick()`.
    pending_interaction_opens: Vec<(ClientId, ClientId)>,
    /// File one-shot (actor, session_id, outcome, target) des sessions résolues à notifier au
    /// client — accumulée par `apply_client_message`, drainée en fin de `tick()`.
    pending_interaction_results: Vec<(ClientId, u64, TransactionOutcome, ClientId)>,
    /// Graphe de navigation (nav_graph.rs) — `None` = aucun PNJ ne se déplace (comportement
    /// fondation PNJ préservé : sans graphe, decide_destination peut produire une destination mais
    /// aucun chemin n'est jamais planifié, donc aucun mouvement). `Some(..)` posé après construction
    /// via un setter dédié (`set_nav_graph`) plutôt qu'un paramètre de constructeur — la navigation
    /// est un comportement additif, pas une variante de configuration comme catalog/director/registry.
    nav_graph: Option<NavGraph>,
}

/// Regroupe l'état PNJ vivant d'un Shard : le catalogue (immuable après boot), le director
/// (config immuable), et les enregistrements PNJ actifs (mutables à chaque tick).
struct NpcRegistry {
    catalog: NpcCatalog,
    director: PopulationDirector,
    records: std::collections::HashMap<ClientId, NpcRecord>,
    next_npc_id: ClientId,
    /// Chemin/progression courants par PNJ (Task 3/5, plan navigation). Absent d'un id =
    /// « n'a jamais eu de destination assignée » — traité comme un NavState vierge.
    nav_states: std::collections::HashMap<ClientId, NavState>,
}

impl Server {
    pub fn new(aoi_radius: f32) -> Self {
        Self {
            world: World::new(),
            aoi_radius,
            pending_events: Vec::new(),
            metrics: None,
            npc_registry: None,
            named_npc_registry: None,
            session_registry: SessionRegistry::new(),
            pending_interaction_opens: Vec::new(),
            pending_interaction_results: Vec::new(),
            nav_graph: None,
        }
    }

    /// Identique à `Server::new`, avec en plus l'enregistrement de la durée de chaque tick dans
    /// `metrics` (buckets d'histogramme + compteur d'overruns, cf. `metrics.rs`). Nouvelle
    /// méthode plutôt qu'un changement de signature de `Server::new` : ce dernier est déjà
    /// appelé par de nombreux tests existants (`server_loop.rs`, `gateway.rs`, `shard.rs`) qui ne
    /// doivent pas être modifiés pour cette tâche.
    pub fn new_with_metrics(aoi_radius: f32, metrics: Arc<Metrics>) -> Self {
        Self {
            world: World::new(),
            aoi_radius,
            pending_events: Vec::new(),
            metrics: Some(metrics),
            npc_registry: None,
            named_npc_registry: None,
            session_registry: SessionRegistry::new(),
            pending_interaction_opens: Vec::new(),
            pending_interaction_results: Vec::new(),
            nav_graph: None,
        }
    }

    /// Identique à `Server::new`, avec en plus un registre PNJ actif (catalogue + director). Le
    /// director raisonne sur un unique district logique "default" dans cette fondation — le
    /// multi-district réel (topologie de shards) est un raffinement différé, pas câblé ici.
    /// Nouvelle méthode plutôt qu'un changement de signature de `Server::new`/`new_with_metrics` :
    /// ces deux constructeurs sont déjà appelés par de nombreux tests existants qui ne doivent pas
    /// changer pour cette tâche (même raisonnement que `new_with_metrics` lui-même).
    pub fn new_with_npcs(
        aoi_radius: f32,
        catalog: NpcCatalog,
        director: PopulationDirector,
    ) -> Self {
        Self {
            world: World::new(),
            aoi_radius,
            pending_events: Vec::new(),
            metrics: None,
            npc_registry: Some(NpcRegistry {
                catalog,
                director,
                records: std::collections::HashMap::new(),
                next_npc_id: crate::world::NPC_ID_RANGE_START,
                nav_states: std::collections::HashMap::new(),
            }),
            named_npc_registry: None,
            session_registry: SessionRegistry::new(),
            pending_interaction_opens: Vec::new(),
            pending_interaction_results: Vec::new(),
            nav_graph: None,
        }
    }

    /// Identique à `Server::new`, avec en plus un registre de PNJ nominatifs actif — SPAWNÉS dans
    /// `World` dès la construction (position/pose initiale tirée de `catalog`), donc immédiatement
    /// visibles dans un snapshot de joueur proche. Prend `catalog` ET `named_npc_registry` (pas
    /// seulement le registre d'ids) précisément pour pouvoir faire ce spawn ici, en un seul
    /// endroit — Task 7 (câblage boot) n'aura donc RIEN à ajouter au-delà de charger le catalogue
    /// et appeler ce constructeur ; cette signature est définitive dès cette tâche, ne change pas
    /// dans les tâches suivantes. Nouvelle méthode plutôt qu'un changement de signature de
    /// `Server::new`/`new_with_metrics`/`new_with_npcs` : ces trois constructeurs sont déjà
    /// appelés par de nombreux tests existants qui ne doivent pas changer pour cette tâche.
    pub fn new_with_named_npcs(
        aoi_radius: f32,
        catalog: &NamedNpcCatalog,
        named_npc_registry: NamedNpcRegistry,
    ) -> Self {
        let mut world = World::new();
        for runtime_id in named_npc_registry.runtime_ids() {
            let Some(manifest_id) = named_npc_registry.manifest_id_of(runtime_id) else {
                continue;
            };
            let Some(config) = catalog.get(manifest_id) else {
                continue;
            };
            world.add_player(runtime_id);
            world.set_pose(
                runtime_id,
                Pose {
                    x: config.position[0],
                    y: config.position[1],
                    z: config.position[2],
                    ..Default::default()
                },
            );
        }
        Self {
            world,
            aoi_radius,
            pending_events: Vec::new(),
            metrics: None,
            npc_registry: None,
            named_npc_registry: Some(named_npc_registry),
            session_registry: SessionRegistry::new(),
            pending_interaction_opens: Vec::new(),
            pending_interaction_results: Vec::new(),
            nav_graph: None,
        }
    }

    /// Nombre de joueurs actuellement dans le monde de ce Shard — pour l'endpoint métriques.
    pub fn player_count(&self) -> usize {
        self.world.player_ids().len()
    }

    /// Attache un graphe de navigation après construction. Sans appel, `tick_npcs` continue de
    /// fonctionner exactement comme avant ce plan (FSM/briques nav-indépendantes seules, aucun
    /// PNJ ne bouge) — comportement historique strictement préservé.
    pub fn set_nav_graph(&mut self, graph: NavGraph) {
        self.nav_graph = Some(graph);
    }

    /// Un pas du director de population + du moteur de briques PNJ (spec fondation PNJ) — no-op si
    /// ce `Server` n'a pas de registre PNJ (`Server::new`/`new_with_metrics`).
    ///
    /// Simplification assumée pour cette fondation (voir Step 7 du plan Task 6) : le director ne
    /// raisonne PAS sur la vraie topologie multi-district (`authority.json`/`tools/district-topology`,
    /// câblée côté Gateway/`handoff.rs`, hors périmètre ici). Tous les joueurs connus de CE `Server`
    /// comptent comme présents dans un unique district logique `"default"`. Le câblage réel
    /// multi-district est un raffinement explicitement différé, pas un oubli.
    fn tick_npcs(&mut self) {
        let Some(registry) = &mut self.npc_registry else {
            return;
        };
        // Trouvé en revue finale de branche (plan navigation) : player_ids() renvoie TOUT le
        // monde dans World, PNJ compris (mêmes add_player/remove_player, décision délibérée de la
        // fondation PNJ pour réutiliser snapshot_for/la grille spatiale telles quelles). Sans ce
        // filtre, dès qu'au moins un PNJ existe, ce compte ne retombe jamais à zéro même si tous
        // les vrais joueurs se déconnectent — le chemin de despawn-sur-district-vide
        // (population_director.rs, !has_players) devient inatteignable en pratique. Bug
        // préexistant à ce plan (présent depuis la fondation PNJ), corrigé ici car c'est ce même
        // plan qui l'a fait échouer pour de vrai (test de nettoyage nav_states, Task 6).
        let player_count = self
            .world
            .player_ids()
            .into_iter()
            .filter(|id| !crate::world::is_npc_id(*id))
            .count() as u32;
        let players_by_district =
            std::collections::HashMap::from([("default".to_string(), player_count)]);
        let existing_by_district = std::collections::HashMap::from([(
            "default".to_string(),
            registry.records.len() as u32,
        )]);
        let actions = registry.director.reconcile(
            &registry.catalog,
            &players_by_district,
            &existing_by_district,
        );
        for action in actions {
            match action {
                crate::population_director::DirectorAction::Spawn { archetype_id, .. } => {
                    let id = registry.next_npc_id;
                    registry.next_npc_id += 1;
                    registry
                        .records
                        .insert(id, NpcRecord::new(id, archetype_id));
                    self.world.add_player(id);
                }
                crate::population_director::DirectorAction::Despawn { excess, .. } => {
                    let to_remove: Vec<ClientId> = registry
                        .records
                        .keys()
                        .take(excess as usize)
                        .copied()
                        .collect();
                    for id in to_remove {
                        registry.records.remove(&id);
                        registry.nav_states.remove(&id);
                        self.world.remove_player(id);
                    }
                }
            }
        }
        const NPC_SPEED_UNITS_PER_SEC: f32 = 3.0; // marche (spec parle de vitesse par brique/archétype
                                                  // via .aiarch, différé — constante v1 uniforme ici,
                                                  // documentée comme simplification assumée).
        let tick_dt = 1.0 / crate::default_tick_rate_hz() as f32;
        let move_distance = NPC_SPEED_UNITS_PER_SEC * tick_dt;

        for (id, record) in registry.records.iter_mut() {
            let Some(archetype) = registry.catalog.archetype(record.archetype) else {
                continue;
            };
            record.apply_brique_tick(archetype);

            let Some(graph) = &self.nav_graph else {
                continue; // comportement historique préservé sans graphe (Global Constraints)
            };
            let nav_state = registry.nav_states.entry(*id).or_default();
            let current_pose = self.world.pose_of(*id).unwrap_or_default();
            let current_pos = (current_pose.x, current_pose.y, current_pose.z);

            if !nav_state.has_path() || nav_state.has_arrived() {
                // Pas de chemin en cours (ou arrivé) -> décide d'une nouvelle destination et planifie.
                // region_center/region_radius v1 : la position ACTUELLE du PNJ (simplification assumée —
                // le vrai centre de région par archétype, spec §4, attend une config dédiée future).
                // menace_or_cible : résolue via World::pose_of sur l'id porté par le behavior FSM (Fuite/
                // Hostile) — sans ceci, un PNJ en fuite ne saurait jamais de quelle position s'éloigner
                // (npc.rs::decide_destination retourne None sans position réelle pour Fuite, testé
                // séparément). Calme/Flane/ATerre n'utilisent jamais cette valeur.
                let menace_or_cible = match record.behavior {
                    crate::npc::EntityBehavior::Fuite { menace } => {
                        self.world.pose_of(menace).map(|p| (p.x, p.y, p.z))
                    }
                    crate::npc::EntityBehavior::Hostile { cible } => {
                        self.world.pose_of(cible).map(|p| (p.x, p.y, p.z))
                    }
                    _ => None,
                };
                if let Some(dest) = crate::npc::decide_destination(
                    record.behavior,
                    archetype,
                    current_pos,
                    menace_or_cible,
                    current_pos,
                    15.0,
                    pseudo_random_unit(*id, self.world.tick()),
                ) {
                    let (dx, dy, dz) = dest;
                    if let Some(path) = plan_path(
                        graph,
                        NavVec3::new(current_pos.0, current_pos.1, current_pos.2),
                        NavVec3::new(dx, dy, dz),
                    ) {
                        nav_state.set_path(path);
                    }
                }
            }

            if nav_state.has_path() {
                let new_pos = nav_state.advance(
                    NavVec3::new(current_pos.0, current_pos.1, current_pos.2),
                    move_distance,
                );
                self.world.set_pose(
                    *id,
                    Pose {
                        x: new_pos.x,
                        y: new_pos.y,
                        z: new_pos.z,
                        locomotion: 1, // Walk (cf. commentaire triplet biped, world.rs) — v1 uniforme,
                        // Sprint réservé à Fuite en raffinement futur (spec §5)
                        ..current_pose
                    },
                );
            }
        }
    }

    /// Un tick : applique les events entrants, avance le monde, envoie un snapshot à chaque client.
    pub fn tick<T: Transport>(&mut self, transport: &mut T) {
        let tick_start = std::time::Instant::now();
        for ev in transport.poll() {
            match ev {
                TransportEvent::Connected(id) => self.world.add_player(id),
                TransportEvent::Disconnected(id) => self.world.remove_player(id),
                TransportEvent::Message { from, data } => self.apply_client_message(from, &data),
            }
        }
        self.world.advance_tick();
        self.tick_npcs();
        // Réclame les sessions d'interaction jamais résolues (client qui ouvre puis se déconnecte
        // sans jamais répondre) — trouvé en revue finale de branche (fondation d'interaction) :
        // SessionRegistry::expire_stale existait et était testé (Task 1) mais n'était jamais
        // appelé, laissant les sessions abandonnées s'accumuler sans borne sur un Shard longue
        // durée. 30s : généreux pour un joueur qui parcourt une offre avant de répondre, borné
        // pour ne pas laisser une session fantôme vivre indéfiniment (spec fondation d'interaction
        // §2 : « timeout serveur », aucune valeur numérique imposée par la spec).
        const INTERACTION_SESSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
        self.session_registry
            .expire_stale(INTERACTION_SESSION_TIMEOUT);
        for (actor, target) in self.pending_interaction_opens.drain(..) {
            let session_id = self.session_registry.open(actor, target, 0);
            let bytes = crate::gateway_routing::encode_interaction_open(session_id, target, 0, &[]);
            transport.send(actor, &bytes);
        }
        for (actor, session_id, outcome, target) in self.pending_interaction_results.drain(..) {
            let bytes = crate::gateway_routing::encode_interaction_result(
                session_id,
                outcome.ok,
                &outcome.payload,
            );
            transport.send(actor, &bytes);
            // Règle du log RP (spec §2/§7) : toute résolution, succès ou refus, est journalisée. Le
            // log lui-même vit dans gateway.rs (SessionLog est possédé par le Gateway, pas par
            // Server) — Server expose l'événement via un canal que Task 7/gateway.rs consomme ;
            // pour cette tâche, l'événement SessionEvent::InteractionResolved existe (session_log.rs)
            // mais n'est PAS encore écrit depuis ici (Server n'a et ne doit pas avoir de dépendance
            // sur session_log.rs, qui vit au niveau Gateway) — limitation assumée, cf. rapport de
            // tâche.
            let _ = target;
        }
        let mut b = FlatBufferBuilder::new();
        for id in self.world.player_ids() {
            b.reset();
            let bytes = self.encode_snapshot_for(id, &mut b);
            transport.send(id, &bytes);
        }
        // Relais des événements one-shot du tick, filtré par le même AoI que les snapshots.
        for (actor, kind, action, param) in self.pending_events.drain(..) {
            let neighbors = self.world.snapshot_for(actor, self.aoi_radius);
            for (neighbor_id, _) in neighbors {
                b.reset();
                let ev = PlayerEvent::create(
                    &mut b,
                    &PlayerEventArgs {
                        actor,
                        kind,
                        action,
                        param,
                    },
                );
                let env = ServerEnvelope::create(
                    &mut b,
                    &ServerEnvelopeArgs {
                        msg_type: ServerMsg::PlayerEvent,
                        msg: Some(ev.as_union_value()),
                    },
                );
                b.finish(env, None);
                transport.send(neighbor_id, b.finished_data());
            }
        }
        if let Some(metrics) = &self.metrics {
            metrics.record_tick_duration_micros(tick_start.elapsed().as_micros() as u64);
        }
    }

    fn apply_client_message(&mut self, from: ClientId, data: &[u8]) {
        let Ok(env) = flatbuffers::root::<ClientEnvelope>(data) else {
            return;
        };
        match env.msg_type() {
            ClientMsg::Join => { /* TODO(Phase-1): stocker le display_name */ }
            ClientMsg::PositionUpdate => {
                if let Some(pu) = env.msg_as_position_update() {
                    if let Some(p) = pu.position() {
                        self.world.set_pose(
                            from,
                            Pose {
                                x: p.x(),
                                y: p.y(),
                                z: p.z(),
                                yaw: pu.yaw(),
                                ..self.world.pose_of(from).unwrap_or_default()
                            },
                        );
                    }
                    self.world
                        .set_locomotion(from, pu.locomotion(), pu.move_dir(), pu.flags());
                }
            }
            ClientMsg::EmoteReport => {
                if let Some(er) = env.msg_as_emote_report() {
                    let emote = if er.start() { er.emote() } else { 0 };
                    self.world.set_sustained(from, emote);
                }
            }
            ClientMsg::PlayerActionReport => {
                if let Some(ar) = env.msg_as_player_action_report() {
                    // kind=0=Action (seul type existant pour l'instant, cf. schéma). Relayé en fin
                    // de tick, filtré par AoI — jamais appliqué à la position/locomotion (canal
                    // cosmétique one-shot).
                    self.pending_events.push((from, 0, ar.action(), ar.param()));
                }
            }
            ClientMsg::EntityInteraction => {
                if let Some(ei) = env.msg_as_entity_interaction() {
                    // Comportement existant (fondation PNJ) : transition FSM sur la foule anonyme —
                    // INCHANGÉ, ne pas toucher cette branche.
                    if let Some(registry) = &mut self.npc_registry {
                        if let Some(record) = registry.records.get_mut(&ei.target()) {
                            record.apply_interaction(from, ei.kind());
                        }
                    }
                    // Nouveau (fondation d'interaction) : kind=2=Interagit sur un PNJ NOMINATIF
                    // ouvre une session. Un PNJ nominatif n'a pas de NpcRecord (pas de FSM propre
                    // dans cette fondation — Calme par défaut, cf. Task 7) donc
                    // interaction_allowed(Calme) est toujours vrai ici ; le refus FSM réel (spec
                    // §2, "un vendeur fuite/à terre refuse") attend que les PNJ nominatifs aient
                    // leur propre NpcRecord, hors périmètre de cette tâche.
                    if ei.kind() == 2 {
                        if let Some(named) = &self.named_npc_registry {
                            if named.manifest_id_of(ei.target()).is_some() {
                                self.pending_interaction_opens.push((from, ei.target()));
                            }
                        }
                    }
                }
            }
            ClientMsg::InteractionChoice => {
                if let Some(ic) = env.msg_as_interaction_choice() {
                    match self.session_registry.resolve(ic.session_id(), from) {
                        Ok(session) => {
                            let outcome =
                                execute_transaction(EntityBehavior::Calme, || TransactionOutcome {
                                    ok: true,
                                    payload: Vec::new(),
                                });
                            self.pending_interaction_results.push((
                                from,
                                ic.session_id(),
                                outcome,
                                session.target,
                            ));
                        }
                        Err(SessionError::NotFound) | Err(SessionError::NotOwner) => {
                            self.pending_interaction_results.push((
                                from,
                                ic.session_id(),
                                TransactionOutcome {
                                    ok: false,
                                    payload: Vec::new(),
                                },
                                0,
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn encode_snapshot_for(&self, viewer: ClientId, b: &mut FlatBufferBuilder) -> Vec<u8> {
        let states: Vec<_> = self
            .world
            .snapshot_for(viewer, self.aoi_radius)
            .into_iter()
            .map(|(id, pose)| {
                let pos = Vec3::new(pose.x, pose.y, pose.z);
                PlayerState::create(
                    b,
                    &PlayerStateArgs {
                        id,
                        position: Some(&pos),
                        yaw: pose.yaw,
                        locomotion: pose.locomotion,
                        move_dir: pose.move_dir,
                        flags: pose.flags,
                        sustained: pose.sustained,
                    },
                )
            })
            .collect();
        let npc_states: Vec<_> = self
            .npc_registry
            .iter()
            .flat_map(|r| r.records.values())
            .filter_map(|record| {
                let pose = self.world.pose_of(record.id)?;
                Some(NpcState::create(
                    b,
                    &NpcStateArgs {
                        id: record.id,
                        archetype: record.archetype,
                        position: Some(&Vec3::new(pose.x, pose.y, pose.z)),
                        yaw: pose.yaw,
                        locomotion: pose.locomotion,
                        move_dir: pose.move_dir,
                        flags: pose.flags,
                        sustained: pose.sustained,
                        behavior: behavior_to_u8(record.behavior),
                    },
                ))
            })
            .collect();
        let players = b.create_vector(&states);
        let npcs = b.create_vector(&npc_states);
        let snap = Snapshot::create(
            b,
            &SnapshotArgs {
                tick: self.world.tick(),
                players: Some(players),
                npcs: Some(npcs),
                // Câblage réel du registre de véhicules = Task 5 (cette task ne fait que le
                // schéma). VehicleState/VehicleStateArgs sont déjà disponibles via `protocol::*`.
                vehicles: None,
            },
        );
        let env = ServerEnvelope::create(
            b,
            &ServerEnvelopeArgs {
                msg_type: ServerMsg::Snapshot,
                msg: Some(snap.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }
}

/// Pseudo-aléatoire déterministe [0.0, 1.0) dérivé de l'id du PNJ + du tick courant — pas un vrai
/// RNG (pas de dépendance `rand` ajoutée), suffisant pour varier les destinations `errer` sans
/// motif visible à l'échelle d'un playtest. Documenté comme simplification v1 assumée.
fn pseudo_random_unit(seed: ClientId, tick: u64) -> f32 {
    let mixed = seed
        .wrapping_mul(2654435761)
        .wrapping_add(tick.wrapping_mul(0x9E3779B97F4A7C15));
    ((mixed >> 40) as f32) / (1u64 << 24) as f32 % 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::InMemoryTransport;

    #[test]
    fn a_server_without_npcs_never_adds_any_npc_state_to_the_snapshot() {
        // Comportement historique préservé : Server::new (sans PNJ) ne doit jamais faire apparaître
        // de NpcState dans un Snapshot, même après plusieurs ticks.
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        let sent = t.take_sent(1);
        let env = flatbuffers::root::<ServerEnvelope>(sent.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert!(
            snap.npcs().map(|v| v.len()).unwrap_or(0) == 0,
            "sans registre PNJ, npcs doit rester vide"
        );
    }

    #[test]
    fn a_server_with_npcs_configured_spawns_and_reports_npcs_in_the_snapshot() {
        use crate::npc_catalog::parse_and_validate;
        use crate::population_director::PopulationDirector;
        use std::collections::HashMap;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "marcheur-de-rue"
            briques = ["flaner-sur-place"]
            "#,
        )
        .unwrap();
        // NOTE : le district est nommé "default" et non "centre" — cette fondation ne raisonne
        // que sur un unique district logique "default" regroupant tous les joueurs connus de ce
        // `Server` (simplification assumée, cf. commentaire sur `Server::tick_npcs` : la vraie
        // topologie multi-district est un raffinement différé, hors périmètre ici).
        let director = PopulationDirector::new(HashMap::from([("default".to_string(), 1)]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        // Tout joueur connu de ce `Server` compte comme présent dans le district "default" —
        // Server::tick_npcs résout ainsi la présence (cf. Step 7 du plan Task 6).
        server.tick(&mut t);
        server.tick(&mut t); // un 2e tick pour laisser le director réagir à la présence du joueur

        let sent = t.take_sent(1);
        let env = flatbuffers::root::<ServerEnvelope>(sent.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert!(
            snap.npcs().map(|v| v.len()).unwrap_or(0) > 0,
            "un director configuré avec un joueur présent doit finir par faire apparaître au moins un PNJ"
        );
    }

    fn encode_position(x: f32, y: f32, z: f32, yaw: f32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let pos = Vec3::new(x, y, z);
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                position: Some(&pos),
                yaw,
                locomotion: 0,
                move_dir: 0,
                flags: 0,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::PositionUpdate,
                msg: Some(pu.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn two_clients_see_each_other_move() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();

        // Deux clients se connectent.
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        // Client 1 bouge en (5,0,0).
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position(5.0, 0.0, 0.0, 0.0),
        });

        server.tick(&mut t);

        // Le client 2 doit recevoir un snapshot contenant le joueur 1 en x=5.
        let sent_to_2 = t.take_sent(2);
        assert_eq!(sent_to_2.len(), 1, "un snapshot envoyé au client 2");
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let players = snap.players().unwrap();
        assert_eq!(players.len(), 1);
        let p = players.get(0);
        assert_eq!(p.id(), 1);
        assert_eq!(p.position().unwrap().x(), 5.0);

        let sent_to_1 = t.take_sent(1);
        assert_eq!(sent_to_1.len(), 1, "un snapshot envoyé au client 1");
        let env1 = flatbuffers::root::<ServerEnvelope>(&sent_to_1[0]).unwrap();
        let snap1 = env1.msg_as_snapshot().unwrap();
        let players1 = snap1.players().unwrap();
        assert_eq!(players1.len(), 1);
        assert_eq!(players1.get(0).id(), 2); // client 1 voit client 2
    }

    fn encode_position_with_locomotion(
        x: f32,
        y: f32,
        z: f32,
        yaw: f32,
        locomotion: u8,
        move_dir: u8,
    ) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let pos = Vec3::new(x, y, z);
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                position: Some(&pos),
                yaw,
                locomotion,
                move_dir,
                flags: 0,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::PositionUpdate,
                msg: Some(pu.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn position_update_carries_locomotion_into_snapshot() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(5.0, 0.0, 0.0, 0.0, 3, 42),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let p = snap.players().unwrap().get(0);
        assert_eq!(p.locomotion(), 3);
        assert_eq!(p.move_dir(), 42);
    }

    #[test]
    fn repeated_position_updates_do_not_reset_locomotion_to_idle() {
        // Piège identifié en Task 2 : set_pose remplace toute la Pose. Un deuxième PositionUpdate
        // (même sans nouveau champ de locomotion explicite envoyé par le client, qui renvoie
        // toujours son état courant à chaque update selon la spec §8.1) ne doit jamais faire
        // disparaître la valeur précédente si le client continue de la reporter correctement —
        // ce test vérifie surtout que l'ordre d'application (set_pose puis set_locomotion, ou une
        // fusion) ne perd pas le champ dans le MÊME message.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(5.0, 0.0, 0.0, 0.0, 2, 5),
        });
        server.tick(&mut t);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(6.0, 0.0, 0.0, 0.0, 2, 5),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let p = snap.players().unwrap().get(0);
        assert_eq!(p.position().unwrap().x(), 6.0);
        assert_eq!(p.locomotion(), 2);
    }

    #[test]
    fn position_update_never_touches_sustained() {
        // Le canal cosmétique continu (locomotion) et la pose tenue (sustained, pilotée par
        // EmoteReport UNIQUEMENT) doivent rester complètement indépendants — un PositionUpdate ne
        // doit jamais remettre sustained à 0 s'il était déjà posé.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        // (Task 4 posera sustained via EmoteReport ; ici on vérifie juste qu'un PositionUpdate seul
        // sur un joueur au sustained par défaut à 0 le laisse à 0 - non-régression basique, le test
        // complet d'indépendance vraie est en Task 4 une fois EmoteReport câblé.)
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(5.0, 0.0, 0.0, 0.0, 1, 0),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert_eq!(snap.players().unwrap().get(0).sustained(), 0);
    }

    fn encode_emote_report(emote: u32, start: bool) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let er = EmoteReport::create(&mut b, &EmoteReportArgs { emote, start });
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::EmoteReport,
                msg: Some(er.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn emote_report_start_sets_sustained_in_snapshot() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(7, true),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert_eq!(snap.players().unwrap().get(0).sustained(), 7);
    }

    #[test]
    fn emote_report_stop_clears_sustained() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(7, true),
        });
        server.tick(&mut t);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(7, false),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert_eq!(snap.players().unwrap().get(0).sustained(), 0);
    }

    #[test]
    fn sustained_emote_survives_a_subsequent_position_update() {
        // LE test clé du raffinement §5 de la spec : l'état continu (sustained) doit survivre à un
        // PositionUpdate qui suit — les deux canaux sont indépendants.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(9, true),
        });
        server.tick(&mut t);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(1.0, 0.0, 0.0, 0.0, 0, 0),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let p = snap.players().unwrap().get(0);
        assert_eq!(
            p.sustained(),
            9,
            "la pose tenue doit survivre au PositionUpdate suivant"
        );
        assert_eq!(p.position().unwrap().x(), 1.0);
    }

    fn encode_player_action(action: u8, param: u32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let ar = PlayerActionReport::create(&mut b, &PlayerActionReportArgs { action, param });
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::PlayerActionReport,
                msg: Some(ar.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    fn decode_player_event(bytes: &[u8]) -> Option<(u64, u8, u8, u32)> {
        let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
        if env.msg_type() != ServerMsg::PlayerEvent {
            return None;
        }
        let pe = env.msg_as_player_event()?;
        Some((pe.actor(), pe.kind(), pe.action(), pe.param()))
    }

    #[test]
    fn player_action_report_relays_player_event_to_aoi_neighbor() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(5, 99),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        // sent_to_2 contient le Snapshot ET le PlayerEvent — filtrer par type.
        let event = sent_to_2.iter().find_map(|b| decode_player_event(b));
        let (actor, kind, action, param) = event.expect("le voisin doit recevoir le PlayerEvent");
        assert_eq!(actor, 1);
        assert_eq!(kind, 0);
        assert_eq!(action, 5);
        assert_eq!(param, 99);
    }

    #[test]
    fn player_action_report_not_relayed_outside_aoi_radius() {
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(500.0, 0.0, 0.0, 0.0, 0, 0),
        });
        server.tick(&mut t);
        t.take_sent(2); // vider le snapshot du premier tick
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(5, 99),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let event = sent_to_2.iter().find_map(|b| decode_player_event(b));
        assert!(
            event.is_none(),
            "le joueur 2 est hors AoI (500 > 50), ne doit rien recevoir"
        );
    }

    #[test]
    fn player_action_report_never_touches_position_or_locomotion() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(3.0, 0.0, 0.0, 0.0, 2, 0),
        });
        server.tick(&mut t);
        t.take_sent(2);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(1, 0),
        }); // ex. Jump
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(
            sent_to_2
                .iter()
                .find(|b| {
                    flatbuffers::root::<ServerEnvelope>(b)
                        .map(|e| e.msg_type() == ServerMsg::Snapshot)
                        .unwrap_or(false)
                })
                .unwrap(),
        )
        .unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let p = snap.players().unwrap().get(0);
        assert_eq!(
            p.position().unwrap().x(),
            3.0,
            "un PlayerActionReport ne doit jamais déplacer le joueur"
        );
        assert_eq!(p.locomotion(), 2, "ni changer sa locomotion continue");
    }

    #[test]
    fn players_far_apart_do_not_see_each_other() {
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();

        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position(500.0, 0.0, 0.0, 0.0),
        });

        server.tick(&mut t);

        let sent_to_2 = t.take_sent(2);
        assert_eq!(sent_to_2.len(), 1);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let players = snap.players().unwrap();
        assert_eq!(
            players.len(),
            0,
            "client 1 est à 500 unités, hors du rayon de 50 — ne doit pas apparaître"
        );
    }

    #[test]
    fn cosmetic_channel_events_never_change_player_count_or_connectivity() {
        // Aucun message du canal d'état (PositionUpdate enrichi, EmoteReport, PlayerActionReport)
        // ne doit jamais connecter/déconnecter un joueur ni modifier player_count.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        server.tick(&mut t);
        assert_eq!(server.player_count(), 2);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(1, true),
        });
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(1, 0),
        });
        server.tick(&mut t);
        assert_eq!(
            server.player_count(),
            2,
            "le canal cosmétique ne doit jamais affecter la connectivité"
        );
    }

    #[test]
    fn late_aoi_joiner_learns_sustained_pose_from_snapshot_not_from_missed_event() {
        // Le test clé §5 de la spec, version bout-en-bout via Server (pas juste World, déjà couvert
        // en Task 4) : un joueur qui rejoint l'AoI APRÈS le début d'une pose tenue doit quand même
        // la voir dans son PREMIER snapshot (auto-cicatrisant), sans avoir reçu l'EmoteReport lui-même.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(3, true),
        });
        server.tick(&mut t);
        // Le joueur 2 arrive APRÈS le début de la pose.
        t.inject(TransportEvent::Connected(2));
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(
            sent_to_2
                .iter()
                .find(|b| {
                    flatbuffers::root::<ServerEnvelope>(b)
                        .map(|e| e.msg_type() == ServerMsg::Snapshot)
                        .unwrap_or(false)
                })
                .unwrap(),
        )
        .unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert_eq!(
            snap.players().unwrap().get(0).sustained(),
            3,
            "un arrivant tardif doit lire la pose depuis le snapshot"
        );
    }

    #[test]
    fn one_shot_event_not_resent_to_late_joiner() {
        // Le contraste du test précédent : un ÉVÉNEMENT one-shot (PlayerActionReport→PlayerEvent)
        // n'est PAS auto-cicatrisant — un arrivant tardif ne le reçoit pas rétroactivement, c'est
        // le comportement voulu (rater un one-shot est inoffensif, cf. spec §5).
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(1, 0),
        });
        server.tick(&mut t); // aucun voisin au moment de l'action — rien relayé, personne pour le recevoir
        t.inject(TransportEvent::Connected(2));
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let event = sent_to_2.iter().find_map(|b| decode_player_event(b));
        assert!(
            event.is_none(),
            "un one-shot manqué reste manqué, pas de rattrapage"
        );
    }

    fn encode_entity_interaction(target: u64, kind: u8, param: u32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let ei = EntityInteraction::create(
            &mut b,
            &EntityInteractionArgs {
                target,
                kind,
                param,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::EntityInteraction,
                msg: Some(ei.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    fn encode_interaction_choice(session_id: u64, choice: u8, param: u32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let ic = InteractionChoice::create(
            &mut b,
            &InteractionChoiceArgs {
                session_id,
                choice,
                param,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::InteractionChoice,
                msg: Some(ic.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn a_server_without_named_npcs_never_opens_a_session_on_interagit() {
        // Comportement historique préservé : sans NamedNpcRegistry configuré, un EntityInteraction
        // kind=2 (Interagit) ne doit jamais produire d'InteractionOpen — juste rien (comme avant ce
        // plan, où kind=2 était déjà silencieusement ignoré par apply_interaction).
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        t.take_sent(1); // purge le premier snapshot

        let interaction = encode_entity_interaction(999, 2, 0); // target=999 (aucun PNJ n'existe)
        t.inject(TransportEvent::Message {
            from: 1,
            data: interaction,
        });
        server.tick(&mut t);
        let sent = t.take_sent(1);
        let has_open = sent.iter().any(|bytes| {
            flatbuffers::root::<ServerEnvelope>(bytes)
                .map(|env| env.msg_type() == ServerMsg::InteractionOpen)
                .unwrap_or(false)
        });
        assert!(
            !has_open,
            "sans registre nominatif, aucune InteractionOpen ne doit être envoyée"
        );
    }

    #[test]
    fn interacting_with_a_named_npc_opens_a_session_and_a_choice_resolves_it() {
        use crate::named_npc_catalog::parse_and_validate;
        use crate::named_npc_registry::NamedNpcRegistry;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[pnj]]
            id = "ripperdoc-watson-01"
            archetype = "a"
            position = [0.0, 0.0, 0.0]
            briques = ["rester-statique"]
            "#,
        )
        .unwrap();
        let named_registry = NamedNpcRegistry::from_catalog(&catalog);
        let npc_runtime_id = named_registry.runtime_ids()[0];
        let mut server = Server::new_with_named_npcs(50.0, &catalog, named_registry);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        let first_snapshot = t.take_sent(1);
        // Vérifie au passage que le spawn au constructeur (pas juste l'arbitrage de session) a bien
        // eu lieu : le PNJ nominatif doit apparaître dans le tout premier snapshot du joueur, avant
        // même toute interaction — preuve qu'il vit réellement dans World depuis new_with_named_npcs.
        let npc_visible = first_snapshot.iter().any(|bytes| {
            let Ok(env) = flatbuffers::root::<ServerEnvelope>(bytes) else {
                return false;
            };
            let Some(snap) = env.msg_as_snapshot() else {
                return false;
            };
            snap.players()
                .map(|ps| ps.iter().any(|p| p.id() == npc_runtime_id))
                .unwrap_or(false)
        });
        assert!(
            npc_visible,
            "le PNJ nominatif doit être visible dans World dès la construction"
        );

        // Étape 1 : le joueur interagit -> le serveur ouvre une session.
        let interaction = encode_entity_interaction(npc_runtime_id, 2, 0);
        t.inject(TransportEvent::Message {
            from: 1,
            data: interaction,
        });
        server.tick(&mut t);
        let sent = t.take_sent(1);
        let session_id = sent
            .iter()
            .find_map(|bytes| {
                let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
                if env.msg_type() != ServerMsg::InteractionOpen {
                    return None;
                }
                env.msg_as_interaction_open().map(|o| o.session_id())
            })
            .expect("une InteractionOpen doit être envoyée pour un PNJ nominatif interactible");

        // Étape 2 : le joueur répond -> le serveur résout et répond InteractionResult{ok=true}.
        let choice = encode_interaction_choice(session_id, 0, 0);
        t.inject(TransportEvent::Message {
            from: 1,
            data: choice,
        });
        server.tick(&mut t);
        let sent = t.take_sent(1);
        let ok = sent.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            if env.msg_type() != ServerMsg::InteractionResult {
                return None;
            }
            env.msg_as_interaction_result().map(|r| r.ok())
        });
        assert_eq!(ok, Some(true));
    }

    #[test]
    fn tick_reclaims_a_session_opened_but_never_resolved_after_it_goes_stale() {
        // Trouvé en revue finale de branche : SessionRegistry::expire_stale (Task 1) était testé en
        // isolation mais jamais appelé depuis Server::tick — une session ouverte par un client qui
        // se déconnecte sans jamais répondre restait en mémoire indéfiniment. Ce test verrouille le
        // câblage réel : après le délai d'expiration, un Choice tardif sur une session ouverte AVANT
        // ce délai doit échouer comme si la session n'avait jamais existé (NotFound -> ok=false),
        // pas juste que SessionRegistry::expire_stale fonctionne isolément (déjà couvert ailleurs).
        use crate::named_npc_catalog::parse_and_validate;
        use crate::named_npc_registry::NamedNpcRegistry;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[pnj]]
            id = "ripperdoc-watson-01"
            archetype = "a"
            position = [0.0, 0.0, 0.0]
            briques = ["rester-statique"]
            "#,
        )
        .unwrap();
        let named_registry = NamedNpcRegistry::from_catalog(&catalog);
        let npc_runtime_id = named_registry.runtime_ids()[0];
        let mut server = Server::new_with_named_npcs(50.0, &catalog, named_registry);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        t.take_sent(1);

        let interaction = encode_entity_interaction(npc_runtime_id, 2, 0);
        t.inject(TransportEvent::Message {
            from: 1,
            data: interaction,
        });
        server.tick(&mut t);
        let sent = t.take_sent(1);
        let session_id = sent
            .iter()
            .find_map(|bytes| {
                let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
                if env.msg_type() != ServerMsg::InteractionOpen {
                    return None;
                }
                env.msg_as_interaction_open().map(|o| o.session_id())
            })
            .expect("une InteractionOpen doit être envoyée pour un PNJ nominatif interactible");

        // Laisse le délai d'expiration (30s, cf. INTERACTION_SESSION_TIMEOUT dans tick()) s'écouler
        // réellement — Instant ne se falsifie pas (même contrainte documentée dans
        // interaction_session.rs pour son propre test d'expiration réelle).
        std::thread::sleep(std::time::Duration::from_millis(30_100));
        // Un tick sans nouveau message doit tout de même réclamer la session expirée.
        server.tick(&mut t);
        t.take_sent(1);

        let choice = encode_interaction_choice(session_id, 0, 0);
        t.inject(TransportEvent::Message {
            from: 1,
            data: choice,
        });
        server.tick(&mut t);
        let sent = t.take_sent(1);
        let ok = sent.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            if env.msg_type() != ServerMsg::InteractionResult {
                return None;
            }
            env.msg_as_interaction_result().map(|r| r.ok())
        });
        assert_eq!(
            ok,
            Some(false),
            "une session expirée doit refuser le Choice tardif comme NotFound (ok=false)"
        );
    }

    #[test]
    fn a_npc_with_a_nav_graph_and_an_errer_brique_moves_over_several_ticks() {
        use crate::nav_graph::{NavGraph, Vec3 as NavVec3};
        use crate::npc_catalog::parse_and_validate;
        use crate::population_director::PopulationDirector;
        use std::collections::HashMap;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "marcheur"
            briques = ["errer"]
            "#,
        )
        .unwrap();
        let director = PopulationDirector::new(HashMap::from([("default".to_string(), 1)]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);

        let mut graph = NavGraph::new();
        let a = graph.add_node(NavVec3::new(0.0, 0.0, 0.0));
        // `b` à 10.0 (pas 200.0) : la brique errer tire des destinations dans un disque de
        // rayon 15.0 autour de la position courante (region_radius câblé dans tick_npcs) — un
        // second nœud à 200 unités n'est JAMAIS le nœud le plus proche d'une destination errer
        // (nearest_node(to) == nearest_node(from) à chaque fois, path=[position courante],
        // le PNJ "arrive" instantanément sans jamais bouger). À 10.0, une partie des destinations
        // tirées (x > 5, la médiatrice des deux nœuds) snappent sur `b`, produisant un vrai chemin
        // à traverser — vérifié en isolant le calcul (pseudo_random_unit + la formule errer)
        // avant de corriger ce fixture, cf. rapport de tâche.
        let b = graph.add_node(NavVec3::new(10.0, 0.0, 0.0));
        graph.add_edge(a, b);
        server.set_nav_graph(graph);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t); // laisse le director spawn le PNJ
        server.tick(&mut t); // laisse le premier tick assigner une destination + planifier

        let position_after_first = t.take_sent(1).iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?.iter().next().map(|n| {
                let p = n.position().unwrap();
                (p.x(), p.y())
            })
        });

        // Plusieurs ticks supplémentaires -> le PNJ doit avoir progressé (position différente).
        for _ in 0..20 {
            server.tick(&mut t);
        }
        let position_later = t.take_sent(1).iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?.iter().next().map(|n| {
                let p = n.position().unwrap();
                (p.x(), p.y())
            })
        });

        assert!(
            position_after_first.is_some() && position_later.is_some(),
            "le PNJ doit apparaître dans les deux snapshots"
        );
        assert_ne!(
            position_after_first, position_later,
            "après plusieurs ticks avec un graphe de navigation, la position du PNJ doit avoir changé"
        );
    }

    #[test]
    fn a_npc_without_a_nav_graph_never_moves_from_its_default_pose() {
        // Comportement historique préservé (fondation PNJ) : sans set_nav_graph, un PNJ avec une
        // brique errer ne bouge jamais — decide_destination peut produire une destination mais aucun
        // chemin n'est planifiable sans graphe.
        use crate::npc_catalog::parse_and_validate;
        use crate::population_director::PopulationDirector;
        use std::collections::HashMap;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "marcheur"
            briques = ["errer"]
            "#,
        )
        .unwrap();
        let director = PopulationDirector::new(HashMap::from([("default".to_string(), 1)]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);
        // PAS de set_nav_graph.

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        for _ in 0..5 {
            server.tick(&mut t);
        }
        let sent = t.take_sent(1);
        let npc_position = sent.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?.iter().next().map(|n| {
                let p = n.position().unwrap();
                (p.x(), p.y())
            })
        });
        assert_eq!(
            npc_position,
            Some((0.0, 0.0)),
            "sans graphe de navigation, le PNJ reste à sa Pose par défaut (0,0)"
        );
    }

    #[test]
    fn a_despawned_npc_has_its_nav_state_removed_not_leaked() {
        // Trouvé en revue finale de branche (plan navigation) : le despawn (population director,
        // fondation PNJ) retirait bien le NpcRecord et l'entrée World, mais laissait l'entrée
        // nav_states orpheline — fuite de mémoire lente (non bornée) sur un Shard longue durée à
        // fort churn spawn/despawn (les ids ne sont jamais réutilisés, next_npc_id ne fait
        // qu'augmenter). Ce test verrouille le nettoyage : après un cycle spawn -> déconnexion du
        // seul joueur présent (qui déclenche le despawn total du district "default",
        // population_director.rs) -> tick, le nombre d'entrées de nav_states doit être revenu à 0.
        use crate::nav_graph::{NavGraph, Vec3 as NavVec3};
        use crate::npc_catalog::parse_and_validate;
        use crate::population_director::PopulationDirector;
        use std::collections::HashMap;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "marcheur"
            briques = ["errer"]
            "#,
        )
        .unwrap();
        let director = PopulationDirector::new(HashMap::from([("default".to_string(), 1)]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);
        let mut graph = NavGraph::new();
        let a = graph.add_node(NavVec3::new(0.0, 0.0, 0.0));
        let b = graph.add_node(NavVec3::new(10.0, 0.0, 0.0));
        graph.add_edge(a, b);
        server.set_nav_graph(graph);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t); // spawn
        server.tick(&mut t); // planifie un chemin -> nav_states gagne une entrée
        assert_eq!(
            server
                .npc_registry
                .as_ref()
                .map(|r| r.nav_states.len())
                .unwrap_or(0),
            1,
            "précondition : le PNJ doit avoir un NavState avant le despawn"
        );

        t.inject(TransportEvent::Disconnected(1));
        server.tick(&mut t); // plus aucun joueur -> le director despawn tout le district "default"

        assert_eq!(
            server
                .npc_registry
                .as_ref()
                .map(|r| r.records.len())
                .unwrap_or(0),
            0,
            "le PNJ doit être despawné (comportement déjà existant, fondation PNJ)"
        );
        assert_eq!(
            server
                .npc_registry
                .as_ref()
                .map(|r| r.nav_states.len())
                .unwrap_or(0),
            0,
            "nav_states ne doit PAS garder une entrée orpheline après le despawn"
        );
    }

    #[test]
    fn a_fleeing_npc_moves_away_from_the_threat_over_several_ticks() {
        // Spec §9 critère « le FSM déclenche les bons déplacements (fuite s'éloigne) » — bout-en-
        // bout via Server/InMemoryTransport (pas juste decide_destination en isolation, déjà
        // couvert unitairement par Task 4).
        //
        // Piège de fixture identique en substance à celui trouvé en Task 5 (moves_over_several_ticks) :
        // le joueur 1 (la menace) ET le PNJ spawnent tous deux à la Pose par défaut (0,0,0)
        // (World::add_player). Si on laisse le PNJ à cette position par défaut, decide_destination
        // calcule dx=cx-mx=0, dy=0 -> `len = 0.0.max(EPSILON)` -> direction (0/EPSILON, 0/EPSILON) =
        // (0.0, 0.0) -> destination = position courante inchangée : la fuite ne produirait AUCUN
        // mouvement, indépendamment du câblage. Vérifié par calcul direct de la formule (rapport de
        // tâche). Fixe : on repositionne le PNJ (pas le joueur 1, qui reste à x=0 comme demandé par
        // le plan) à x=10 AVANT de déclencher la fuite, avec un graphe dont le premier nœud est
        // EXACTEMENT cette position (distance 0, pas de snap ambigu) et le second nœud EXACTEMENT à
        // la destination de fuite attendue (10 + FLEE_DISTANCE(30) = 40, distance 0 également) — donc
        // aucun aller-retour vers un nœud plus proche de la position de départ que la destination
        // réelle (le piège symétrique observé en Task 5 avec un nœud hors de portée aurait ici pris
        // la forme d'un chemin qui commence par reculer vers un nœud mal placé avant d'avancer ;
        // vérifié par simulation manuelle de NavState::advance sur les deux fixtures avant de choisir
        // celle-ci, cf. rapport de tâche).
        use crate::nav_graph::{NavGraph, Vec3 as NavVec3};
        use crate::npc_catalog::parse_and_validate;
        use crate::population_director::PopulationDirector;
        use std::collections::HashMap;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "marcheur"
            briques = ["errer"]
            "#,
        )
        .unwrap();
        let director = PopulationDirector::new(HashMap::from([("default".to_string(), 1)]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);

        let mut graph = NavGraph::new();
        let a = graph.add_node(NavVec3::new(10.0, 0.0, 0.0)); // position forcée de départ du PNJ
        let b = graph.add_node(NavVec3::new(40.0, 0.0, 0.0)); // = destination de fuite exacte
        graph.add_edge(a, b);
        server.set_nav_graph(graph);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t); // laisse le director spawn le PNJ (Pose par défaut (0,0,0))

        let sent = t.take_sent(1);
        let npc_id = sent.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?.iter().next().map(|n| n.id())
        });
        let npc_id = npc_id.expect("le PNJ doit être visible dès le premier snapshot");

        // Repositionne le PNJ à x=10 (le joueur 1, la menace, reste à x=0) — évite le cas dégénéré
        // menace==PNJ décrit ci-dessus. Accès direct à `self.world` : mêmes privilèges que le reste
        // de ce module de test (mod tests est un enfant de server_loop, `world` reste privé au
        // crate/module, cf. le reste de ce fichier).
        server.world.set_pose(
            npc_id,
            Pose {
                x: 10.0,
                y: 0.0,
                z: 0.0,
                ..Pose::default()
            },
        );

        // Le joueur 1 déclenche la fuite (kind=0=Fuite, fondation PNJ déjà câblée).
        let interaction = encode_entity_interaction(npc_id, 0, 0);
        t.inject(TransportEvent::Message {
            from: 1,
            data: interaction,
        });
        server.tick(&mut t); // applique l'interaction (Fuite) PUIS decide/planifie dans le même tick

        let position_after_first = t.take_sent(1).iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?
                .iter()
                .next()
                .map(|n| n.position().unwrap().x())
        });
        let position_after_first = position_after_first
            .expect("le PNJ doit apparaître dans le snapshot après la fuite déclenchée");

        for _ in 0..20 {
            server.tick(&mut t);
        }
        let position_later = t.take_sent(1).iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?
                .iter()
                .next()
                .map(|n| n.position().unwrap().x())
        });
        let position_later =
            position_later.expect("le PNJ doit rester visible après plusieurs ticks de fuite");

        assert!(
            position_later > position_after_first,
            "le PNJ en fuite doit s'éloigner du joueur 1 (menace à x=0) : x doit croître au fil des \
             ticks (après={position_after_first}, plus tard={position_later})"
        );
    }

    #[test]
    fn a_npc_keeps_advancing_along_its_path_even_with_no_player_connected_dead_reckoning() {
        // Spec §9 critère « adoption sans à-coup » : un PNJ persistant hors de portée avance quand
        // même (dead-reckoning) et doit être vu à sa position avancée — pas à sa position de spawn —
        // dès qu'un observateur rejoint. `tick_npcs` avance tous les PNJ inconditionnellement (pas
        // de garde AoI dans la boucle de mouvement, confirmé Task 5 Step 5) : ce test le prouve en
        // tickant sans observateur PROCHE du PNJ.
        //
        // Trouvé en revue finale de branche : le compte `player_count` de `tick_npcs` comptait à
        // tort les PNJ eux-mêmes (bug corrigé dans le même commit que ce test, cf. commentaire sur
        // `let player_count` plus haut dans ce fichier). Une fois corrigé, un district sans AUCUN
        // vrai joueur despawn réellement (population_director.rs, `!has_players`) — donc ce test ne
        // peut plus déconnecter le SEUL joueur du serveur sans faire despawn le PNJ avant même
        // d'atteindre les ticks de dead-reckoning. Un second joueur reste connecté, loin du PNJ
        // (hors de son rayon AoI de 50, donc jamais son observateur), pour garder le district
        // "default" non-vide pendant tout le scénario — le test prouve ainsi la vraie sémantique
        // LOD spec'd : « persistant hors de PORTÉE », pas « zéro joueur au monde ».
        //
        // Piège de fixture : le brief d'origine de cette tâche proposait un graphe à 2 nœuds, (0,0,0)
        // et (200,0,0) — EXACTEMENT le même piège que Task 5 a trouvé et documenté dans
        // `a_npc_with_a_nav_graph_and_an_errer_brique_moves_over_several_ticks` : ce PNJ utilise la
        // brique `errer` (comportement Calme), dont `decide_destination` tire une destination dans
        // un disque de rayon FIXE 15.0 autour de la position courante (`region_radius` câblé en dur
        // dans `tick_npcs`). Le rayon maximum réellement atteignable par la formule
        // `region_radius * sqrt((rng_unit * 7.0) % 1.0)` est ~14.849 (vérifié par calcul direct sur
        // la formule + reproduit empiriquement : le fixture à 200 unités laisse le PNJ figé en
        // (0,0,0) après 30 ticks de dead-reckoning, cf. rapport de tâche) — donc un nœud à 200 unités
        // n'est JAMAIS le nœud le plus proche d'une destination errer, `nearest_node` retombe toujours
        // sur le nœud de départ, et le "chemin" planifié est un unique waypoint à la position déjà
        // occupée par le PNJ : aucun mouvement, quel que soit le nombre de ticks. Repris ici avec le
        // second nœud à 10.0 (dans le rayon de 15.0), comme le fix de Task 5.
        use crate::nav_graph::{NavGraph, Vec3 as NavVec3};
        use crate::npc_catalog::parse_and_validate;
        use crate::population_director::PopulationDirector;
        use std::collections::HashMap;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "marcheur"
            briques = ["errer"]
            "#,
        )
        .unwrap();
        // Densité cible 1 mais AUCUN joueur présent -> le director ne spawn normalement rien (LOD,
        // fondation PNJ) ; ce test a donc besoin d'un PNJ déjà existant. Le joueur 1 déclenche le
        // spawn puis se déconnecte ; le joueur 2, loin du PNJ, reste connecté pendant tout le
        // scénario pour garder le district non-vide (cf. commentaire ci-dessus) sans jamais observer
        // le PNJ (hors du rayon AoI de 50).
        let director = PopulationDirector::new(HashMap::from([("default".to_string(), 1)]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);
        let mut graph = NavGraph::new();
        let a = graph.add_node(NavVec3::new(0.0, 0.0, 0.0));
        // `b` à 10.0 (pas 200.0, cf. commentaire ci-dessus) : dans le rayon de tir errer (15.0).
        let b = graph.add_node(NavVec3::new(10.0, 0.0, 0.0));
        graph.add_edge(a, b);
        server.set_nav_graph(graph);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t); // spawn
        server.tick(&mut t); // 1re destination/chemin

        // Joueur 2, loin (bien hors du rayon AoI de 50 depuis l'origine où erre le PNJ) — garde le
        // district non-vide sans jamais devenir l'observateur du PNJ. Sa propre position n'est
        // jamais mise à jour ensuite (aucun PositionUpdate envoyé), donc il reste loin tout le test.
        t.inject(TransportEvent::Connected(2));
        server.tick(&mut t);
        let far_position = encode_position(500.0, 500.0, 0.0, 0.0);
        t.inject(TransportEvent::Message {
            from: 2,
            data: far_position,
        });
        server.tick(&mut t);

        t.inject(TransportEvent::Disconnected(1));
        server.tick(&mut t);

        // 30 ticks de dead-reckoning sans observateur PROCHE (le joueur 2 reste connecté, mais loin).
        for _ in 0..30 {
            server.tick(&mut t);
        }
        t.take_sent(2); // purge les snapshots accumulés du joueur 2 (jamais dans l'AoI du PNJ)

        // Un troisième joueur rejoint près du PNJ (le wander le maintient dans le disque de rayon 15
        // autour de l'origine, donc toujours dans le rayon AoI de 50) et doit le voir immédiatement à
        // sa position avancée par dead-reckoning, sans à-coup.
        t.inject(TransportEvent::Connected(3));
        server.tick(&mut t);
        let sent = t.take_sent(3);
        let npc_position = sent.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?.iter().next().map(|n| {
                let p = n.position().unwrap();
                (p.x(), p.y())
            })
        });
        let npc_position =
            npc_position.expect("le PNJ doit être immédiatement visible au joueur qui rejoint");
        assert_ne!(
            npc_position,
            (0.0, 0.0),
            "après 30+ ticks de dead-reckoning sans observateur, le PNJ ne doit plus être à sa \
             position de spawn — preuve que le mouvement a continué sans joueur connecté \
             (position observée = {npc_position:?})"
        );
    }
}
